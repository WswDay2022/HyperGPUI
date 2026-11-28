use std::{
    ffi::c_void,
    slice,
    time::{Duration, Instant},
};

use anyhow::{Result, ensure};
use hgpui::{Background, ScaledPixels, bounds, point, rgb, size};
use windows::{
    Win32::{
        Foundation::S_OK,
        Graphics::{
            Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            Direct3D11::{
                D3D11_CPU_ACCESS_READ, D3D11_QUERY, D3D11_QUERY_DATA_TIMESTAMP_DISJOINT,
                D3D11_QUERY_DESC, D3D11_QUERY_TIMESTAMP, D3D11_QUERY_TIMESTAMP_DISJOINT,
                D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, D3D11_VIEWPORT, ID3D11Device,
                ID3D11DeviceContext, ID3D11Query, ID3D11RenderTargetView, ID3D11Texture2D,
            },
            Dxgi::IDXGIDevice,
        },
    },
    core::Interface,
};

use super::super::{
    GlobalUniforms, PathRasterizationSprite, PipelineState, RENDER_TARGET_FORMAT,
    create_blend_state_for_path_rasterization, create_path_intermediate_msaa_texture_and_view,
    create_path_intermediate_texture, create_rasterizer_state, shader_resources::ShaderModule,
    update_buffer,
};
use super::{create_hardware_device, read_pixel, test_globals};

struct GpuTimer {
    clock: ID3D11Query,
    start: ID3D11Query,
    end: ID3D11Query,
}

impl GpuTimer {
    fn new(device: &ID3D11Device) -> Result<Self> {
        let query = |kind: D3D11_QUERY| -> Result<_> {
            let mut out = None;
            unsafe {
                device.CreateQuery(
                    &D3D11_QUERY_DESC {
                        Query: kind,
                        MiscFlags: 0,
                    },
                    Some(&mut out),
                )?
            };
            Ok(out.unwrap())
        };
        Ok(Self {
            clock: query(D3D11_QUERY_TIMESTAMP_DISJOINT)?,
            start: query(D3D11_QUERY_TIMESTAMP)?,
            end: query(D3D11_QUERY_TIMESTAMP)?,
        })
    }

    fn measure_ms(
        &self,
        context: &ID3D11DeviceContext,
        batches: u32,
        mut run: impl FnMut() -> Result<()>,
    ) -> Result<f64> {
        unsafe {
            context.Begin(&self.clock);
            context.End(&self.start);
        }
        for _ in 0..batches {
            run()?;
        }
        unsafe {
            context.End(&self.end);
            context.End(&self.clock);
            context.Flush();
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        let read = |query: &ID3D11Query, data: *mut c_void, size| -> Result<()> {
            loop {
                // The high-level Result merges S_FALSE (not ready) with S_OK.
                let status = unsafe {
                    (context.vtable().GetData)(context.as_raw(), query.as_raw(), data, size, 0)
                };
                status.ok()?;
                if status == S_OK {
                    return Ok(());
                }
                ensure!(Instant::now() < deadline, "GPU timestamp query timed out");
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        let mut interval = D3D11_QUERY_DATA_TIMESTAMP_DISJOINT::default();
        read(
            &self.clock,
            (&mut interval as *mut D3D11_QUERY_DATA_TIMESTAMP_DISJOINT).cast(),
            std::mem::size_of_val(&interval) as u32,
        )?;
        ensure!(
            !interval.Disjoint.as_bool() && interval.Frequency != 0,
            "disjoint GPU interval"
        );
        let mut start = 0u64;
        let mut end = 0u64;
        read(&self.start, (&mut start as *mut u64).cast(), 8)?;
        read(&self.end, (&mut end as *mut u64).cast(), 8)?;
        ensure!(
            end > start,
            "inverted or empty GPU interval: {start}..{end}"
        );
        Ok((end - start) as f64 * 1000.0 / (interval.Frequency as f64 * f64::from(batches)))
    }
}

struct PathTarget {
    dimensions: [u32; 2],
    resolved: ID3D11Texture2D,
    msaa: ID3D11Texture2D,
    rtv: Option<ID3D11RenderTargetView>,
}

#[test]
#[ignore = "DX11 hardware timestamp benchmark; run by itself"]
fn path_msaa_target_size_hardware_timing() -> Result<()> {
    const BATCHES: u32 = 24;
    const ROUNDS: usize = 7;
    const DIMENSIONS: [[u32; 2]; 3] = [[3840, 2160], [1920, 1080], [128, 128]];
    const MODES: [&str; 4] = ["draw", "clear+draw", "resolve", "clear+draw+resolve"];

    let (device, context) =
        create_hardware_device().expect("DX11 hardware required; no WARP fallback");
    let adapter = unsafe { device.cast::<IDXGIDevice>()?.GetAdapter()?.GetDesc()? };
    eprintln!(
        "MSAA adapter: {}",
        String::from_utf16_lossy(&adapter.Description).trim_end_matches('\0')
    );

    let timer = GpuTimer::new(&device)?;
    let rasterizer = create_rasterizer_state(&device, false)?;
    unsafe { context.RSSetState(&rasterizer) };
    let globals = test_globals(&device, &context)?;
    update_buffer(
        &context,
        globals[0].as_ref().unwrap(),
        &[GlobalUniforms {
            viewport_size: [3840.0, 2160.0],
            premultiplied_alpha: 0,
            padding: 0,
        }],
    )?;
    let viewport = D3D11_VIEWPORT {
        Width: 3840.0,
        Height: 2160.0,
        MaxDepth: 1.0,
        ..Default::default()
    };
    let path_bounds = bounds(
        point(ScaledPixels(16.25), ScaledPixels(16.25)),
        size(ScaledPixels(32.0), ScaledPixels(32.0)),
    );
    let vertices =
        [(16.25, 16.25), (48.25, 16.25), (16.25, 48.25)].map(|(x, y)| PathRasterizationSprite {
            xy_position: point(ScaledPixels(x), ScaledPixels(y)),
            curve_position: point(0.0, 0.0),
            color: Background::from(rgb(0xff0000)),
            bounds: path_bounds,
            content_mask: path_bounds,
            mask_chain: 0,
            tile_info: [0; 3],
        });
    let mut raster = PipelineState::new(
        &device,
        "path_msaa_bench",
        ShaderModule::PathRasterization,
        32,
        create_blend_state_for_path_rasterization(&device)?,
    )?;
    raster.update_buffer(&device, &context, &vertices)?;

    let mut targets = Vec::new();
    for dimensions @ [width, height] in DIMENSIONS {
        let (resolved, _) = create_path_intermediate_texture(&device, width, height)?;
        let (msaa, rtv) = create_path_intermediate_msaa_texture_and_view(&device, width, height)?;
        targets.push(PathTarget {
            dimensions,
            resolved,
            msaa,
            rtv,
        });
    }

    let run = |target: &PathTarget, mode: usize| -> Result<()> {
        if matches!(mode, 1 | 3) {
            unsafe { context.ClearRenderTargetView(target.rtv.as_ref().unwrap(), &[0.0; 4]) };
        }
        if mode != 2 {
            unsafe { context.OMSetRenderTargets(Some(slice::from_ref(&target.rtv)), None) };
            raster.draw(
                &context,
                &[viewport],
                &globals,
                D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
                3,
                1,
            )?;
        }
        if matches!(mode, 2 | 3) {
            unsafe {
                context.ResolveSubresource(
                    &target.resolved,
                    0,
                    &target.msaa,
                    0,
                    RENDER_TARGET_FORMAT,
                )
            };
        }
        unsafe { context.OMSetRenderTargets(None, None) };
        Ok(())
    };

    // Allocate, clear and resolve once before timing. Verify that the same tiny path really
    // rasterizes in every size, with empty background; staging transfers are outside timing.
    for target in &targets {
        run(target, 3)?;
        let mut staging_desc = D3D11_TEXTURE2D_DESC::default();
        let mut staging = None;
        unsafe {
            target.resolved.GetDesc(&mut staging_desc);
            staging_desc.Usage = D3D11_USAGE_STAGING;
            staging_desc.BindFlags = 0;
            staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            device.CreateTexture2D(&staging_desc, None, Some(&mut staging))?;
        }
        let staging = staging.unwrap();
        assert_eq!(
            read_pixel(&context, &target.resolved, &staging, 24, 24)?,
            [0, 0, 255, 255]
        );
        assert_eq!(
            read_pixel(&context, &target.resolved, &staging, 80, 80)?,
            [0; 4]
        );
    }

    let mut samples: [[Vec<f64>; 4]; 3] =
        std::array::from_fn(|_| std::array::from_fn(|_| Vec::new()));
    for round in 0..ROUNDS {
        for t in 0..DIMENSIONS.len() {
            let i = (t + round) % DIMENSIONS.len();
            for m in 0..MODES.len() {
                let mode = (m + round) % MODES.len();
                // Every timing starts with a seeded source, so resolve-only never measures
                // an untouched clear texture and draw-only starts from defined pixels.
                run(&targets[i], 1)?;
                let ms = timer.measure_ms(&context, BATCHES, || run(&targets[i], mode))?;
                samples[i][mode].push(ms);
            }
        }
    }
    eprintln!(
        "MSAA sample: 32x32 triangle bounds, 4x BGRA8, {BATCHES} batches/sample, viewport fixed at 3840x2160"
    );
    for (target, timings) in targets.iter().zip(samples.iter_mut()) {
        let [w, h] = target.dimensions;
        for (mode, values) in MODES.into_iter().zip(timings) {
            values.sort_by(f64::total_cmp);
            eprintln!(
                "MSAA {w}x{h} {mode:18} median={:.5} ms/batch, samples={values:?}",
                values[ROUNDS / 2]
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "DX11 hardware timestamp benchmark; run by itself"]
fn compact_path_batch_hardware_timing() -> Result<()> {
    use hgpui::{Path, px};
    use hgpui_render::PathTarget as Plan;

    let (device, context) =
        create_hardware_device().expect("DX11 hardware required; no WARP fallback");
    let adapter = unsafe { device.cast::<IDXGIDevice>()?.GetAdapter()?.GetDesc()? };
    let globals = test_globals(&device, &context)?;
    let scene = [3840, 2160];
    update_buffer(
        &context,
        globals[0].as_ref().unwrap(),
        &[GlobalUniforms {
            viewport_size: scene.map(|v| v as f32),
            premultiplied_alpha: 0,
            padding: 0,
        }],
    )?;
    let rasterizer = create_rasterizer_state(&device, false)?;
    unsafe { context.RSSetState(&rasterizer) };
    let start = point(px(1702.25), px(910.375));
    let mut path = Path::new(start);
    path.line_to(point(start.x + px(32.0), start.y));
    path.line_to(point(start.x, start.y + px(32.0)));
    let mut path = path.scale(1.0);
    path.color = Background::from(rgb(0xff0000));
    path.content_mask.bounds = path.bounds;
    let plans = [
        Plan::full(scene),
        Plan::for_paths(scene, slice::from_ref(&path)),
    ];
    assert_eq!(plans[1].size(), [128; 2]);
    assert_ne!(plans[1].sprite(path.bounds).tile_info[..2], [0; 2]);

    let timer = GpuTimer::new(&device)?;
    let mut cases = Vec::new();
    for plan in plans {
        let [width, height] = plan.size();
        let (resolved, _) = create_path_intermediate_texture(&device, width, height)?;
        let (msaa, rtv) = create_path_intermediate_msaa_texture_and_view(&device, width, height)?;
        let viewport = D3D11_VIEWPORT {
            Width: width as f32,
            Height: height as f32,
            MaxDepth: 1.0,
            ..Default::default()
        };
        let mut raster = PipelineState::new(
            &device,
            "projected_path_bench",
            ShaderModule::PathRasterization,
            3,
            create_blend_state_for_path_rasterization(&device)?,
        )?;
        let vertices = path
            .vertices
            .iter()
            .map(|vertex| plan.vertex(&path, vertex))
            .collect::<Vec<_>>();
        assert_eq!(vertices.len(), 3);
        raster.update_buffer(&device, &context, &vertices)?;
        cases.push((resolved, msaa, rtv, viewport, raster));
    }
    let run = |index: usize| -> Result<()> {
        let (resolved, msaa, rtv, viewport, raster) = &cases[index];
        unsafe {
            context.ClearRenderTargetView(rtv.as_ref().unwrap(), &[0.0; 4]);
            context.OMSetRenderTargets(Some(slice::from_ref(rtv)), None);
        }
        raster.draw(
            &context,
            slice::from_ref(viewport),
            &globals,
            D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            3,
            1,
        )?;
        unsafe {
            context.ResolveSubresource(resolved, 0, msaa, 0, RENDER_TARGET_FORMAT);
            context.OMSetRenderTargets(None, None);
        }
        Ok(())
    };
    for i in 0..2 {
        timer.measure_ms(&context, 24, || run(i))?;
    }
    let mut samples = [Vec::new(), Vec::new()];
    for round in 0..7 {
        for i in [round % 2, 1 - round % 2] {
            samples[i].push(timer.measure_ms(&context, 24, || run(i))?);
        }
    }
    for s in &mut samples {
        s.sort_by(f64::total_cmp);
    }
    let device_name = String::from_utf16_lossy(&adapter.Description);
    eprintln!(
        "{}: production path mapping at nonzero origin; GPU ms/batch median7 full={:.5} compact={:.5}, samples={samples:?}",
        device_name.trim_end_matches('\0'),
        samples[0][3],
        samples[1][3]
    );
    Ok(())
}

#[test]
fn compact_path_targets_match_full_rendering() -> Result<()> {
    use super::super::{
        PathSprite, create_blend_state_for_path_sprite, create_blur_sampler, create_buffer,
        create_buffer_view, create_color_target,
    };
    use super::create_warp_device;
    use hgpui::{
        Bounds, ContentMask, Corners, Edges, Path, hsla, linear_color_stop, linear_gradient, px,
    };
    use hgpui_render::{PATH_TARGET_SLOTS, PathTarget as Plan};
    use windows::Win32::Graphics::Direct3D11::{D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE};

    let (device, context) = create_warp_device().expect("WARP required for path tile readback");
    let scene_size = [965, 677];
    let viewport = |[w, h]: [u32; 2]| D3D11_VIEWPORT {
        Width: w as f32,
        Height: h as f32,
        MaxDepth: 1.0,
        ..Default::default()
    };
    let globals = test_globals(&device, &context)?;
    update_buffer(
        &context,
        globals[0].as_ref().unwrap(),
        &[GlobalUniforms {
            viewport_size: scene_size.map(|n| n as f32),
            premultiplied_alpha: 0,
            padding: 0,
        }],
    )?;
    let rect = |x: f32, y: f32, w: f32, h: f32| {
        bounds(
            point(ScaledPixels(x), ScaledPixels(y)),
            size(ScaledPixels(w), ScaledPixels(h)),
        )
    };
    let mask = ContentMask {
        bounds: rect(301.7, 198.3, 86.5, 82.5),
        corner_radii: Corners {
            top_left: ScaledPixels(12.0),
            bottom_right: ScaledPixels(15.0),
            ..Default::default()
        },
        fade_out: Edges {
            bottom: ScaledPixels(12.0),
            ..Default::default()
        },
    };
    let mask_buffer = create_buffer(&device, std::mem::size_of::<ContentMask<ScaledPixels>>(), 1)?;
    update_buffer(&context, &mask_buffer, &[mask])?;
    let mask_view = create_buffer_view(&device, &mask_buffer)?;
    let rasterizer = create_rasterizer_state(&device, false)?;
    unsafe {
        context.VSSetShaderResources(5, Some(slice::from_ref(&mask_view)));
        context.PSSetShaderResources(5, Some(slice::from_ref(&mask_view)));
        context.RSSetState(&rasterizer);
    }
    let sampler = create_blur_sampler(&device)?;
    let mut raster = PipelineState::new(
        &device,
        "path_tile_reference",
        ShaderModule::PathRasterization,
        6,
        create_blend_state_for_path_rasterization(&device)?,
    )?;
    let mut sprite = PipelineState::<PathSprite>::new(
        &device,
        "path_tile_composite",
        ShaderModule::PathSprite,
        1,
        create_blend_state_for_path_sprite(&device)?,
    )?;
    let make_path = |x, y, w, h, masked, order| {
        let mut path = Path::new(point(px(x), px(y)));
        path.line_to(point(px(x + w), px(y)));
        path.curve_to(
            point(px(x), px(y + h)),
            point(px(x + w * 0.8), px(y + h * 0.8)),
        );
        path.line_to(point(px(x), px(y)));
        let mut path = path.scale(1.0);
        path.content_mask = if masked {
            mask
        } else {
            ContentMask {
                bounds: rect(0.0, 0.0, scene_size[0] as f32, scene_size[1] as f32),
                ..Default::default()
            }
        };
        path.mask_chain = if masked { 1 << 24 } else { 0 };
        path.order = order;
        path.color = linear_gradient(
            37.0,
            linear_color_stop(hsla(0.0, 1.0, 0.5, 0.85), 0.0),
            linear_color_stop(hsla(0.6, 1.0, 0.5, 0.55), 1.0),
        );
        path
    };
    let batches = [
        vec![make_path(298.2, 192.4, 82.7, 77.6, true, 1)],
        vec![make_path(329.3, 208.2, 25.9, 24.3, true, 2)],
        vec![
            make_path(20.3, 311.4, 25.0, 31.0, false, 3),
            make_path(739.2, 533.6, 45.7, 50.2, false, 4),
        ],
        vec![make_path(934.6, 642.2, 49.7, 51.1, false, 5)],
        vec![make_path(97.5, 126.2, 190.4, 144.1, false, 6)],
    ];
    assert_eq!(
        batches
            .each_ref()
            .map(|paths| Plan::for_paths(scene_size, paths).slot()),
        [1, 1, 0, 1, 2]
    );

    let mut render = |compact: bool| -> Result<Vec<u8>> {
        let (output, rtv, _) = create_color_target(&device, scene_size[0], scene_size[1])?;
        let mut cached: [Option<super::super::DxPathTargets>; PATH_TARGET_SLOTS] =
            Default::default();
        unsafe { context.ClearRenderTargetView(rtv.as_ref().unwrap(), &[0.0; 4]) };
        for paths in &batches {
            let plan = if compact {
                Plan::for_paths(scene_size, paths)
            } else {
                Plan::full(scene_size)
            };
            if cached[plan.slot()].is_none() {
                let [width, height] = plan.size();
                let (resolved, srv) = create_path_intermediate_texture(&device, width, height)?;
                let (msaa, rtv) =
                    create_path_intermediate_msaa_texture_and_view(&device, width, height)?;
                cached[plan.slot()] = Some(super::super::DxPathTargets {
                    resolved,
                    srv,
                    msaa,
                    rtv,
                });
            }
            let targets = cached[plan.slot()].as_ref().unwrap();
            let mut vertices = Vec::new();
            for path in paths {
                vertices.extend(path.vertices.iter().map(|vertex| plan.vertex(path, vertex)));
            }
            raster.update_buffer(&device, &context, &vertices)?;
            unsafe {
                context.ClearRenderTargetView(targets.rtv.as_ref().unwrap(), &[0.0; 4]);
                context.OMSetRenderTargets(Some(slice::from_ref(&targets.rtv)), None);
            }
            raster.draw(
                &context,
                &[viewport(plan.size())],
                &globals,
                D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
                vertices.len() as u32,
                1,
            )?;
            unsafe {
                context.ResolveSubresource(
                    &targets.resolved,
                    0,
                    &targets.msaa,
                    0,
                    RENDER_TARGET_FORMAT,
                );
                context.OMSetRenderTargets(Some(slice::from_ref(&rtv)), None);
            }
            let union = paths
                .iter()
                .map(Path::clipped_bounds)
                .reduce(|a: Bounds<ScaledPixels>, b| a.union(&b))
                .unwrap();
            sprite.update_buffer(&device, &context, &[plan.sprite(union)])?;
            sprite.draw_with_texture(
                &context,
                slice::from_ref(&targets.srv),
                &[viewport(scene_size)],
                &globals,
                slice::from_ref(&sampler),
                1,
            )?;
        }

        let mut desc = D3D11_TEXTURE2D_DESC::default();
        let mut staging = None;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            context.OMSetRenderTargets(None, None);
            output.GetDesc(&mut desc);
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            device.CreateTexture2D(&desc, None, Some(&mut staging))?;
            let staging = staging.unwrap();
            context.CopyResource(&staging, &output);
            context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let pitch = mapped.RowPitch as usize;
            let result =
                slice::from_raw_parts(mapped.pData.cast::<u8>(), pitch * scene_size[1] as usize)
                    .chunks_exact(pitch)
                    .flat_map(|row| row[..scene_size[0] as usize * 4].iter().copied())
                    .collect();
            context.Unmap(&staging, 0);
            Ok(result)
        }
    };
    let full = render(false)?;
    let compact = render(true)?;
    assert!(full.chunks_exact(4).filter(|pixel| pixel[3] != 0).count() > 1000);
    let max_diff = full
        .iter()
        .zip(&compact)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    eprintln!("full vs tiled BGRA8 maximum channel delta: {max_diff}");
    for (index, (a, b)) in full
        .chunks_exact(4)
        .zip(compact.chunks_exact(4))
        .enumerate()
        .filter(|(_, (a, b))| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 1))
        .take(10)
    {
        eprintln!(
            "  ({}, {}) full={a:?} compact={b:?}",
            index % scene_size[0] as usize,
            index / scene_size[0] as usize
        );
    }
    assert!(
        max_diff <= 1,
        "compact path coverage or paint changed (max delta {max_diff})"
    );
    Ok(())
}
