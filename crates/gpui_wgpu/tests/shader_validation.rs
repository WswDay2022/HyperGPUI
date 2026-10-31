//! Validates the WGSL shader modules through the real wgpu device — the same validation path
//! production uses when it calls `create_shader_module` — and renders one quad through the real
//! `vs_quad`/`fs_quad` entry points to pin the pixel-level behaviour the Windows HLSL blueprint
//! defines (fractional-edge AA, mask straight-edge AA, transparent-border `over`).
//!
//! The combined dual-source module used to reference `to_device_position_transformed` and
//! `distance_from_clip_rect_transformed`, neither of which existed anywhere; naga rejected the
//! whole module and any adapter with `DUAL_SOURCE_BLENDING` would have failed to create the
//! subpixel pipeline.
//!
//! No adapter available (headless CI, no GPU) skips with a message, like the WARP tests on
//! Windows.
//!
//! This lives in `tests/` (its own process) rather than in the unit-test binary: creating a
//! wgpu device alongside the font/cosmic-text unit tests crashes this machine's driver with
//! STATUS_HEAP_CORRUPTION when the harness runs them in parallel.

use std::borrow::Cow;

use hgpui::block_on;
use hgpui::{
    bounds, point, rgba, size, Background, ContentMask, Corners, Edges, Quad, ScaledPixels,
    transparent_black,
};
use hgpui_wgpu::wgpu;

const SIZE: u32 = 64;
const ROW_BYTES: u32 = SIZE * 4; // 256, already wgpu's required row alignment.

/// Mirrors the WGSL `GlobalParams` (16 bytes).
#[repr(C)]
struct Globals {
    viewport_size: [f32; 2],
    premultiplied_alpha: u32,
    pad: u32,
}

/// Mirrors the WGSL `GammaParams` (32 bytes).
#[repr(C)]
struct GammaParams {
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
    subpixel_enhanced_contrast: f32,
    is_bgr: u32,
    pad: u32,
}

fn as_bytes<T>(value: &T) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts((value as *const T).cast(), std::mem::size_of::<T>())
    }
}

/// Packed `(len << 24) | offset` reference into the frame's content-mask chain arena, matching
/// `Scene::intern_mask_chain` and the shaders' `CONTENT_MASKS` indexing.
fn mask_chain_ref(offset: u32, len: u32) -> u32 {
    (len << 24) | offset
}

fn mask_bytes(masks: &[ContentMask<ScaledPixels>]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            masks.as_ptr().cast(),
            std::mem::size_of_val(masks),
        )
    }
}

fn validate(device: &wgpu::Device, label: &str, source: &str) {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let _module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(source)),
    });
    let error = block_on(scope.pop());
    assert!(
        error.is_none(),
        "{label} failed GPU shader validation: {error:?}"
    );
}

fn create_device() -> Option<(wgpu::Device, wgpu::Queue, bool)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = match block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        force_fallback_adapter: false,
    })) {
        Ok(adapter) => adapter,
        Err(err) => {
            eprintln!("[wgsl-validate] no GPU adapter, skipping: {err}");
            return None;
        }
    };

    let has_dual_source = adapter
        .features()
        .contains(wgpu::Features::DUAL_SOURCE_BLENDING);
    let mut features = wgpu::Features::empty();
    if has_dual_source {
        features |= wgpu::Features::DUAL_SOURCE_BLENDING;
    }

    let (device, queue) = match block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("hgpui_shader_validation"),
        required_features: features,
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    })) {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("[wgsl-validate] no wgpu device, skipping: {err}");
            return None;
        }
    };
    Some((device, queue, has_dual_source))
}

fn viewport_mask() -> ContentMask<ScaledPixels> {
    ContentMask {
        bounds: bounds(
            point(ScaledPixels(0.0), ScaledPixels(0.0)),
            size(ScaledPixels(SIZE as f32), ScaledPixels(SIZE as f32)),
        ),
        corner_radii: Corners::default(),
            fade_out: Default::default(),
    }
}

/// Renders one quad and returns the RGBA8 readback (row-major, `ROW_BYTES` per row).
#[allow(clippy::too_many_arguments)]
fn render_quad(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &wgpu::RenderPipeline,
    globals_group: &wgpu::BindGroup,
    quad_group: &wgpu::BindGroup,
    quad_buffer: &wgpu::Buffer,
    readback: &wgpu::Buffer,
    texture: &wgpu::Texture,
    view: &wgpu::TextureView,
    quad: &Quad,
    clear: wgpu::Color,
) -> Vec<u8> {
    queue.write_buffer(quad_buffer, 0, as_bytes(quad));

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("quad_test_encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("quad_test_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                // Black clear: partial red coverage reads directly as the red channel.
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, globals_group, &[]);
        pass.set_bind_group(1, quad_group, &[]);
        pass.draw(0..4, 0..1);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ROW_BYTES),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        tx.send(result).expect("readback channel closed");
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("device poll failed");
    rx.recv().expect("readback callback never ran").expect("readback failed");
    let data = readback.slice(..).get_mapped_range().to_vec();
    readback.unmap();
    data
}

fn pixel(data: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = (y * ROW_BYTES + x * 4) as usize;
    [data[i], data[i + 1], data[i + 2], data[i + 3]]
}

fn assert_red_close(px: [u8; 4], expected: u8, what: &str) {
    let delta = i16::from(px[0]) - i16::from(expected);
    assert!(
        (-3..=3).contains(&delta),
        "{what}: expected red ~{expected}, got {px:?} (RGBA)"
    );
}

#[test]
fn wgsl_shader_modules_validate_and_render_quad() {
    let base = hgpui_render::artifacts::BASE_WGSL;
    let subpixel = hgpui_render::artifacts::SUBPIXEL_WGSL;

    let Some((device, queue, has_dual_source)) = create_device() else {
        return;
    };

    // 1. Both modules must pass the driver's validation. The subpixel module is compiled
    // separately by `WgpuPipelines::new`, so validate it on its own.
    validate(&device, "hgpui_shaders", base);
    if has_dual_source {
        validate(&device, "hgpui_subpixel_shaders", subpixel);
    } else {
        eprintln!(
            "[wgsl-validate] DUAL_SOURCE_BLENDING unavailable; combined subpixel module skipped"
        );
    }

    // 2. Render the quad pipeline and pin the same pixel values as the Windows WARP tests.
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("quad_test_shader"),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(base)),
    });
    let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("quad_test_globals_layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(16),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(32),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let quad_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("quad_test_quad_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Quad>() as u64),
            },
            count: None,
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("quad_test_pipeline_layout"),
        bind_group_layouts: &[Some(&globals_layout), Some(&quad_layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("quad_test_pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("quad_vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("quad_fragment"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let uniform_usage = wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST;
    let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("quad_test_globals"),
        size: std::mem::size_of::<Globals>() as u64,
        usage: uniform_usage,
        mapped_at_creation: false,
    });
    queue.write_buffer(
        &globals_buffer,
        0,
        as_bytes(&Globals {
            viewport_size: [SIZE as f32, SIZE as f32],
            premultiplied_alpha: 0,
            pad: 0,
        }),
    );
    let gamma_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("quad_test_gamma"),
        size: std::mem::size_of::<GammaParams>() as u64,
        usage: uniform_usage,
        mapped_at_creation: false,
    });
    queue.write_buffer(
        &gamma_buffer,
        0,
        as_bytes(&GammaParams {
            gamma_ratios: [1.0, 1.0, 1.0, 1.0],
            grayscale_enhanced_contrast: 0.0,
            subpixel_enhanced_contrast: 0.0,
            is_bgr: 0,
            pad: 0,
        }),
    );
    let quad_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("quad_test_quads"),
        size: std::mem::size_of::<Quad>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    // Chain arena: [0] = the fractional quad's own bounds, [1] = the partially covered mask edge.
    let mask_arena = [
        ContentMask {
            bounds: bounds(
                point(ScaledPixels(16.25), ScaledPixels(16.25)),
                size(ScaledPixels(32.5), ScaledPixels(32.5)),
            ),
            corner_radii: Corners::default(),
            fade_out: Default::default(),
        },
        ContentMask {
            bounds: bounds(
                point(ScaledPixels(8.0), ScaledPixels(8.0)),
                size(ScaledPixels(32.75), ScaledPixels(48.0)),
            ),
            corner_radii: Corners::default(),
            fade_out: Default::default(),
        },
    ];
    let mask_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("quad_test_masks"),
        size: std::mem::size_of_val(&mask_arena) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&mask_buffer, 0, mask_bytes(&mask_arena));
    let globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("quad_test_globals_group"),
        layout: &globals_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: gamma_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: mask_buffer.as_entire_binding(),
            },
        ],
    });
    let quad_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("quad_test_quad_group"),
        layout: &quad_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: quad_buffer.as_entire_binding(),
        }],
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("quad_test_target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("quad_test_readback"),
        size: u64::from(ROW_BYTES * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    // Case 1: [16.25, 48.75] on both axes — each edge covers 0.75 of its boundary pixel.
    let fractional = Quad {
        bounds: bounds(
            point(ScaledPixels(16.25), ScaledPixels(16.25)),
            size(ScaledPixels(32.5), ScaledPixels(32.5)),
        ),
        // Upstream antialiases a plain quad through its content mask; use the
        // quad's own bounds so the fractional edge gets a one-pixel ramp.
        content_mask: ContentMask {
            bounds: bounds(
                point(ScaledPixels(16.25), ScaledPixels(16.25)),
                size(ScaledPixels(32.5), ScaledPixels(32.5)),
            ),
            corner_radii: Corners::default(),
            fade_out: Default::default(),
        },
        background: Background::from(rgba(0xff0000ff)),
        mask_chain: mask_chain_ref(0, 1),
        ..Default::default()
    };
    let data = render_quad(
        &device,
        &queue,
        &pipeline,
        &globals_group,
        &quad_group,
        &quad_buffer,
        &readback,
        &texture,
        &view,
        &fractional,
        wgpu::Color::BLACK,
    );
    for (x, y, what) in [
        (16, 32, "left edge"),
        (48, 32, "right edge"),
        (32, 16, "top edge"),
        (32, 48, "bottom edge"),
    ] {
        assert_red_close(pixel(&data, x, y), 191, what);
    }
    assert_eq!(pixel(&data, 15, 32)[0], 0, "outside the quad must stay clear");
    assert_eq!(pixel(&data, 49, 32)[0], 0, "outside the quad must stay clear");
    assert_eq!(pixel(&data, 32, 32)[0], 255, "interior must be fully covered");

    // Case 2: the unrounded mask's right edge sits 0.25px inside pixel 40 -> coverage 0.75.
    let masked = Quad {
        bounds: bounds(
            point(ScaledPixels(8.0), ScaledPixels(8.0)),
            size(ScaledPixels(48.0), ScaledPixels(48.0)),
        ),
        content_mask: ContentMask {
            bounds: bounds(
                point(ScaledPixels(8.0), ScaledPixels(8.0)),
                size(ScaledPixels(32.75), ScaledPixels(48.0)),
            ),
            corner_radii: Corners::default(),
                    fade_out: Default::default(),
        },
        background: Background::from(rgba(0xff0000ff)),
        mask_chain: mask_chain_ref(1, 1),
        ..Default::default()
    };
    let data = render_quad(
        &device,
        &queue,
        &pipeline,
        &globals_group,
        &quad_group,
        &quad_buffer,
        &readback,
        &texture,
        &view,
        &masked,
        wgpu::Color::BLACK,
    );
    assert_red_close(pixel(&data, 40, 32), 191, "partially covered mask edge");
    assert_eq!(pixel(&data, 41, 32)[0], 0, "pixel past the mask must be clipped");
    assert_eq!(pixel(&data, 39, 32)[0], 255, "pixel inside the mask must be solid");

    // Case 3: transparent background over a transparent border is 0/0 in `over`; the guard
    // must keep NaN out of the framebuffer (the clear colour survives).
    let transparent_border = Quad {
        bounds: bounds(
            point(ScaledPixels(16.0), ScaledPixels(16.0)),
            size(ScaledPixels(32.0), ScaledPixels(32.0)),
        ),
        content_mask: viewport_mask(),
        background: Background::from(transparent_black()),
        border_color: transparent_black().into(),
        border_widths: Edges {
            top: ScaledPixels(4.0),
            right: ScaledPixels(4.0),
            bottom: ScaledPixels(4.0),
            left: ScaledPixels(4.0),
        },
        ..Default::default()
    };
    let data = render_quad(
        &device,
        &queue,
        &pipeline,
        &globals_group,
        &quad_group,
        &quad_buffer,
        &readback,
        &texture,
        &view,
        &transparent_border,
        wgpu::Color {
            r: 0.0,
            g: 0.0,
            b: 1.0,
            a: 1.0,
        },
    );
    for (x, y) in [(17, 32), (47, 32), (32, 17), (32, 47)] {
        assert_eq!(
            pixel(&data, x, y),
            [0, 0, 255, 255],
            "transparent border must leave the blue clear colour at ({x}, {y})"
        );
    }
}
