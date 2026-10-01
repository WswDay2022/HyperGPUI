use std::{
    slice,
    sync::{Arc, OnceLock},
};

use anyhow::{Context, Result};
use hgpui::ResultExt;
use windows::{
    Win32::{
        Foundation::HWND,
        Graphics::{
            Direct3D::*,
            Direct3D11::*,
            DirectComposition::*,
            DirectWrite::*,
            Dxgi::{Common::*, *},
        },
    },
    core::{HSTRING, Interface},
};

use crate::directx_renderer::shader_resources::{RawShaderBytes, ShaderModule, ShaderTarget};
use crate::*;
use hgpui::*;

/// The largest blur radius in a scene-space filter chain, in device pixels — used to size the
/// blur kernel and the dilated region the blur passes are scissored to.
///
/// The `match` is exhaustive on purpose: adding a [`ScaledFilter`] variant breaks it here,
/// forcing this backend to handle (or deliberately ignore) the new filter rather than silently
/// dropping it.
fn max_blur_radius(filters: &[ScaledFilter]) -> f32 {
    filters.iter().fold(0.0, |acc, filter| match filter {
        ScaledFilter::Blur(radius) => acc.max(radius.0),
    })
}

pub(crate) const DISABLE_DIRECT_COMPOSITION: &str = "GPUI_DISABLE_DIRECT_COMPOSITION";
const RENDER_TARGET_FORMAT: DXGI_FORMAT = DXGI_FORMAT_B8G8R8A8_UNORM;
// This configuration is used for MSAA rendering on paths only, and it's guaranteed to be supported by DirectX 11.
const PATH_MULTISAMPLE_COUNT: u32 = 4;

/// Number of content-filter (`filter`) nesting levels that get their own isolated group target.
/// Two covers the realistic "a blurred element inside another blurred element" case; deeper nests
/// render inline (unblurred at the inner level) rather than allocating unbounded VRAM. Must match
/// the wgpu backend's `MAX_FILTER_DEPTH` so nested blur renders consistently across platforms.
const MAX_FILTER_DEPTH: usize = 2;

pub(crate) struct FontInfo {
    pub gamma_ratios: [f32; 4],
    pub grayscale_enhanced_contrast: f32,
    pub subpixel_enhanced_contrast: f32,
    pub is_bgr: bool,
}

pub(crate) struct DirectXRenderer {
    hwnd: HWND,
    atlas: Arc<DirectXAtlas>,
    devices: Option<DirectXRendererDevices>,
    resources: Option<DirectXResources>,
    globals: DirectXGlobalElements,
    pipelines: DirectXRenderPipelines,
    direct_composition: Option<DirectComposition>,
    font_info: &'static FontInfo,

    width: u32,
    height: u32,

    /// Whether we want to skip drawing due to device lost events.
    ///
    /// In that case we want to discard the first frame that we draw as we got reset in the middle of a frame
    /// meaning we lost all the allocated gpu textures and scene resources.
    skip_draws: bool,

    /// The render target currently bound for the main scene this frame (the offscreen
    /// `scene_color` when blur filters are present, a content-filter group texture inside such a
    /// group, or the swapchain otherwise). `draw_paths_to_intermediate` restores to this after
    /// its own pass so paths land on the correct target.
    active_render_target: Option<ID3D11RenderTargetView>,
}

/// Direct3D objects
#[derive(Clone)]
pub(crate) struct DirectXRendererDevices {
    pub(crate) adapter: IDXGIAdapter1,
    pub(crate) dxgi_factory: IDXGIFactory6,
    pub(crate) device: ID3D11Device,
    pub(crate) device_context: ID3D11DeviceContext,
    dxgi_device: Option<IDXGIDevice>,
    annotation: Option<ID3DUserDefinedAnnotation>,
}

struct DirectXResources {
    // Direct3D rendering objects
    swap_chain: IDXGISwapChain1,
    render_target: Option<ID3D11Texture2D>,
    render_target_view: Option<ID3D11RenderTargetView>,

    // Path intermediate textures (with MSAA)
    path_intermediate_texture: ID3D11Texture2D,
    path_intermediate_srv: Option<ID3D11ShaderResourceView>,
    path_intermediate_msaa_texture: ID3D11Texture2D,
    path_intermediate_msaa_view: Option<ID3D11RenderTargetView>,

    // Offscreen targets for blur filters (each is render-target + shader-resource).
    blur: BlurResources,

    // Cached viewport
    viewport: D3D11_VIEWPORT,
}

/// Offscreen render targets used by the blur filters. The scene is rendered into `scene_color`
/// (so filters can sample it), `ping`/`pong` are half-resolution scratch for the separable
/// gaussian, and `groups` isolate content-filter (`filter`) subtrees — one per nesting level
/// (indexed by isolation depth), up to [`MAX_FILTER_DEPTH`], so nested content blurs isolate
/// correctly; deeper nests render inline.
struct BlurResources {
    scene_color: ID3D11Texture2D,
    scene_color_rtv: Option<ID3D11RenderTargetView>,
    scene_color_srv: Option<ID3D11ShaderResourceView>,
    #[expect(dead_code)]
    ping: ID3D11Texture2D,
    ping_rtv: Option<ID3D11RenderTargetView>,
    ping_srv: Option<ID3D11ShaderResourceView>,
    #[expect(dead_code)]
    pong: ID3D11Texture2D,
    pong_rtv: Option<ID3D11RenderTargetView>,
    pong_srv: Option<ID3D11ShaderResourceView>,
    // Kept alive for the lifetime of their views; indexed by isolation depth.
    #[expect(dead_code)]
    groups: Vec<ID3D11Texture2D>,
    group_rtvs: Vec<Option<ID3D11RenderTargetView>>,
    group_srvs: Vec<Option<ID3D11ShaderResourceView>>,
}

impl BlurResources {
    fn new(device: &ID3D11Device, width: u32, height: u32) -> Result<Self> {
        let half_w = (width / 2).max(1);
        let half_h = (height / 2).max(1);
        let (scene_color, scene_color_rtv, scene_color_srv) =
            create_color_target(device, width, height)?;
        let (ping, ping_rtv, ping_srv) = create_color_target(device, half_w, half_h)?;
        let (pong, pong_rtv, pong_srv) = create_color_target(device, half_w, half_h)?;
        let mut groups = Vec::with_capacity(MAX_FILTER_DEPTH);
        let mut group_rtvs = Vec::with_capacity(MAX_FILTER_DEPTH);
        let mut group_srvs = Vec::with_capacity(MAX_FILTER_DEPTH);
        for _ in 0..MAX_FILTER_DEPTH {
            let (group, group_rtv, group_srv) = create_color_target(device, width, height)?;
            groups.push(group);
            group_rtvs.push(group_rtv);
            group_srvs.push(group_srv);
        }
        Ok(Self {
            scene_color,
            scene_color_rtv,
            scene_color_srv,
            ping,
            ping_rtv,
            ping_srv,
            pong,
            pong_rtv,
            pong_srv,
            groups,
            group_rtvs,
            group_srvs,
        })
    }
}

struct DirectXRenderPipelines {
    shadow_pipeline: PipelineState<Shadow>,
    quad_pipeline: PipelineState<Quad>,
    path_rasterization_pipeline: PipelineState<PathRasterizationSprite>,
    path_sprite_pipeline: PipelineState<PathSprite>,
    underline_pipeline: PipelineState<Underline>,
    mono_sprites: PipelineState<MonochromeSprite>,
    subpixel_sprites: PipelineState<SubpixelSprite>,
    poly_sprites: PipelineState<PolychromeSprite>,
    // Blur (backdrop-filter / filter). These don't use the generic PipelineState since they
    // sample a texture rather than read a structured instance buffer; their parameters live in
    // a dedicated constant buffer at register b1.
    blur_downsample_vertex: ID3D11VertexShader,
    blur_downsample_fragment: ID3D11PixelShader,
    blur_vertex: ID3D11VertexShader,
    blur_fragment: ID3D11PixelShader,
    blur_composite_vertex: ID3D11VertexShader,
    blur_composite_fragment: ID3D11PixelShader,
    blur_params_buffer: ID3D11Buffer,
    blur_blend_replace: ID3D11BlendState,
    blur_blend_composite: ID3D11BlendState,
}

struct DirectXGlobalElements {
    global_params_buffer: Option<ID3D11Buffer>,
    sampler: Option<ID3D11SamplerState>,
    /// Clamp-addressed sampler used by the blur passes: gaussian taps may leave the texture at
    /// the window edges, and must clamp like CSS rather than wrap around to the far side.
    blur_sampler: Option<ID3D11SamplerState>,
}

struct Annotation<'a>(&'a ID3DUserDefinedAnnotation);

impl<'a> Annotation<'a> {
    fn new(annotation: &'a ID3DUserDefinedAnnotation, label: HSTRING) -> Self {
        unsafe { annotation.BeginEvent(&label) };
        Self(annotation)
    }
}

impl Drop for Annotation<'_> {
    fn drop(&mut self) {
        unsafe { self.0.EndEvent() };
    }
}

struct DirectComposition {
    comp_device: IDCompositionDevice,
    comp_target: IDCompositionTarget,
    comp_visual: IDCompositionVisual,
}

impl DirectXRendererDevices {
    pub(crate) fn new(
        directx_devices: &DirectXDevices,
        disable_direct_composition: bool,
    ) -> Result<Self> {
        let DirectXDevices {
            adapter,
            dxgi_factory,
            device,
            device_context,
        } = directx_devices;
        let dxgi_device = if disable_direct_composition {
            None
        } else {
            Some(device.cast().context("Creating DXGI device")?)
        };
        let annotation = device_context.cast().ok();

        Ok(Self {
            adapter: adapter.clone(),
            dxgi_factory: dxgi_factory.clone(),
            device: device.clone(),
            device_context: device_context.clone(),
            dxgi_device,
            annotation,
        })
    }
}

impl DirectXRenderer {
    pub(crate) fn new(
        hwnd: HWND,
        directx_devices: &DirectXDevices,
        disable_direct_composition: bool,
    ) -> Result<Self> {
        if disable_direct_composition {
            log::info!("Direct Composition is disabled.");
        }

        let devices = DirectXRendererDevices::new(directx_devices, disable_direct_composition)
            .context("Creating DirectX devices")?;
        let atlas = Arc::new(DirectXAtlas::new(&devices.device, &devices.device_context));

        let resources = DirectXResources::new(&devices, 1, 1, hwnd, disable_direct_composition)
            .context("Creating DirectX resources")?;
        let globals = DirectXGlobalElements::new(&devices.device)
            .context("Creating DirectX global elements")?;
        let pipelines = DirectXRenderPipelines::new(&devices.device)
            .context("Creating DirectX render pipelines")?;

        let direct_composition = if disable_direct_composition {
            None
        } else {
            let composition = DirectComposition::new(devices.dxgi_device.as_ref().unwrap(), hwnd)
                .context("Creating DirectComposition")?;
            composition
                .set_swap_chain(&resources.swap_chain)
                .context("Setting swap chain for DirectComposition")?;
            Some(composition)
        };

        Ok(DirectXRenderer {
            hwnd,
            atlas,
            devices: Some(devices),
            resources: Some(resources),
            globals,
            pipelines,
            direct_composition,
            font_info: Self::get_font_info(),
            width: 1,
            height: 1,
            skip_draws: false,
            active_render_target: None,
        })
    }

    pub(crate) fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.atlas.clone()
    }

    fn pre_draw(&self, clear_color: &[f32; 4]) -> Result<()> {
        let resources = self.resources.as_ref().expect("resources missing");
        let device_context = &self
            .devices
            .as_ref()
            .expect("devices missing")
            .device_context;
        update_buffer(
            device_context,
            self.globals.global_params_buffer.as_ref().unwrap(),
            &[GlobalParams {
                gamma_ratios: self.font_info.gamma_ratios,
                viewport_size: [resources.viewport.Width, resources.viewport.Height],
                grayscale_enhanced_contrast: self.font_info.grayscale_enhanced_contrast,
                subpixel_enhanced_contrast: self.font_info.subpixel_enhanced_contrast,
                is_bgr: self.font_info.is_bgr as u32,
                _pad: [0; 3],
            }],
        )?;
        unsafe {
            device_context.ClearRenderTargetView(
                resources
                    .render_target_view
                    .as_ref()
                    .context("missing render target view")?,
                clear_color,
            );
            device_context
                .OMSetRenderTargets(Some(slice::from_ref(&resources.render_target_view)), None);
            device_context.RSSetViewports(Some(slice::from_ref(&resources.viewport)));
        }
        Ok(())
    }

    #[inline]
    fn present(&mut self) -> Result<()> {
        let result = unsafe {
            self.resources
                .as_ref()
                .expect("resources missing")
                .swap_chain
                .Present(0, DXGI_PRESENT(0))
        };
        result.ok().context("Presenting swap chain failed")
    }

    pub(crate) fn handle_device_lost(&mut self, directx_devices: &DirectXDevices) -> Result<()> {
        try_to_recover_from_device_lost(|| {
            self.handle_device_lost_impl(directx_devices)
                .context("DirectXRenderer handling device lost")
        })
    }

    fn handle_device_lost_impl(&mut self, directx_devices: &DirectXDevices) -> Result<()> {
        let disable_direct_composition = self.direct_composition.is_none();

        unsafe {
            #[cfg(debug_assertions)]
            if let Some(devices) = &self.devices {
                report_live_objects(&devices.device)
                    .context("Failed to report live objects after device lost")
                    .log_err();
            }

            self.resources.take();
            if let Some(devices) = &self.devices {
                devices.device_context.OMSetRenderTargets(None, None);
                devices.device_context.ClearState();
                devices.device_context.Flush();
                #[cfg(debug_assertions)]
                report_live_objects(&devices.device)
                    .context("Failed to report live objects after device lost")
                    .log_err();
            }

            self.direct_composition.take();
            self.devices.take();
        }

        let devices = DirectXRendererDevices::new(directx_devices, disable_direct_composition)
            .context("Recreating DirectX devices")?;
        let resources = DirectXResources::new(
            &devices,
            self.width,
            self.height,
            self.hwnd,
            disable_direct_composition,
        )
        .context("Creating DirectX resources")?;
        let globals = DirectXGlobalElements::new(&devices.device)
            .context("Creating DirectXGlobalElements")?;
        let pipelines = DirectXRenderPipelines::new(&devices.device)
            .context("Creating DirectXRenderPipelines")?;

        let direct_composition = if disable_direct_composition {
            None
        } else {
            let composition =
                DirectComposition::new(devices.dxgi_device.as_ref().unwrap(), self.hwnd)?;
            composition.set_swap_chain(&resources.swap_chain)?;
            Some(composition)
        };

        self.atlas
            .handle_device_lost(&devices.device, &devices.device_context);

        unsafe {
            devices
                .device_context
                .OMSetRenderTargets(Some(slice::from_ref(&resources.render_target_view)), None);
        }
        self.devices = Some(devices);
        self.resources = Some(resources);
        self.globals = globals;
        self.pipelines = pipelines;
        self.direct_composition = direct_composition;
        self.skip_draws = true;
        Ok(())
    }

    pub(crate) fn draw(
        &mut self,
        scene: &Scene,
        background_appearance: WindowBackgroundAppearance,
    ) -> Result<()> {
        if self.skip_draws {
            // skip drawing this frame, we just recovered from a device lost event
            // and so likely do not have the textures anymore that are required for drawing
            return Ok(());
        }
        self.pre_draw(&match background_appearance {
            WindowBackgroundAppearance::Opaque => [1.0f32; 4],
            _ => [0.0f32; 4],
        })?;

        self.upload_scene_buffers(scene)?;

        // Only route through the offscreen scene texture when the scene contains blur filters;
        // otherwise render straight to the swapchain exactly as before.
        let use_offscreen =
            !scene.backdrop_filters.is_empty() || !scene.filter_boundaries.is_empty();

        // Clone the views we need (AddRef) so the loop can rebind render targets without holding a
        // borrow of `self` across the `&mut self` draw_* calls.
        let (scene_rtv, scene_srv, group_rtvs, group_srvs, swapchain_rtv) = {
            let r = self.resources.as_ref().context("resources missing")?;
            (
                r.blur.scene_color_rtv.clone(),
                r.blur.scene_color_srv.clone(),
                r.blur.group_rtvs.clone(),
                r.blur.group_srvs.clone(),
                r.render_target_view.clone(),
            )
        };
        let ctx = self
            .devices
            .as_ref()
            .context("devices missing")?
            .device_context
            .clone();

        if use_offscreen {
            unsafe {
                if let Some(rtv) = scene_rtv.as_ref() {
                    ctx.ClearRenderTargetView(rtv, &[0.0; 4]);
                }
                ctx.OMSetRenderTargets(Some(slice::from_ref(&scene_rtv)), None);
            }
            self.active_render_target = scene_rtv;
        } else {
            self.active_render_target = swapchain_rtv.clone();
        }

        // Current target for the main scene + a parent stack for content-filter groups.
        let mut current_rtv = self.active_render_target.clone();
        let mut current_srv = if use_offscreen {
            scene_srv.clone()
        } else {
            None
        };
        // (parent_rtv, parent_srv, isolated)
        let mut filter_stack: Vec<(
            Option<ID3D11RenderTargetView>,
            Option<ID3D11ShaderResourceView>,
            bool,
        )> = Vec::new();

        let annotation = self
            .devices
            .as_ref()
            .and_then(|devices| devices.annotation.clone())
            .filter(|annotation| unsafe { annotation.GetStatus().as_bool() });
        for batch in scene.batches() {
            let _annotation = annotation
                .as_ref()
                .map(|annotation| Annotation::new(annotation, HSTRING::from(batch.label())));
            match batch {
                PrimitiveBatch::Shadows(range) => self.draw_shadows(range.start, range.len()),
                PrimitiveBatch::Quads(range) => self.draw_quads(range.start, range.len()),
                PrimitiveBatch::Paths(range) => {
                    let paths = &scene.paths[range];
                    self.draw_paths_to_intermediate(paths)?;
                    self.draw_paths_from_intermediate(paths)
                }
                PrimitiveBatch::Underlines(range) => self.draw_underlines(range.start, range.len()),
                PrimitiveBatch::MonochromeSprites { texture_id, range } => {
                    self.draw_monochrome_sprites(texture_id, range.start, range.len())
                }
                PrimitiveBatch::SubpixelSprites { texture_id, range } => {
                    self.draw_subpixel_sprites(texture_id, range.start, range.len())
                }
                PrimitiveBatch::PolychromeSprites { texture_id, range } => {
                    self.draw_polychrome_sprites(texture_id, range.start, range.len())
                }
                PrimitiveBatch::Surfaces(range) => self.draw_surfaces(&scene.surfaces[range]),
                PrimitiveBatch::BackdropFilters(range) => {
                    let result = (|| {
                        for filter in &scene.backdrop_filters[range] {
                            self.dx_blur_and_composite(
                                &current_srv,
                                &current_rtv,
                                filter.bounds,
                                filter.content_mask,
                                corner_radii_array(filter.corner_radii),
                                max_blur_radius(&filter.filters),
                                filter.opacity,
                                true,
                            )?;
                        }
                        Ok::<(), anyhow::Error>(())
                    })();
                    // Restore the current target for subsequent batches.
                    unsafe {
                        ctx.OMSetRenderTargets(Some(slice::from_ref(&current_rtv)), None);
                    }
                    result
                }
                PrimitiveBatch::FilterBoundary(ix) => {
                    let boundary = scene.filter_boundaries[ix].clone();
                    if boundary.is_start {
                        // Each isolated nesting level uses its own group target from the pool
                        // (indexed by current isolation depth). Beyond the pool size
                        // (MAX_FILTER_DEPTH) deeper filters render inline without isolation rather
                        // than corrupting an outer group.
                        let depth = filter_stack.iter().filter(|entry| entry.2).count();
                        if depth < group_rtvs.len() {
                            filter_stack.push((current_rtv.clone(), current_srv.clone(), true));
                            current_rtv = group_rtvs[depth].clone();
                            current_srv = group_srvs[depth].clone();
                            self.active_render_target = current_rtv.clone();
                            unsafe {
                                if let Some(rtv) = current_rtv.as_ref() {
                                    ctx.ClearRenderTargetView(rtv, &[0.0; 4]);
                                }
                                ctx.OMSetRenderTargets(Some(slice::from_ref(&current_rtv)), None);
                            }
                        } else {
                            filter_stack.push((current_rtv.clone(), current_srv.clone(), false));
                        }
                        Ok(())
                    } else if let Some((parent_rtv, parent_srv, isolated)) = filter_stack.pop() {
                        let result = if isolated {
                            self.dx_blur_and_composite(
                                &current_srv,
                                &parent_rtv,
                                boundary.bounds,
                                boundary.content_mask,
                                corner_radii_array(boundary.corner_radii),
                                max_blur_radius(&boundary.filters),
                                boundary.opacity,
                                false,
                            )
                        } else {
                            Ok(())
                        };
                        current_rtv = parent_rtv;
                        current_srv = parent_srv;
                        self.active_render_target = current_rtv.clone();
                        unsafe {
                            ctx.OMSetRenderTargets(Some(slice::from_ref(&current_rtv)), None);
                        }
                        result
                    } else {
                        Ok(())
                    }
                }
            }
            .with_context(|| {
                format!(
                    "scene too large:\
                    {} paths, {} shadows, {} quads, {} underlines, {} mono, {} subpixel, {} poly, {} surfaces",
                    scene.paths.len(),
                    scene.shadows.len(),
                    scene.quads.len(),
                    scene.underlines.len(),
                    scene.monochrome_sprites.len(),
                    scene.subpixel_sprites.len(),
                    scene.polychrome_sprites.len(),
                    scene.surfaces.len(),
                )
            })?;
        }

        // Present the offscreen scene by copying it into the swapchain. The swapchain was created
        // with the same format, size and sample count as the scene texture, so a resource copy is
        // exact — and far cheaper than the full-screen draw it replaces.
        if use_offscreen {
            let resources = self.resources.as_ref().context("resources missing")?;
            let target = resources
                .render_target
                .as_ref()
                .context("render target texture missing")?;
            self.dx_blit(&resources.blur.scene_color, target)?;
        }
        self.active_render_target = None;
        self.present()
    }

    pub(crate) fn resize(&mut self, new_size: Size<DevicePixels>) -> Result<()> {
        let width = new_size.width.0.max(1) as u32;
        let height = new_size.height.0.max(1) as u32;
        if self.width == width && self.height == height {
            return Ok(());
        }
        self.width = width;
        self.height = height;

        // Clear the render target before resizing
        let devices = self.devices.as_ref().context("devices missing")?;
        unsafe { devices.device_context.OMSetRenderTargets(None, None) };
        let resources = self.resources.as_mut().context("resources missing")?;
        resources.render_target.take();
        resources.render_target_view.take();

        // Resizing the swap chain requires a call to the underlying DXGI adapter, which can return the device removed error.
        // The app might have moved to a monitor that's attached to a different graphics device.
        // When a graphics device is removed or reset, the desktop resolution often changes, resulting in a window size change.
        // But here we just return the error, because we are handling device lost scenarios elsewhere.
        unsafe {
            resources
                .swap_chain
                .ResizeBuffers(
                    BUFFER_COUNT as u32,
                    width,
                    height,
                    RENDER_TARGET_FORMAT,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )
                .context("Failed to resize swap chain")?;
        }

        resources.recreate_resources(devices, width, height)?;

        unsafe {
            devices
                .device_context
                .OMSetRenderTargets(Some(slice::from_ref(&resources.render_target_view)), None);
        }

        Ok(())
    }

    fn upload_scene_buffers(&mut self, scene: &Scene) -> Result<()> {
        let devices = self.devices.as_ref().context("devices missing")?;

        if !scene.shadows.is_empty() {
            self.pipelines.shadow_pipeline.update_buffer(
                &devices.device,
                &devices.device_context,
                &scene.shadows,
            )?;
        }

        if !scene.quads.is_empty() {
            self.pipelines.quad_pipeline.update_buffer(
                &devices.device,
                &devices.device_context,
                &scene.quads,
            )?;
        }

        if !scene.underlines.is_empty() {
            self.pipelines.underline_pipeline.update_buffer(
                &devices.device,
                &devices.device_context,
                &scene.underlines,
            )?;
        }

        if !scene.monochrome_sprites.is_empty() {
            self.pipelines.mono_sprites.update_buffer(
                &devices.device,
                &devices.device_context,
                &scene.monochrome_sprites,
            )?;
        }

        if !scene.subpixel_sprites.is_empty() {
            self.pipelines.subpixel_sprites.update_buffer(
                &devices.device,
                &devices.device_context,
                &scene.subpixel_sprites,
            )?;
        }

        if !scene.polychrome_sprites.is_empty() {
            self.pipelines.poly_sprites.update_buffer(
                &devices.device,
                &devices.device_context,
                &scene.polychrome_sprites,
            )?;
        }

        Ok(())
    }

    fn draw_shadows(&mut self, start: usize, len: usize) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let devices = self.devices.as_ref().context("devices missing")?;
        self.pipelines.shadow_pipeline.draw_range(
            &devices.device,
            &devices.device_context,
            slice::from_ref(
                &self
                    .resources
                    .as_ref()
                    .context("resources missing")?
                    .viewport,
            ),
            slice::from_ref(&self.globals.global_params_buffer),
            4,
            start as u32,
            len as u32,
        )
    }

    fn draw_quads(&mut self, start: usize, len: usize) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let devices = self.devices.as_ref().context("devices missing")?;
        self.pipelines.quad_pipeline.draw_range(
            &devices.device,
            &devices.device_context,
            slice::from_ref(
                &self
                    .resources
                    .as_ref()
                    .context("resources missing")?
                    .viewport,
            ),
            slice::from_ref(&self.globals.global_params_buffer),
            4,
            start as u32,
            len as u32,
        )
    }

    fn draw_paths_to_intermediate(&mut self, paths: &[Path<ScaledPixels>]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }

        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        // Clear intermediate MSAA texture
        unsafe {
            devices.device_context.ClearRenderTargetView(
                resources.path_intermediate_msaa_view.as_ref().unwrap(),
                &[0.0; 4],
            );
            // Set intermediate MSAA texture as render target
            devices.device_context.OMSetRenderTargets(
                Some(slice::from_ref(&resources.path_intermediate_msaa_view)),
                None,
            );
        }

        // Collect all vertices and sprites for a single draw call
        let mut vertices = Vec::new();

        for path in paths {
            vertices.extend(path.vertices.iter().map(|v| PathRasterizationSprite {
                xy_position: v.xy_position,
                st_position: v.st_position,
                color: path.color,
                bounds: path.clipped_bounds(),
            }));
        }

        self.pipelines.path_rasterization_pipeline.update_buffer(
            &devices.device,
            &devices.device_context,
            &vertices,
        )?;

        self.pipelines.path_rasterization_pipeline.draw(
            &devices.device_context,
            slice::from_ref(&resources.viewport),
            slice::from_ref(&self.globals.global_params_buffer),
            D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            vertices.len() as u32,
            1,
        )?;

        // Resolve MSAA to non-MSAA intermediate texture
        unsafe {
            devices.device_context.ResolveSubresource(
                &resources.path_intermediate_texture,
                0,
                &resources.path_intermediate_msaa_texture,
                0,
                RENDER_TARGET_FORMAT,
            );
            // Restore the active render target (the offscreen scene/group target when blurring,
            // otherwise the swapchain) so the path sprites land on the correct surface.
            let restore_target = if self.active_render_target.is_some() {
                &self.active_render_target
            } else {
                &resources.render_target_view
            };
            devices
                .device_context
                .OMSetRenderTargets(Some(slice::from_ref(restore_target)), None);
        }

        Ok(())
    }

    fn draw_paths_from_intermediate(&mut self, paths: &[Path<ScaledPixels>]) -> Result<()> {
        let Some(first_path) = paths.first() else {
            return Ok(());
        };

        // When copying paths from the intermediate texture to the drawable,
        // each pixel must only be copied once, in case of transparent paths.
        //
        // If all paths have the same draw order, then their bounds are all
        // disjoint, so we can copy each path's bounds individually. If this
        // batch combines different draw orders, we perform a single copy
        // for a minimal spanning rect.
        let sprites = if paths.last().unwrap().order == first_path.order {
            paths
                .iter()
                .map(|path| PathSprite {
                    bounds: path.clipped_bounds(),
                })
                .collect::<Vec<_>>()
        } else {
            let mut bounds = first_path.clipped_bounds();
            for path in paths.iter().skip(1) {
                bounds = bounds.union(&path.clipped_bounds());
            }
            vec![PathSprite { bounds }]
        };

        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        self.pipelines.path_sprite_pipeline.update_buffer(
            &devices.device,
            &devices.device_context,
            &sprites,
        )?;

        // Draw the sprites with the path texture
        self.pipelines.path_sprite_pipeline.draw_with_texture(
            &devices.device_context,
            slice::from_ref(&resources.path_intermediate_srv),
            slice::from_ref(&resources.viewport),
            slice::from_ref(&self.globals.global_params_buffer),
            slice::from_ref(&self.globals.sampler),
            sprites.len() as u32,
        )
    }

    fn draw_underlines(&mut self, start: usize, len: usize) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        self.pipelines.underline_pipeline.draw_range(
            &devices.device,
            &devices.device_context,
            slice::from_ref(&resources.viewport),
            slice::from_ref(&self.globals.global_params_buffer),
            4,
            start as u32,
            len as u32,
        )
    }

    fn draw_monochrome_sprites(
        &mut self,
        texture_id: AtlasTextureId,
        start: usize,
        len: usize,
    ) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        let texture_view = self.atlas.get_texture_view(texture_id);
        self.pipelines.mono_sprites.draw_range_with_texture(
            &devices.device,
            &devices.device_context,
            &texture_view,
            slice::from_ref(&resources.viewport),
            slice::from_ref(&self.globals.global_params_buffer),
            slice::from_ref(&self.globals.sampler),
            start as u32,
            len as u32,
        )
    }

    fn draw_subpixel_sprites(
        &mut self,
        texture_id: AtlasTextureId,
        start: usize,
        len: usize,
    ) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        let texture_view = self.atlas.get_texture_view(texture_id);
        self.pipelines.subpixel_sprites.draw_range_with_texture(
            &devices.device,
            &devices.device_context,
            &texture_view,
            slice::from_ref(&resources.viewport),
            slice::from_ref(&self.globals.global_params_buffer),
            slice::from_ref(&self.globals.sampler),
            start as u32,
            len as u32,
        )
    }

    fn draw_polychrome_sprites(
        &mut self,
        texture_id: AtlasTextureId,
        start: usize,
        len: usize,
    ) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        let texture_view = self.atlas.get_texture_view(texture_id);
        self.pipelines.poly_sprites.draw_range_with_texture(
            &devices.device,
            &devices.device_context,
            &texture_view,
            slice::from_ref(&resources.viewport),
            slice::from_ref(&self.globals.global_params_buffer),
            slice::from_ref(&self.globals.sampler),
            start as u32,
            len as u32,
        )
    }

    fn draw_surfaces(&mut self, surfaces: &[PaintSurface]) -> Result<()> {
        if surfaces.is_empty() {
            return Ok(());
        }
        Ok(())
    }

    /// Run a single blur pass: a full-screen (or composite) draw sampling `source_srv` into
    /// `target_rtv`, with `params` in the blur constant buffer (b1).
    #[allow(clippy::too_many_arguments)]
    fn dx_blur_pass(
        &self,
        vertex: &ID3D11VertexShader,
        fragment: &ID3D11PixelShader,
        blend: &ID3D11BlendState,
        target_rtv: &Option<ID3D11RenderTargetView>,
        source_srv: &Option<ID3D11ShaderResourceView>,
        params: BlurParams,
        viewport: &D3D11_VIEWPORT,
        topology: D3D_PRIMITIVE_TOPOLOGY,
        vertex_count: u32,
        clear: bool,
    ) -> Result<()> {
        let devices = self.devices.as_ref().context("devices missing")?;
        let ctx = &devices.device_context;
        update_buffer(ctx, &self.pipelines.blur_params_buffer, &[params])?;
        let null_srv: [Option<ID3D11ShaderResourceView>; 1] = [None];
        let blur_params = [Some(self.pipelines.blur_params_buffer.clone())];
        unsafe {
            // Unbind any SRV at slot 0 so the target texture isn't simultaneously bound as input.
            ctx.PSSetShaderResources(0, Some(&null_srv));
            if clear {
                ctx.ClearRenderTargetView(
                    target_rtv.as_ref().context("blur target view missing")?,
                    &[0.0; 4],
                );
            }
            ctx.OMSetRenderTargets(Some(slice::from_ref(target_rtv)), None);
            ctx.RSSetViewports(Some(slice::from_ref(viewport)));
            ctx.IASetPrimitiveTopology(topology);
            ctx.VSSetShader(vertex, None);
            ctx.PSSetShader(fragment, None);
            ctx.VSSetConstantBuffers(0, Some(slice::from_ref(&self.globals.global_params_buffer)));
            ctx.PSSetConstantBuffers(0, Some(slice::from_ref(&self.globals.global_params_buffer)));
            ctx.VSSetConstantBuffers(1, Some(&blur_params));
            ctx.PSSetConstantBuffers(1, Some(&blur_params));
            // Blur passes sample with a clamp sampler: gaussian taps that leave the texture at
            // the window edge must not wrap around to the opposite side.
            ctx.PSSetSamplers(0, Some(slice::from_ref(&self.globals.blur_sampler)));
            ctx.PSSetShaderResources(0, Some(slice::from_ref(source_srv)));
            ctx.OMSetBlendState(blend, None, 0xFFFFFFFF);
            ctx.DrawInstanced(vertex_count, 1, 0, 0);
            // Unbind the source so the target can be rebound as a render target next.
            ctx.PSSetShaderResources(0, Some(&null_srv));
        }
        Ok(())
    }

    /// Blur `source_srv` (full-resolution) using the half-res ping/pong textures and composite the
    /// result into `target_rtv`, clipped to `bounds`/`corner_radii`/`content_mask` and modulated
    /// by `opacity`. Shared by the backdrop and content-filter paths.
    #[allow(clippy::too_many_arguments)]
    fn dx_blur_and_composite(
        &self,
        source_srv: &Option<ID3D11ShaderResourceView>,
        target_rtv: &Option<ID3D11RenderTargetView>,
        bounds: Bounds<ScaledPixels>,
        content_mask: ContentMask<ScaledPixels>,
        corner_radii: [f32; 4],
        blur_radius: f32,
        opacity: f32,
        // Backdrop clips to the rounded rect; content (`filter`) bleeds past its bounds.
        clip_rounded: bool,
    ) -> Result<()> {
        // Sigma is halved because the blur runs at half resolution.
        let sigma = (blur_radius * 0.5).max(0.0);
        if sigma <= 0.0 {
            return Ok(());
        }
        // Span ±3σ. If that needs more than 32 taps, spread the taps apart (tap_step > 1) rather
        // than truncating the kernel — keeps very large radii from clipping. Matches wgpu.
        let ideal_taps = (3.0 * sigma).ceil();
        let tap_count = ideal_taps.clamp(1.0, 32.0);
        let tap_step = (ideal_taps / tap_count).max(1.0);
        // Content blur bleeds ~3·radius past the box, so its composite quad covers a dilated rect.
        let composite_bounds = if clip_rounded {
            bounds
        } else {
            bounds.dilate(ScaledPixels(3.0 * blur_radius))
        };
        let half_w = (self.width / 2).max(1);
        let half_h = (self.height / 2).max(1);
        let half_vp = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: half_w as f32,
            Height: half_h as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let (full_vp, ping_rtv, ping_srv, pong_rtv, pong_srv) = {
            let r = self.resources.as_ref().context("resources missing")?;
            (
                r.viewport,
                r.blur.ping_rtv.clone(),
                r.blur.ping_srv.clone(),
                r.blur.pong_rtv.clone(),
                r.blur.pong_srv.clone(),
            )
        };

        // Downsample source -> ping, then separable gaussian ping -> pong -> ping.
        self.dx_blur_pass(
            &self.pipelines.blur_downsample_vertex,
            &self.pipelines.blur_downsample_fragment,
            &self.pipelines.blur_blend_replace,
            &ping_rtv,
            source_srv,
            BlurParams {
                downsample: 1.0,
                // The element's box: this pass mirrors the taps that leave it back inside,
                // rather than letting them read the transparent surround (see `mirror_into_rect`).
                bounds,
                ..Default::default()
            },
            &half_vp,
            D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            3,
            true,
        )?;
        self.dx_blur_pass(
            &self.pipelines.blur_vertex,
            &self.pipelines.blur_fragment,
            &self.pipelines.blur_blend_replace,
            &pong_rtv,
            &ping_srv,
            BlurParams {
                direction: [1.0 / half_w as f32, 0.0],
                sigma,
                tap_count,
                tap_step,
                ..Default::default()
            },
            &half_vp,
            D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            3,
            true,
        )?;
        self.dx_blur_pass(
            &self.pipelines.blur_vertex,
            &self.pipelines.blur_fragment,
            &self.pipelines.blur_blend_replace,
            &ping_rtv,
            &pong_srv,
            BlurParams {
                direction: [0.0, 1.0 / half_h as f32],
                sigma,
                tap_count,
                tap_step,
                ..Default::default()
            },
            &half_vp,
            D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            3,
            true,
        )?;
        // Composite the blurred result into the target (preserving its contents).
        self.dx_blur_pass(
            &self.pipelines.blur_composite_vertex,
            &self.pipelines.blur_composite_fragment,
            &self.pipelines.blur_blend_composite,
            target_rtv,
            &ping_srv,
            BlurParams {
                bounds: composite_bounds,
                content_mask: content_mask.bounds,
                content_mask_radii: corner_radii_array(content_mask.corner_radii),
                corner_radii,
                opacity,
                clip_rounded: if clip_rounded { 1.0 } else { 0.0 },
                ..Default::default()
            },
            &full_vp,
            D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
            4,
            false,
        )?;
        Ok(())
    }

    /// Copy the offscreen scene texture into the swapchain render target. Both textures have the
    /// same format, size and sample count, so this is exact.
    fn dx_blit(&self, source: &ID3D11Texture2D, target: &ID3D11Texture2D) -> Result<()> {
        let ctx = &self
            .devices
            .as_ref()
            .context("devices missing")?
            .device_context;
        unsafe {
            ctx.CopyResource(target, source);
        }
        Ok(())
    }

    pub(crate) fn gpu_specs(&self) -> Result<GpuSpecs> {
        let devices = self.devices.as_ref().context("devices missing")?;
        let desc = unsafe { devices.adapter.GetDesc1() }?;
        let is_software_emulated = (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0;
        let device_name = String::from_utf16_lossy(&desc.Description)
            .trim_matches(char::from(0))
            .to_string();
        let driver_name = match desc.VendorId {
            0x10DE => "NVIDIA Corporation".to_string(),
            0x1002 => "AMD Corporation".to_string(),
            0x8086 => "Intel Corporation".to_string(),
            id => format!("Unknown Vendor (ID: {:#X})", id),
        };
        let driver_version = match desc.VendorId {
            0x10DE => nvidia::get_driver_version(),
            0x1002 => amd::get_driver_version(),
            // For Intel and other vendors, we use the DXGI API to get the driver version.
            _ => dxgi::get_driver_version(&devices.adapter),
        }
        .context("Failed to get gpu driver info")
        .log_err()
        .unwrap_or("Unknown Driver".to_string());
        Ok(GpuSpecs {
            is_software_emulated,
            device_name,
            driver_name,
            driver_info: driver_version,
        })
    }

    pub(crate) fn get_font_info() -> &'static FontInfo {
        static CACHED_FONT_INFO: OnceLock<FontInfo> = OnceLock::new();
        CACHED_FONT_INFO.get_or_init(|| unsafe {
            let factory: IDWriteFactory5 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).unwrap();
            let render_params: IDWriteRenderingParams1 =
                factory.CreateRenderingParams().unwrap().cast().unwrap();
            FontInfo {
                gamma_ratios: hgpui::get_gamma_correction_ratios(render_params.GetGamma()),
                grayscale_enhanced_contrast: render_params.GetGrayscaleEnhancedContrast(),
                subpixel_enhanced_contrast: render_params.GetEnhancedContrast(),
                is_bgr: render_params.GetPixelGeometry() == DWRITE_PIXEL_GEOMETRY_BGR,
            }
        })
    }

    pub(crate) fn mark_drawable(&mut self) {
        self.skip_draws = false;
    }
}

impl DirectXResources {
    pub fn new(
        devices: &DirectXRendererDevices,
        width: u32,
        height: u32,
        hwnd: HWND,
        disable_direct_composition: bool,
    ) -> Result<Self> {
        let swap_chain = if disable_direct_composition {
            create_swap_chain(&devices.dxgi_factory, &devices.device, hwnd, width, height)?
        } else {
            create_swap_chain_for_composition(
                &devices.dxgi_factory,
                &devices.device,
                width,
                height,
            )?
        };

        let (
            render_target,
            render_target_view,
            path_intermediate_texture,
            path_intermediate_srv,
            path_intermediate_msaa_texture,
            path_intermediate_msaa_view,
            viewport,
        ) = create_resources(devices, &swap_chain, width, height)?;
        set_rasterizer_state(&devices.device, &devices.device_context)?;
        let blur = BlurResources::new(&devices.device, width, height)?;

        Ok(Self {
            swap_chain,
            render_target: Some(render_target),
            render_target_view,
            path_intermediate_texture,
            path_intermediate_msaa_texture,
            path_intermediate_msaa_view,
            path_intermediate_srv,
            blur,
            viewport,
        })
    }

    #[inline]
    fn recreate_resources(
        &mut self,
        devices: &DirectXRendererDevices,
        width: u32,
        height: u32,
    ) -> Result<()> {
        let (
            render_target,
            render_target_view,
            path_intermediate_texture,
            path_intermediate_srv,
            path_intermediate_msaa_texture,
            path_intermediate_msaa_view,
            viewport,
        ) = create_resources(devices, &self.swap_chain, width, height)?;
        self.render_target = Some(render_target);
        self.render_target_view = render_target_view;
        self.path_intermediate_texture = path_intermediate_texture;
        self.path_intermediate_msaa_texture = path_intermediate_msaa_texture;
        self.path_intermediate_msaa_view = path_intermediate_msaa_view;
        self.path_intermediate_srv = path_intermediate_srv;
        self.blur = BlurResources::new(&devices.device, width, height)?;
        self.viewport = viewport;
        Ok(())
    }
}

impl DirectXRenderPipelines {
    pub fn new(device: &ID3D11Device) -> Result<Self> {
        let shadow_pipeline = PipelineState::new(
            device,
            "shadow_pipeline",
            ShaderModule::Shadow,
            4,
            create_blend_state(device)?,
        )?;
        let quad_pipeline = PipelineState::new(
            device,
            "quad_pipeline",
            ShaderModule::Quad,
            64,
            create_blend_state(device)?,
        )?;
        let path_rasterization_pipeline = PipelineState::new(
            device,
            "path_rasterization_pipeline",
            ShaderModule::PathRasterization,
            32,
            create_blend_state_for_path_rasterization(device)?,
        )?;
        let path_sprite_pipeline = PipelineState::new(
            device,
            "path_sprite_pipeline",
            ShaderModule::PathSprite,
            4,
            create_blend_state_for_path_sprite(device)?,
        )?;
        let underline_pipeline = PipelineState::new(
            device,
            "underline_pipeline",
            ShaderModule::Underline,
            4,
            create_blend_state(device)?,
        )?;
        let mono_sprites = PipelineState::new(
            device,
            "monochrome_sprite_pipeline",
            ShaderModule::MonochromeSprite,
            512,
            create_blend_state(device)?,
        )?;
        let subpixel_sprites = PipelineState::new(
            device,
            "subpixel_sprite_pipeline",
            ShaderModule::SubpixelSprite,
            512,
            create_blend_state_for_subpixel_rendering(device)?,
        )?;
        let poly_sprites = PipelineState::new(
            device,
            "polychrome_sprite_pipeline",
            ShaderModule::PolychromeSprite,
            16,
            create_blend_state(device)?,
        )?;

        let blur_downsample_vertex = create_vertex_shader(
            device,
            RawShaderBytes::new(ShaderModule::BlurDownsample, ShaderTarget::Vertex)?.as_bytes(),
        )?;
        let blur_downsample_fragment = create_fragment_shader(
            device,
            RawShaderBytes::new(ShaderModule::BlurDownsample, ShaderTarget::Fragment)?.as_bytes(),
        )?;
        let blur_vertex = create_vertex_shader(
            device,
            RawShaderBytes::new(ShaderModule::Blur, ShaderTarget::Vertex)?.as_bytes(),
        )?;
        let blur_fragment = create_fragment_shader(
            device,
            RawShaderBytes::new(ShaderModule::Blur, ShaderTarget::Fragment)?.as_bytes(),
        )?;
        let blur_composite_vertex = create_vertex_shader(
            device,
            RawShaderBytes::new(ShaderModule::BlurComposite, ShaderTarget::Vertex)?.as_bytes(),
        )?;
        let blur_composite_fragment = create_fragment_shader(
            device,
            RawShaderBytes::new(ShaderModule::BlurComposite, ShaderTarget::Fragment)?.as_bytes(),
        )?;
        let blur_params_buffer = create_constant_buffer(device, std::mem::size_of::<BlurParams>())?;
        let blur_blend_replace = create_blend_state_no_blend(device)?;
        // Premultiplied (One / InvSrcAlpha) — the composite outputs a premultiplied blurred sample;
        // straight-alpha blending would darken the faded edges.
        let blur_blend_composite = create_blend_state_for_path_sprite(device)?;

        Ok(Self {
            shadow_pipeline,
            quad_pipeline,
            path_rasterization_pipeline,
            path_sprite_pipeline,
            underline_pipeline,
            mono_sprites,
            subpixel_sprites,
            poly_sprites,
            blur_downsample_vertex,
            blur_downsample_fragment,
            blur_vertex,
            blur_fragment,
            blur_composite_vertex,
            blur_composite_fragment,
            blur_params_buffer,
            blur_blend_replace,
            blur_blend_composite,
        })
    }
}

impl DirectComposition {
    pub fn new(dxgi_device: &IDXGIDevice, hwnd: HWND) -> Result<Self> {
        let comp_device = get_comp_device(dxgi_device)?;
        let comp_target = unsafe { comp_device.CreateTargetForHwnd(hwnd, true) }?;
        let comp_visual = unsafe { comp_device.CreateVisual() }?;

        Ok(Self {
            comp_device,
            comp_target,
            comp_visual,
        })
    }

    pub fn set_swap_chain(&self, swap_chain: &IDXGISwapChain1) -> Result<()> {
        unsafe {
            self.comp_visual.SetContent(swap_chain)?;
            self.comp_target.SetRoot(&self.comp_visual)?;
            self.comp_device.Commit()?;
        }
        Ok(())
    }
}

impl DirectXGlobalElements {
    pub fn new(device: &ID3D11Device) -> Result<Self> {
        let global_params_buffer = unsafe {
            let desc = D3D11_BUFFER_DESC {
                ByteWidth: std::mem::size_of::<GlobalParams>() as u32,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                ..Default::default()
            };
            let mut buffer = None;
            device.CreateBuffer(&desc, None, Some(&mut buffer))?;
            buffer
        };

        let sampler = unsafe {
            let desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_WRAP,
                AddressV: D3D11_TEXTURE_ADDRESS_WRAP,
                AddressW: D3D11_TEXTURE_ADDRESS_WRAP,
                MipLODBias: 0.0,
                MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_ALWAYS,
                BorderColor: [0.0; 4],
                MinLOD: 0.0,
                MaxLOD: D3D11_FLOAT32_MAX,
            };
            let mut output = None;
            device.CreateSamplerState(&desc, Some(&mut output))?;
            output
        };

        let blur_sampler = create_blur_sampler(device)?;

        Ok(Self {
            global_params_buffer,
            sampler,
            blur_sampler,
        })
    }
}

/// The sampler the blur passes share: linear, clamped. Taps may leave the texture at the window
/// edges, and clamping keeps them from wrapping around to the far side.
fn create_blur_sampler(device: &ID3D11Device) -> Result<Option<ID3D11SamplerState>> {
    unsafe {
        let desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            MipLODBias: 0.0,
            MaxAnisotropy: 1,
            ComparisonFunc: D3D11_COMPARISON_ALWAYS,
            BorderColor: [0.0; 4],
            MinLOD: 0.0,
            MaxLOD: D3D11_FLOAT32_MAX,
        };
        let mut output = None;
        device.CreateSamplerState(&desc, Some(&mut output))?;
        Ok(output)
    }
}

#[derive(Debug, Default)]
#[repr(C)]
struct GlobalParams {
    gamma_ratios: [f32; 4],
    viewport_size: [f32; 2],
    grayscale_enhanced_contrast: f32,
    subpixel_enhanced_contrast: f32,
    is_bgr: u32,
    _pad: [u32; 3],
}

/// Mirrors the `BlurParams` cbuffer (register b1) in `shaders.hlsl`. 80 bytes (a multiple of 16,
/// as constant buffers require). Updated per blur pass via `update_buffer`.
#[repr(C)]
#[derive(Clone, Copy)]
struct BlurParams {
    /// The composite quad (composite pass), or the element's box that the source samples are
    /// mirrored into (downsample pass), in device pixels.
    bounds: Bounds<ScaledPixels>,
    content_mask: Bounds<ScaledPixels>,
    /// The content mask's corner radii (tl, tr, br, bl), in device pixels. The clip an element is
    /// painted under may itself be rounded (`overflow_hidden` + a corner radius), and a filter's
    /// output is clipped by it like any other painting — corners included.
    content_mask_radii: [f32; 4],
    corner_radii: [f32; 4],
    direction: [f32; 2],
    sigma: f32,
    opacity: f32,
    tap_count: f32,
    /// 1.0 clips the composite to the rounded rect (backdrop); 0.0 lets content blur spread past
    /// its bounds (the blur itself no longer fades at the edge — see `mirror_into_rect`).
    clip_rounded: f32,
    /// 1.0 = snapped 2:1 box downsample (anchor the half-res grid to a fixed 2px grid at the
    /// origin, so a stationary element blurs identically at every window size); 0.0 = 1:1 copy
    /// (the scene blit, which must not downsample). Downsample pass only.
    downsample: f32,
    /// Spacing between taps in pixels (gaussian passes only); >1 lets `tap_count` taps span very
    /// large radii without truncating the gaussian, matching the wgpu backend.
    tap_step: f32,
}

impl Default for BlurParams {
    fn default() -> Self {
        BlurParams {
            bounds: Bounds::default(),
            content_mask: Bounds::default(),
            content_mask_radii: [0.0; 4],
            corner_radii: [0.0; 4],
            direction: [0.0, 0.0],
            sigma: 0.0,
            opacity: 1.0,
            tap_count: 0.0,
            clip_rounded: 0.0,
            downsample: 0.0,
            tap_step: 0.0,
        }
    }
}

struct PipelineState<T> {
    label: &'static str,
    vertex: ID3D11VertexShader,
    fragment: ID3D11PixelShader,
    buffer: ID3D11Buffer,
    buffer_size: usize,
    view: Option<ID3D11ShaderResourceView>,
    blend_state: ID3D11BlendState,
    _marker: std::marker::PhantomData<T>,
}

impl<T> PipelineState<T> {
    fn new(
        device: &ID3D11Device,
        label: &'static str,
        shader_module: ShaderModule,
        buffer_size: usize,
        blend_state: ID3D11BlendState,
    ) -> Result<Self> {
        let vertex = {
            let raw_shader = RawShaderBytes::new(shader_module, ShaderTarget::Vertex)?;
            create_vertex_shader(device, raw_shader.as_bytes())?
        };
        let fragment = {
            let raw_shader = RawShaderBytes::new(shader_module, ShaderTarget::Fragment)?;
            create_fragment_shader(device, raw_shader.as_bytes())?
        };
        let buffer = create_buffer(device, std::mem::size_of::<T>(), buffer_size)?;
        let view = create_buffer_view(device, &buffer)?;

        Ok(PipelineState {
            label,
            vertex,
            fragment,
            buffer,
            buffer_size,
            view,
            blend_state,
            _marker: std::marker::PhantomData,
        })
    }

    fn update_buffer(
        &mut self,
        device: &ID3D11Device,
        device_context: &ID3D11DeviceContext,
        data: &[T],
    ) -> Result<()> {
        if self.buffer_size < data.len() {
            let new_buffer_size = data.len().next_power_of_two();
            log::debug!(
                "Updating {} buffer size from {} to {}",
                self.label,
                self.buffer_size,
                new_buffer_size
            );
            let buffer = create_buffer(device, std::mem::size_of::<T>(), new_buffer_size)?;
            let view = create_buffer_view(device, &buffer)?;
            self.buffer = buffer;
            self.view = view;
            self.buffer_size = new_buffer_size;
        }
        update_buffer(device_context, &self.buffer, data)
    }

    fn draw(
        &self,
        device_context: &ID3D11DeviceContext,
        viewport: &[D3D11_VIEWPORT],
        global_params: &[Option<ID3D11Buffer>],
        topology: D3D_PRIMITIVE_TOPOLOGY,
        vertex_count: u32,
        instance_count: u32,
    ) -> Result<()> {
        set_pipeline_state(
            device_context,
            slice::from_ref(&self.view),
            topology,
            viewport,
            &self.vertex,
            &self.fragment,
            global_params,
            &self.blend_state,
        );
        unsafe {
            device_context.DrawInstanced(vertex_count, instance_count, 0, 0);
        }
        Ok(())
    }

    fn draw_with_texture(
        &self,
        device_context: &ID3D11DeviceContext,
        texture: &[Option<ID3D11ShaderResourceView>],
        viewport: &[D3D11_VIEWPORT],
        global_params: &[Option<ID3D11Buffer>],
        sampler: &[Option<ID3D11SamplerState>],
        instance_count: u32,
    ) -> Result<()> {
        set_pipeline_state(
            device_context,
            slice::from_ref(&self.view),
            D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
            viewport,
            &self.vertex,
            &self.fragment,
            global_params,
            &self.blend_state,
        );
        unsafe {
            device_context.PSSetSamplers(0, Some(sampler));
            device_context.VSSetShaderResources(0, Some(texture));
            device_context.PSSetShaderResources(0, Some(texture));

            device_context.DrawInstanced(4, instance_count, 0, 0);
        }
        Ok(())
    }

    fn draw_range(
        &self,
        device: &ID3D11Device,
        device_context: &ID3D11DeviceContext,
        viewport: &[D3D11_VIEWPORT],
        global_params: &[Option<ID3D11Buffer>],
        vertex_count: u32,
        first_instance: u32,
        instance_count: u32,
    ) -> Result<()> {
        let view = create_buffer_view_range(device, &self.buffer, first_instance, instance_count)?;
        set_pipeline_state(
            device_context,
            slice::from_ref(&view),
            D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
            viewport,
            &self.vertex,
            &self.fragment,
            global_params,
            &self.blend_state,
        );
        unsafe {
            device_context.DrawInstanced(vertex_count, instance_count, 0, 0);
        }
        Ok(())
    }

    fn draw_range_with_texture(
        &self,
        device: &ID3D11Device,
        device_context: &ID3D11DeviceContext,
        texture: &[Option<ID3D11ShaderResourceView>],
        viewport: &[D3D11_VIEWPORT],
        global_params: &[Option<ID3D11Buffer>],
        sampler: &[Option<ID3D11SamplerState>],
        first_instance: u32,
        instance_count: u32,
    ) -> Result<()> {
        let view = create_buffer_view_range(device, &self.buffer, first_instance, instance_count)?;
        set_pipeline_state(
            device_context,
            slice::from_ref(&view),
            D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
            viewport,
            &self.vertex,
            &self.fragment,
            global_params,
            &self.blend_state,
        );
        unsafe {
            device_context.PSSetSamplers(0, Some(sampler));
            device_context.VSSetShaderResources(0, Some(texture));
            device_context.PSSetShaderResources(0, Some(texture));
            device_context.DrawInstanced(4, instance_count, 0, 0);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
struct PathRasterizationSprite {
    xy_position: Point<ScaledPixels>,
    st_position: Point<f32>,
    color: Background,
    bounds: Bounds<ScaledPixels>,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct PathSprite {
    bounds: Bounds<ScaledPixels>,
}

impl Drop for DirectXRenderer {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        if let Some(devices) = &self.devices {
            report_live_objects(&devices.device).ok();
        }
    }
}

#[inline]
fn get_comp_device(dxgi_device: &IDXGIDevice) -> Result<IDCompositionDevice> {
    Ok(unsafe { DCompositionCreateDevice(dxgi_device)? })
}

fn create_swap_chain_for_composition(
    dxgi_factory: &IDXGIFactory6,
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<IDXGISwapChain1> {
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: width,
        Height: height,
        Format: RENDER_TARGET_FORMAT,
        Stereo: false.into(),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: BUFFER_COUNT as u32,
        // Composition SwapChains only support the DXGI_SCALING_STRETCH Scaling.
        Scaling: DXGI_SCALING_STRETCH,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
        AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
        Flags: 0,
    };
    Ok(unsafe { dxgi_factory.CreateSwapChainForComposition(device, &desc, None)? })
}

fn create_swap_chain(
    dxgi_factory: &IDXGIFactory6,
    device: &ID3D11Device,
    hwnd: HWND,
    width: u32,
    height: u32,
) -> Result<IDXGISwapChain1> {
    use windows::Win32::Graphics::Dxgi::DXGI_MWA_NO_ALT_ENTER;

    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: width,
        Height: height,
        Format: RENDER_TARGET_FORMAT,
        Stereo: false.into(),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: BUFFER_COUNT as u32,
        Scaling: DXGI_SCALING_NONE,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
        AlphaMode: DXGI_ALPHA_MODE_IGNORE,
        Flags: 0,
    };
    let swap_chain =
        unsafe { dxgi_factory.CreateSwapChainForHwnd(device, hwnd, &desc, None, None) }?;
    unsafe { dxgi_factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER) }?;
    Ok(swap_chain)
}

#[inline]
fn create_resources(
    devices: &DirectXRendererDevices,
    swap_chain: &IDXGISwapChain1,
    width: u32,
    height: u32,
) -> Result<(
    ID3D11Texture2D,
    Option<ID3D11RenderTargetView>,
    ID3D11Texture2D,
    Option<ID3D11ShaderResourceView>,
    ID3D11Texture2D,
    Option<ID3D11RenderTargetView>,
    D3D11_VIEWPORT,
)> {
    let (render_target, render_target_view) =
        create_render_target_and_its_view(swap_chain, &devices.device)?;
    let (path_intermediate_texture, path_intermediate_srv) =
        create_path_intermediate_texture(&devices.device, width, height)?;
    let (path_intermediate_msaa_texture, path_intermediate_msaa_view) =
        create_path_intermediate_msaa_texture_and_view(&devices.device, width, height)?;
    let viewport = set_viewport(&devices.device_context, width as f32, height as f32);
    Ok((
        render_target,
        render_target_view,
        path_intermediate_texture,
        path_intermediate_srv,
        path_intermediate_msaa_texture,
        path_intermediate_msaa_view,
        viewport,
    ))
}

#[inline]
/// Flatten a `Corners` into the `[tl, tr, br, bl]` order expected by the blur composite shader.
fn corner_radii_array(corners: Corners<ScaledPixels>) -> [f32; 4] {
    [
        corners.top_left.0,
        corners.top_right.0,
        corners.bottom_right.0,
        corners.bottom_left.0,
    ]
}

fn create_render_target_and_its_view(
    swap_chain: &IDXGISwapChain1,
    device: &ID3D11Device,
) -> Result<(ID3D11Texture2D, Option<ID3D11RenderTargetView>)> {
    let render_target: ID3D11Texture2D = unsafe { swap_chain.GetBuffer(0) }?;
    let mut render_target_view = None;
    unsafe { device.CreateRenderTargetView(&render_target, None, Some(&mut render_target_view))? };
    Ok((render_target, render_target_view))
}

#[inline]
fn create_path_intermediate_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<(ID3D11Texture2D, Option<ID3D11ShaderResourceView>)> {
    let texture = unsafe {
        let mut output = None;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: RENDER_TARGET_FORMAT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        device.CreateTexture2D(&desc, None, Some(&mut output))?;
        output.unwrap()
    };

    let mut shader_resource_view = None;
    unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut shader_resource_view))? };

    Ok((texture, Some(shader_resource_view.unwrap())))
}

/// Create a color texture usable as both a render target and a shader resource, returning both
/// views. Used for the blur offscreen targets.
#[inline]
fn create_color_target(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<(
    ID3D11Texture2D,
    Option<ID3D11RenderTargetView>,
    Option<ID3D11ShaderResourceView>,
)> {
    let texture = unsafe {
        let mut output = None;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width.max(1),
            Height: height.max(1),
            MipLevels: 1,
            ArraySize: 1,
            Format: RENDER_TARGET_FORMAT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        device.CreateTexture2D(&desc, None, Some(&mut output))?;
        output.unwrap()
    };
    let mut rtv = None;
    unsafe { device.CreateRenderTargetView(&texture, None, Some(&mut rtv))? };
    let mut srv = None;
    unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut srv))? };
    Ok((texture, rtv, srv))
}

#[inline]
fn create_path_intermediate_msaa_texture_and_view(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<(ID3D11Texture2D, Option<ID3D11RenderTargetView>)> {
    let msaa_texture = unsafe {
        let mut output = None;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: RENDER_TARGET_FORMAT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: PATH_MULTISAMPLE_COUNT,
                Quality: D3D11_STANDARD_MULTISAMPLE_PATTERN.0 as u32,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        device.CreateTexture2D(&desc, None, Some(&mut output))?;
        output.unwrap()
    };
    let mut msaa_view = None;
    unsafe { device.CreateRenderTargetView(&msaa_texture, None, Some(&mut msaa_view))? };
    Ok((msaa_texture, Some(msaa_view.unwrap())))
}

#[inline]
fn set_viewport(device_context: &ID3D11DeviceContext, width: f32, height: f32) -> D3D11_VIEWPORT {
    let viewport = [D3D11_VIEWPORT {
        TopLeftX: 0.0,
        TopLeftY: 0.0,
        Width: width,
        Height: height,
        MinDepth: 0.0,
        MaxDepth: 1.0,
    }];
    unsafe { device_context.RSSetViewports(Some(&viewport)) };
    viewport[0]
}

#[inline]
fn set_rasterizer_state(device: &ID3D11Device, device_context: &ID3D11DeviceContext) -> Result<()> {
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_NONE,
        FrontCounterClockwise: false.into(),
        DepthBias: 0,
        DepthBiasClamp: 0.0,
        SlopeScaledDepthBias: 0.0,
        DepthClipEnable: true.into(),
        ScissorEnable: false.into(),
        MultisampleEnable: true.into(),
        AntialiasedLineEnable: false.into(),
    };
    let rasterizer_state = unsafe {
        let mut state = None;
        device.CreateRasterizerState(&desc, Some(&mut state))?;
        state.unwrap()
    };
    unsafe { device_context.RSSetState(&rasterizer_state) };
    Ok(())
}

// https://learn.microsoft.com/en-us/windows/win32/api/d3d11/ns-d3d11-d3d11_blend_desc
#[inline]
fn create_blend_state(device: &ID3D11Device) -> Result<ID3D11BlendState> {
    let mut desc = D3D11_BLEND_DESC::default();
    desc.RenderTarget[0].BlendEnable = true.into();
    desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].SrcBlend = D3D11_BLEND_SRC_ALPHA;
    desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
    desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8;
    unsafe {
        let mut state = None;
        device.CreateBlendState(&desc, Some(&mut state))?;
        Ok(state.unwrap())
    }
}

#[inline]
fn create_blend_state_for_subpixel_rendering(device: &ID3D11Device) -> Result<ID3D11BlendState> {
    let mut desc = D3D11_BLEND_DESC::default();
    desc.RenderTarget[0].BlendEnable = true.into();
    desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].SrcBlend = D3D11_BLEND_SRC1_COLOR;
    desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC1_COLOR;
    // It does not make sense to draw transparent subpixel-rendered text, since it cannot be meaningfully alpha-blended onto anything else.
    desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_ZERO;
    desc.RenderTarget[0].RenderTargetWriteMask =
        D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8 & !D3D11_COLOR_WRITE_ENABLE_ALPHA.0 as u8;

    unsafe {
        let mut state = None;
        device.CreateBlendState(&desc, Some(&mut state))?;
        Ok(state.unwrap())
    }
}

#[inline]
fn create_blend_state_for_path_rasterization(device: &ID3D11Device) -> Result<ID3D11BlendState> {
    // If the feature level is set to greater than D3D_FEATURE_LEVEL_9_3, the display
    // device performs the blend in linear space, which is ideal.
    let mut desc = D3D11_BLEND_DESC::default();
    desc.RenderTarget[0].BlendEnable = true.into();
    desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].SrcBlend = D3D11_BLEND_ONE;
    desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
    desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_INV_SRC_ALPHA;
    desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8;
    unsafe {
        let mut state = None;
        device.CreateBlendState(&desc, Some(&mut state))?;
        Ok(state.unwrap())
    }
}

#[inline]
fn create_blend_state_for_path_sprite(device: &ID3D11Device) -> Result<ID3D11BlendState> {
    // If the feature level is set to greater than D3D_FEATURE_LEVEL_9_3, the display
    // device performs the blend in linear space, which is ideal.
    let mut desc = D3D11_BLEND_DESC::default();
    desc.RenderTarget[0].BlendEnable = true.into();
    desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].SrcBlend = D3D11_BLEND_ONE;
    desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
    desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8;
    unsafe {
        let mut state = None;
        device.CreateBlendState(&desc, Some(&mut state))?;
        Ok(state.unwrap())
    }
}

/// Create a CPU-writable dynamic constant buffer of the given byte size (rounded up to 16).
#[inline]
fn create_constant_buffer(device: &ID3D11Device, byte_size: usize) -> Result<ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: byte_size.next_multiple_of(16) as u32,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        ..Default::default()
    };
    let mut buffer = None;
    unsafe { device.CreateBuffer(&desc, None, Some(&mut buffer)) }?;
    Ok(buffer.unwrap())
}

/// A blend state that overwrites the target (no blending) — used for the blur downsample and
/// gaussian passes.
#[inline]
fn create_blend_state_no_blend(device: &ID3D11Device) -> Result<ID3D11BlendState> {
    let mut desc = D3D11_BLEND_DESC::default();
    desc.RenderTarget[0].BlendEnable = false.into();
    desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8;
    unsafe {
        let mut state = None;
        device.CreateBlendState(&desc, Some(&mut state))?;
        Ok(state.unwrap())
    }
}

#[inline]
fn create_vertex_shader(device: &ID3D11Device, bytes: &[u8]) -> Result<ID3D11VertexShader> {
    unsafe {
        let mut shader = None;
        device.CreateVertexShader(bytes, None, Some(&mut shader))?;
        Ok(shader.unwrap())
    }
}

#[inline]
fn create_fragment_shader(device: &ID3D11Device, bytes: &[u8]) -> Result<ID3D11PixelShader> {
    unsafe {
        let mut shader = None;
        device.CreatePixelShader(bytes, None, Some(&mut shader))?;
        Ok(shader.unwrap())
    }
}

#[inline]
fn create_buffer(
    device: &ID3D11Device,
    element_size: usize,
    buffer_size: usize,
) -> Result<ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: (element_size * buffer_size) as u32,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: D3D11_RESOURCE_MISC_BUFFER_STRUCTURED.0 as u32,
        StructureByteStride: element_size as u32,
    };
    let mut buffer = None;
    unsafe { device.CreateBuffer(&desc, None, Some(&mut buffer)) }?;
    Ok(buffer.unwrap())
}

#[inline]
fn create_buffer_view(
    device: &ID3D11Device,
    buffer: &ID3D11Buffer,
) -> Result<Option<ID3D11ShaderResourceView>> {
    let mut view = None;
    unsafe { device.CreateShaderResourceView(buffer, None, Some(&mut view)) }?;
    Ok(view)
}

#[inline]
fn create_buffer_view_range(
    device: &ID3D11Device,
    buffer: &ID3D11Buffer,
    first_element: u32,
    num_elements: u32,
) -> Result<Option<ID3D11ShaderResourceView>> {
    let desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
        Format: DXGI_FORMAT_UNKNOWN,
        ViewDimension: D3D11_SRV_DIMENSION_BUFFER,
        Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
            Buffer: D3D11_BUFFER_SRV {
                Anonymous1: D3D11_BUFFER_SRV_0 {
                    FirstElement: first_element,
                },
                Anonymous2: D3D11_BUFFER_SRV_1 {
                    NumElements: num_elements,
                },
            },
        },
    };
    let mut view = None;
    unsafe { device.CreateShaderResourceView(buffer, Some(&desc), Some(&mut view)) }?;
    Ok(view)
}

#[inline]
fn update_buffer<T>(
    device_context: &ID3D11DeviceContext,
    buffer: &ID3D11Buffer,
    data: &[T],
) -> Result<()> {
    unsafe {
        let mut dest = std::mem::zeroed();
        device_context.Map(buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut dest))?;
        std::ptr::copy_nonoverlapping(data.as_ptr(), dest.pData as _, data.len());
        device_context.Unmap(buffer, 0);
    }
    Ok(())
}

#[inline]
fn set_pipeline_state(
    device_context: &ID3D11DeviceContext,
    buffer_view: &[Option<ID3D11ShaderResourceView>],
    topology: D3D_PRIMITIVE_TOPOLOGY,
    viewport: &[D3D11_VIEWPORT],
    vertex_shader: &ID3D11VertexShader,
    fragment_shader: &ID3D11PixelShader,
    global_params: &[Option<ID3D11Buffer>],
    blend_state: &ID3D11BlendState,
) {
    unsafe {
        device_context.VSSetShaderResources(1, Some(buffer_view));
        device_context.PSSetShaderResources(1, Some(buffer_view));
        device_context.IASetPrimitiveTopology(topology);
        device_context.RSSetViewports(Some(viewport));
        device_context.VSSetShader(vertex_shader, None);
        device_context.PSSetShader(fragment_shader, None);
        device_context.VSSetConstantBuffers(0, Some(global_params));
        device_context.PSSetConstantBuffers(0, Some(global_params));
        device_context.OMSetBlendState(blend_state, None, 0xFFFFFFFF);
    }
}

#[cfg(debug_assertions)]
fn report_live_objects(device: &ID3D11Device) -> Result<()> {
    let debug_device: ID3D11Debug = device.cast()?;
    unsafe {
        debug_device.ReportLiveDeviceObjects(D3D11_RLDO_DETAIL)?;
    }
    Ok(())
}

const BUFFER_COUNT: usize = 3;

pub(crate) mod shader_resources {
    use anyhow::Result;

    #[cfg(debug_assertions)]
    use windows::{
        Win32::Graphics::Direct3D::{
            Fxc::{D3DCOMPILE_DEBUG, D3DCOMPILE_SKIP_OPTIMIZATION, D3DCompileFromFile},
            ID3DBlob,
        },
        core::{HSTRING, PCSTR},
    };

    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub(crate) enum ShaderModule {
        Quad,
        Shadow,
        Underline,
        PathRasterization,
        PathSprite,
        MonochromeSprite,
        SubpixelSprite,
        PolychromeSprite,
        EmojiRasterization,
        BlurDownsample,
        Blur,
        BlurComposite,
    }

    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub(crate) enum ShaderTarget {
        Vertex,
        Fragment,
    }

    pub(crate) struct RawShaderBytes<'t> {
        inner: &'t [u8],

        #[cfg(debug_assertions)]
        _blob: ID3DBlob,
    }

    impl<'t> RawShaderBytes<'t> {
        pub(crate) fn new(module: ShaderModule, target: ShaderTarget) -> Result<Self> {
            #[cfg(not(debug_assertions))]
            {
                Ok(Self::from_bytes(module, target))
            }
            #[cfg(debug_assertions)]
            {
                let blob = build_shader_blob(module, target)?;
                let inner = unsafe {
                    std::slice::from_raw_parts(
                        blob.GetBufferPointer() as *const u8,
                        blob.GetBufferSize(),
                    )
                };
                Ok(Self { inner, _blob: blob })
            }
        }

        pub(crate) fn as_bytes(&'t self) -> &'t [u8] {
            self.inner
        }

        #[cfg(not(debug_assertions))]
        fn from_bytes(module: ShaderModule, target: ShaderTarget) -> Self {
            let bytes = match module {
                ShaderModule::Quad => match target {
                    ShaderTarget::Vertex => QUAD_VERTEX_BYTES,
                    ShaderTarget::Fragment => QUAD_FRAGMENT_BYTES,
                },
                ShaderModule::Shadow => match target {
                    ShaderTarget::Vertex => SHADOW_VERTEX_BYTES,
                    ShaderTarget::Fragment => SHADOW_FRAGMENT_BYTES,
                },
                ShaderModule::Underline => match target {
                    ShaderTarget::Vertex => UNDERLINE_VERTEX_BYTES,
                    ShaderTarget::Fragment => UNDERLINE_FRAGMENT_BYTES,
                },
                ShaderModule::PathRasterization => match target {
                    ShaderTarget::Vertex => PATH_RASTERIZATION_VERTEX_BYTES,
                    ShaderTarget::Fragment => PATH_RASTERIZATION_FRAGMENT_BYTES,
                },
                ShaderModule::PathSprite => match target {
                    ShaderTarget::Vertex => PATH_SPRITE_VERTEX_BYTES,
                    ShaderTarget::Fragment => PATH_SPRITE_FRAGMENT_BYTES,
                },
                ShaderModule::MonochromeSprite => match target {
                    ShaderTarget::Vertex => MONOCHROME_SPRITE_VERTEX_BYTES,
                    ShaderTarget::Fragment => MONOCHROME_SPRITE_FRAGMENT_BYTES,
                },
                ShaderModule::SubpixelSprite => match target {
                    ShaderTarget::Vertex => SUBPIXEL_SPRITE_VERTEX_BYTES,
                    ShaderTarget::Fragment => SUBPIXEL_SPRITE_FRAGMENT_BYTES,
                },
                ShaderModule::PolychromeSprite => match target {
                    ShaderTarget::Vertex => POLYCHROME_SPRITE_VERTEX_BYTES,
                    ShaderTarget::Fragment => POLYCHROME_SPRITE_FRAGMENT_BYTES,
                },
                ShaderModule::EmojiRasterization => match target {
                    ShaderTarget::Vertex => EMOJI_RASTERIZATION_VERTEX_BYTES,
                    ShaderTarget::Fragment => EMOJI_RASTERIZATION_FRAGMENT_BYTES,
                },
                ShaderModule::BlurDownsample => match target {
                    ShaderTarget::Vertex => BLUR_DOWNSAMPLE_VERTEX_BYTES,
                    ShaderTarget::Fragment => BLUR_DOWNSAMPLE_FRAGMENT_BYTES,
                },
                ShaderModule::Blur => match target {
                    ShaderTarget::Vertex => BLUR_VERTEX_BYTES,
                    ShaderTarget::Fragment => BLUR_FRAGMENT_BYTES,
                },
                ShaderModule::BlurComposite => match target {
                    ShaderTarget::Vertex => BLUR_COMPOSITE_VERTEX_BYTES,
                    ShaderTarget::Fragment => BLUR_COMPOSITE_FRAGMENT_BYTES,
                },
            };
            Self { inner: bytes }
        }
    }

    #[cfg(debug_assertions)]
    pub(super) fn build_shader_blob(entry: ShaderModule, target: ShaderTarget) -> Result<ID3DBlob> {
        unsafe {
            use windows::Win32::Graphics::{
                Direct3D::ID3DInclude, Hlsl::D3D_COMPILE_STANDARD_FILE_INCLUDE,
            };

            let shader_name = if matches!(entry, ShaderModule::EmojiRasterization) {
                "color_text_raster.hlsl"
            } else {
                "shaders.hlsl"
            };

            let entry = format!(
                "{}_{}\0",
                entry.as_str(),
                match target {
                    ShaderTarget::Vertex => "vertex",
                    ShaderTarget::Fragment => "fragment",
                }
            );
            let target = match target {
                ShaderTarget::Vertex => "vs_4_1\0",
                ShaderTarget::Fragment => "ps_4_1\0",
            };

            let mut compile_blob = None;
            let mut error_blob = None;
            let shader_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(&format!("src/{}", shader_name))
                .canonicalize()?;

            let entry_point = PCSTR::from_raw(entry.as_ptr());
            let target_cstr = PCSTR::from_raw(target.as_ptr());

            // really dirty trick because winapi bindings are unhappy otherwise
            let include_handler = &std::mem::transmute::<usize, ID3DInclude>(
                D3D_COMPILE_STANDARD_FILE_INCLUDE as usize,
            );

            let ret = D3DCompileFromFile(
                &HSTRING::from(shader_path.to_str().unwrap()),
                None,
                include_handler,
                entry_point,
                target_cstr,
                D3DCOMPILE_DEBUG | D3DCOMPILE_SKIP_OPTIMIZATION,
                0,
                &mut compile_blob,
                Some(&mut error_blob),
            );
            if ret.is_err() {
                let Some(error_blob) = error_blob else {
                    return Err(anyhow::anyhow!("{ret:?}"));
                };

                let error_string =
                    std::ffi::CStr::from_ptr(error_blob.GetBufferPointer() as *const i8)
                        .to_string_lossy();
                log::error!("Shader compile error: {}", error_string);
                return Err(anyhow::anyhow!("Compile error: {}", error_string));
            }
            Ok(compile_blob.unwrap())
        }
    }

    #[cfg(not(debug_assertions))]
    include!(concat!(env!("OUT_DIR"), "/shaders_bytes.rs"));

    #[cfg(debug_assertions)]
    impl ShaderModule {
        pub fn as_str(self) -> &'static str {
            match self {
                ShaderModule::Quad => "quad",
                ShaderModule::Shadow => "shadow",
                ShaderModule::Underline => "underline",
                ShaderModule::PathRasterization => "path_rasterization",
                ShaderModule::PathSprite => "path_sprite",
                ShaderModule::MonochromeSprite => "monochrome_sprite",
                ShaderModule::SubpixelSprite => "subpixel_sprite",
                ShaderModule::PolychromeSprite => "polychrome_sprite",
                ShaderModule::EmojiRasterization => "emoji_rasterization",
                ShaderModule::BlurDownsample => "blur_downsample",
                ShaderModule::Blur => "blur",
                ShaderModule::BlurComposite => "blur_composite",
            }
        }
    }
}

mod nvidia {
    use std::{
        ffi::CStr,
        os::raw::{c_char, c_int, c_uint},
    };

    use anyhow::Result;
    use windows::{Win32::System::LibraryLoader::GetProcAddress, core::s};

    use crate::with_dll_library;

    // https://github.com/NVIDIA/nvapi/blob/7cb76fce2f52de818b3da497af646af1ec16ce27/nvapi_lite_common.h#L180
    const NVAPI_SHORT_STRING_MAX: usize = 64;

    // https://github.com/NVIDIA/nvapi/blob/7cb76fce2f52de818b3da497af646af1ec16ce27/nvapi_lite_common.h#L235
    #[allow(non_camel_case_types)]
    type NvAPI_ShortString = [c_char; NVAPI_SHORT_STRING_MAX];

    // https://github.com/NVIDIA/nvapi/blob/7cb76fce2f52de818b3da497af646af1ec16ce27/nvapi_lite_common.h#L447
    #[allow(non_camel_case_types)]
    type NvAPI_SYS_GetDriverAndBranchVersion_t = unsafe extern "C" fn(
        driver_version: *mut c_uint,
        build_branch_string: *mut NvAPI_ShortString,
    ) -> c_int;

    pub(super) fn get_driver_version() -> Result<String> {
        #[cfg(target_pointer_width = "64")]
        let nvidia_dll_name = s!("nvapi64.dll");
        #[cfg(target_pointer_width = "32")]
        let nvidia_dll_name = s!("nvapi.dll");

        with_dll_library(nvidia_dll_name, |nvidia_dll| unsafe {
            let nvapi_query_addr = GetProcAddress(nvidia_dll, s!("nvapi_QueryInterface"))
                .ok_or_else(|| anyhow::anyhow!("Failed to get nvapi_QueryInterface address"))?;
            let nvapi_query: extern "C" fn(u32) -> *mut () = std::mem::transmute(nvapi_query_addr);

            // https://github.com/NVIDIA/nvapi/blob/7cb76fce2f52de818b3da497af646af1ec16ce27/nvapi_interface.h#L41
            let nvapi_get_driver_version_ptr = nvapi_query(0x2926aaad);
            if nvapi_get_driver_version_ptr.is_null() {
                anyhow::bail!("Failed to get NVIDIA driver version function pointer");
            }
            let nvapi_get_driver_version: NvAPI_SYS_GetDriverAndBranchVersion_t =
                std::mem::transmute(nvapi_get_driver_version_ptr);

            let mut driver_version: c_uint = 0;
            let mut build_branch_string: NvAPI_ShortString = [0; NVAPI_SHORT_STRING_MAX];
            let result = nvapi_get_driver_version(
                &mut driver_version as *mut c_uint,
                &mut build_branch_string as *mut NvAPI_ShortString,
            );

            if result != 0 {
                anyhow::bail!(
                    "Failed to get NVIDIA driver version, error code: {}",
                    result
                );
            }
            let major = driver_version / 100;
            let minor = driver_version % 100;
            let branch_string = CStr::from_ptr(build_branch_string.as_ptr());
            Ok(format!(
                "{}.{} {}",
                major,
                minor,
                branch_string.to_string_lossy()
            ))
        })
    }
}

mod amd {
    use std::os::raw::{c_char, c_int, c_void};

    use anyhow::Result;
    use windows::{Win32::System::LibraryLoader::GetProcAddress, core::s};

    use crate::with_dll_library;

    // https://github.com/GPUOpen-LibrariesAndSDKs/AGS_SDK/blob/5d8812d703d0335741b6f7ffc37838eeb8b967f7/ags_lib/inc/amd_ags.h#L145
    const AGS_CURRENT_VERSION: i32 = (6 << 22) | (3 << 12);

    // https://github.com/GPUOpen-LibrariesAndSDKs/AGS_SDK/blob/5d8812d703d0335741b6f7ffc37838eeb8b967f7/ags_lib/inc/amd_ags.h#L204
    // This is an opaque type, using struct to represent it properly for FFI
    #[repr(C)]
    struct AGSContext {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct AGSGPUInfo {
        pub driver_version: *const c_char,
        pub radeon_software_version: *const c_char,
        pub num_devices: c_int,
        pub devices: *mut c_void,
    }

    // https://github.com/GPUOpen-LibrariesAndSDKs/AGS_SDK/blob/5d8812d703d0335741b6f7ffc37838eeb8b967f7/ags_lib/inc/amd_ags.h#L429
    #[allow(non_camel_case_types)]
    type agsInitialize_t = unsafe extern "C" fn(
        version: c_int,
        config: *const c_void,
        context: *mut *mut AGSContext,
        gpu_info: *mut AGSGPUInfo,
    ) -> c_int;

    // https://github.com/GPUOpen-LibrariesAndSDKs/AGS_SDK/blob/5d8812d703d0335741b6f7ffc37838eeb8b967f7/ags_lib/inc/amd_ags.h#L436
    #[allow(non_camel_case_types)]
    type agsDeInitialize_t = unsafe extern "C" fn(context: *mut AGSContext) -> c_int;

    pub(super) fn get_driver_version() -> Result<String> {
        #[cfg(target_pointer_width = "64")]
        let amd_dll_name = s!("amd_ags_x64.dll");
        #[cfg(target_pointer_width = "32")]
        let amd_dll_name = s!("amd_ags_x86.dll");

        with_dll_library(amd_dll_name, |amd_dll| unsafe {
            let ags_initialize_addr = GetProcAddress(amd_dll, s!("agsInitialize"))
                .ok_or_else(|| anyhow::anyhow!("Failed to get agsInitialize address"))?;
            let ags_deinitialize_addr = GetProcAddress(amd_dll, s!("agsDeInitialize"))
                .ok_or_else(|| anyhow::anyhow!("Failed to get agsDeInitialize address"))?;

            let ags_initialize: agsInitialize_t = std::mem::transmute(ags_initialize_addr);
            let ags_deinitialize: agsDeInitialize_t = std::mem::transmute(ags_deinitialize_addr);

            let mut context: *mut AGSContext = std::ptr::null_mut();
            let mut gpu_info: AGSGPUInfo = AGSGPUInfo {
                driver_version: std::ptr::null(),
                radeon_software_version: std::ptr::null(),
                num_devices: 0,
                devices: std::ptr::null_mut(),
            };

            let result = ags_initialize(
                AGS_CURRENT_VERSION,
                std::ptr::null(),
                &mut context,
                &mut gpu_info,
            );
            if result != 0 {
                anyhow::bail!("Failed to initialize AMD AGS, error code: {}", result);
            }

            // Vulkan actually returns this as the driver version
            let software_version = if !gpu_info.radeon_software_version.is_null() {
                std::ffi::CStr::from_ptr(gpu_info.radeon_software_version)
                    .to_string_lossy()
                    .into_owned()
            } else {
                "Unknown Radeon Software Version".to_string()
            };

            let driver_version = if !gpu_info.driver_version.is_null() {
                std::ffi::CStr::from_ptr(gpu_info.driver_version)
                    .to_string_lossy()
                    .into_owned()
            } else {
                "Unknown Radeon Driver Version".to_string()
            };

            ags_deinitialize(context);
            Ok(format!("{} ({})", software_version, driver_version))
        })
    }
}

mod dxgi {
    use windows::{
        Win32::Graphics::Dxgi::{IDXGIAdapter1, IDXGIDevice},
        core::Interface,
    };

    pub(super) fn get_driver_version(adapter: &IDXGIAdapter1) -> anyhow::Result<String> {
        let number = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID as _) }?;
        Ok(format!(
            "{}.{}.{}.{}",
            number >> 48,
            (number >> 32) & 0xFFFF,
            (number >> 16) & 0xFFFF,
            number & 0xFFFF
        ))
    }
}

#[cfg(test)]
mod tests {
    // NOTE: do NOT use `use super::*;` here — the top-level module glob-imports
    // `hgpui::*`, and hgpui re-exports a macro named `test` (gpui.rs: `pub use
    // hgpui_macros::{..., test, ...}`). That glob chain shadows the built-in
    // `#[test]` attribute and blows up `#[test]` expansion with an infinite
    // recursion. Import only what this module needs, explicitly.
    use super::{
        create_blend_state, create_blend_state_for_path_sprite, create_blend_state_no_blend,
        corner_radii_array, create_blur_sampler, create_color_target, create_constant_buffer,
        create_fragment_shader, create_vertex_shader, update_buffer, BlurParams, GlobalParams,
        PipelineState,
        RawShaderBytes, ShaderTarget, RENDER_TARGET_FORMAT,
    };
    use crate::directx_renderer::shader_resources::ShaderModule;
    use anyhow::Result;
    use hgpui::{
        bounds, hsla, linear_color_stop, linear_gradient, point, radial_gradient, rgba, size,
        Background, Bounds, ContentMask, Corners, Quad, RadialShape, RadialSize, ScaledPixels,
    };
    use std::slice;
    use windows::core::Interface;
    use windows::Win32::{
        Foundation::HMODULE,
        Graphics::{
            Direct3D::{
                D3D_DRIVER_TYPE_WARP, D3D_PRIMITIVE_TOPOLOGY,
                D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
            },
            Direct3D11::{
                D3D11CreateDevice, D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_RENDER_TARGET,
                D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_CPU_ACCESS_WRITE,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_DEBUG, D3D11_MAP_READ,
                D3D11_MAPPED_SUBRESOURCE, D3D11_MESSAGE, D3D11_SDK_VERSION,
                D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
                D3D11_USAGE_DYNAMIC, D3D11_USAGE_STAGING, D3D11_VIEWPORT, ID3D11BlendState,
                ID3D11Buffer, ID3D11Device, ID3D11DeviceContext, ID3D11InfoQueue,
                ID3D11PixelShader, ID3D11RenderTargetView, ID3D11SamplerState,
                ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
                D3D11_BUFFER_DESC,
            },
            Dxgi::Common::DXGI_SAMPLE_DESC,
        },
    };

    const TEST_SIZE: u32 = 64;

    fn create_warp_device() -> Option<(ID3D11Device, ID3D11DeviceContext)> {
        unsafe {
            let mut device: Option<ID3D11Device> = None;
            let mut device_context: Option<ID3D11DeviceContext> = None;
            // Prefer the debug layer so GPU-side errors (which WARP reports as device
            // removal, 0x887A0005) surface with their real message via the info queue.
            let mut result = D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_DEBUG | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut device_context),
            );
            if result.is_err() {
                // Debug layer not installed (Windows feature "Graphics Tools"); retry
                // without it.
                device = None;
                device_context = None;
                result = D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_WARP,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut device_context),
                );
            }
            result.ok()?;
            Some((device?, device_context?))
        }
    }

    /// Dumps every stored debug-layer message (errors + warnings) to stderr and clears the
    /// queue. WARP + debug layer describe the exact failure that caused a device removal.
    fn dump_device_errors(device: &ID3D11Device) {
        unsafe {
            match device.GetDeviceRemovedReason() {
                Ok(()) => eprintln!("[quad_offscreen] GetDeviceRemovedReason: S_OK"),
                Err(e) => eprintln!(
                    "[quad_offscreen] GetDeviceRemovedReason: {:#010X} ({})",
                    e.code().0,
                    e.message()
                ),
            }
            let Ok(queue) = device.cast::<ID3D11InfoQueue>() else {
                eprintln!("[quad_offscreen] no info queue (debug layer not present)");
                return;
            };
            let count = queue.GetNumStoredMessages();
            if count == 0 {
                eprintln!("[quad_offscreen] info queue empty");
                return;
            }
            for i in 0..count {
                let mut len: usize = 0;
                let _ = queue.GetMessage(i, None, &mut len);
                let mut buf = vec![0u8; len];
                let msg = buf.as_mut_ptr() as *mut D3D11_MESSAGE;
                if queue.GetMessage(i, Some(msg), &mut len).is_ok() {
                    let m = &*msg;
                    let desc =
                        std::ffi::CStr::from_ptr(m.pDescription as *const i8).to_string_lossy();
                    eprintln!(
                        "[quad_offscreen] D3D11 msg {i} (severity {:#X}): {desc}",
                        m.Severity.0
                    );
                }
            }
            queue.ClearStoredMessages();
        }
    }

    fn make_render_target(
        device: &ID3D11Device,
    ) -> Result<(ID3D11Texture2D, ID3D11RenderTargetView, ID3D11Texture2D)> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: TEST_SIZE,
            Height: TEST_SIZE,
            MipLevels: 1,
            ArraySize: 1,
            Format: RENDER_TARGET_FORMAT,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }?;
        let texture = texture.unwrap();

        let mut rtv = None;
        unsafe { device.CreateRenderTargetView(&texture, None, Some(&mut rtv)) }?;
        let rtv = rtv.unwrap();

        let staging_desc = D3D11_TEXTURE2D_DESC {
            Usage: D3D11_USAGE_STAGING,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            BindFlags: 0,
            ..desc
        };
        let mut staging = None;
        unsafe { device.CreateTexture2D(&staging_desc, None, Some(&mut staging)) }?;
        Ok((texture, rtv, staging.unwrap()))
    }

    fn read_pixel(
        context: &ID3D11DeviceContext,
        texture: &ID3D11Texture2D,
        staging: &ID3D11Texture2D,
        x: u32,
        y: u32,
    ) -> Result<[u8; 4]> {
        unsafe {
            // Both parameters are already `&ID3D11Texture2D`; pass them by value
            // (windows 0.62 `Param<T> for &U` — `&` here would be a double reference).
            context.CopyResource(staging, texture);
            context.Flush();
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context
                .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| anyhow::anyhow!("Map(staging) failed: {e}"))?;
            let row = (y as usize) * mapped.RowPitch as usize;
            let px = [
                *(mapped.pData as *const u8).add(row + x as usize * 4),
                *(mapped.pData as *const u8).add(row + x as usize * 4 + 1),
                *(mapped.pData as *const u8).add(row + x as usize * 4 + 2),
                *(mapped.pData as *const u8).add(row + x as usize * 4 + 3),
            ];
            context.Unmap(staging, 0);
            Ok(px)
        }
    }

    /// Renders one solid quad through the real `quad_vertex`/`quad_fragment` shaders onto an
    /// offscreen render target and reads the pixels back. If the quad pipeline fails to rasterize
    /// anything (the "no rectangles" symptom on the DirectX backend), the center pixel assertion
    /// fails here, separating "shader/pipeline broken" from "scene upload broken".
    #[test]
    fn hlsl_quad_offscreen_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[quad_offscreen] WARP device unavailable, skipping");
            return Ok(());
        };
        let (texture, rtv, staging) = make_render_target(&device)?;

        let globals = GlobalParams {
            gamma_ratios: [1.0, 1.0, 1.0, 1.0],
            viewport_size: [TEST_SIZE as f32, TEST_SIZE as f32],
            grayscale_enhanced_contrast: 0.0,
            subpixel_enhanced_contrast: 0.0,
            is_bgr: 0,
            _pad: [0; 3],
        };
        // Must be a real constant buffer: `create_buffer` makes a structured SRV (bound as
        // cbuffer → debug-layer error, device removal on WARP). Mirror the main renderer's
        // `DirectXGlobalElements::new`.
        let globals_buffer = unsafe {
            let desc = D3D11_BUFFER_DESC {
                ByteWidth: std::mem::size_of::<GlobalParams>() as u32,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                ..Default::default()
            };
            let mut buffer = None;
            device.CreateBuffer(&desc, None, Some(&mut buffer))?;
            buffer.unwrap()
        };
        update_buffer(&context, &globals_buffer, &[globals])?;

        let blend = create_blend_state(&device)?;
        let mut pipeline = PipelineState::<Quad>::new(&device, "test_quad", ShaderModule::Quad, 4, blend)?;

        // Solid red quad covering [8,8]-[56,56] of the 64x64 viewport, identity transform.
        let quad = Quad {
            bounds: bounds(
                point(ScaledPixels(8.0), ScaledPixels(8.0)),
                size(ScaledPixels(48.0), ScaledPixels(48.0)),
            ),
            content_mask: ContentMask {
                bounds: bounds(
                    point(ScaledPixels(0.0), ScaledPixels(0.0)),
                    size(ScaledPixels(64.0), ScaledPixels(64.0)),
                ),
                corner_radii: Corners::default(),
            },
            background: Background::from(rgba(0xff0000ff)),
            ..Default::default()
        };
        pipeline.update_buffer(&device, &context, &[quad])?;

        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: TEST_SIZE as f32,
            Height: TEST_SIZE as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        // Opaque purple background so any quad pixel is distinguishable from the clear color.
        // ClearRenderTargetView takes RGBA float order; the B8G8R8A8 target stores memory in
        // B,G,R,A order, so R=0.5 ends up in the B byte and vice versa.
        let bg = [0.25, 0.125, 0.5, 1.0];
        unsafe {
            context.ClearRenderTargetView(&rtv, &bg);
            context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv))), None);
        }
        pipeline.draw_range(&device, &context, &[viewport], &[Some(globals_buffer)], 4, 0, 1)?;
        // If the draw killed the device (the "no rectangles" symptom), the debug layer
        // tells us exactly why — dump before touching the render target again.
        dump_device_errors(&device);

        let center = read_pixel(&context, &texture, &staging, 32, 32)
            .expect("readback of center pixel failed");
        assert_eq!(
            center,
            [0, 0, 255, 255],
            "center pixel should be solid red (B,G,R,A), got {center:?}"
        );
        let corner = read_pixel(&context, &texture, &staging, 2, 2)
            .expect("readback of corner pixel failed");
        assert_eq!(
            corner,
            [128, 32, 64, 255],
            "corner pixel should be the clear color (B,G,R,A), got {corner:?}"
        );
        Ok(())
    }

    /// A `TEST_SIZE`-square texture holding an opaque `color` square on a transparent surround:
    /// the case of a photo on a page.
    fn create_source_square(
        device: &ID3D11Device,
        square: (u32, u32, u32, u32),
        color: [u8; 4],
    ) -> Result<ID3D11Texture2D> {
        let mut pixels = vec![0u8; (TEST_SIZE * TEST_SIZE * 4) as usize];
        for y in square.1..square.3 {
            for x in square.0..square.2 {
                let i = ((y * TEST_SIZE + x) * 4) as usize;
                pixels[i..i + 4].copy_from_slice(&color);
            }
        }
        unsafe {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: TEST_SIZE,
                Height: TEST_SIZE,
                MipLevels: 1,
                ArraySize: 1,
                Format: RENDER_TARGET_FORMAT,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let data = D3D11_SUBRESOURCE_DATA {
                pSysMem: pixels.as_ptr().cast(),
                SysMemPitch: TEST_SIZE * 4,
                SysMemSlicePitch: 0,
            };
            let mut texture = None;
            device.CreateTexture2D(&desc, Some(&data), Some(&mut texture))?;
            Ok(texture.unwrap())
        }
    }

    /// One blur pass: binds everything the pass needs and draws a fullscreen triangle (or the
    /// composite's quad) with `params` in the b1 constant buffer, exactly as `dx_blur_pass` does.
    #[allow(clippy::too_many_arguments)]
    fn draw_blur_pass(
        context: &ID3D11DeviceContext,
        device: &ID3D11Device,
        vertex: &ID3D11VertexShader,
        fragment: &ID3D11PixelShader,
        blend: &ID3D11BlendState,
        target: &ID3D11RenderTargetView,
        source: &ID3D11ShaderResourceView,
        sampler: &ID3D11SamplerState,
        globals: &ID3D11Buffer,
        params_buffer: &ID3D11Buffer,
        params: BlurParams,
        viewport: &D3D11_VIEWPORT,
        topology: D3D_PRIMITIVE_TOPOLOGY,
        vertex_count: u32,
    ) -> Result<()> {
        let _ = device;
        update_buffer(context, params_buffer, &[params])?;
        let srv = Some(source.clone());
        let rtv = Some(target.clone());
        let globals = Some(globals.clone());
        let params_buffer = Some(params_buffer.clone());
        let sampler = Some(sampler.clone());
        unsafe {
            // Unbind slot 0 before setting the target: a texture that is still bound as output
            // (the target of the previous pass) cannot be bound as input at the same time.
            let null_srv: [Option<ID3D11ShaderResourceView>; 1] = [None];
            context.PSSetShaderResources(0, Some(&null_srv));
            context.OMSetRenderTargets(Some(slice::from_ref(&rtv)), None);
            context.RSSetViewports(Some(slice::from_ref(viewport)));
            context.IASetPrimitiveTopology(topology);
            context.VSSetShader(vertex, None);
            context.PSSetShader(fragment, None);
            context.VSSetConstantBuffers(0, Some(slice::from_ref(&globals)));
            context.PSSetConstantBuffers(0, Some(slice::from_ref(&globals)));
            context.VSSetConstantBuffers(1, Some(slice::from_ref(&params_buffer)));
            context.PSSetConstantBuffers(1, Some(slice::from_ref(&params_buffer)));
            context.PSSetSamplers(0, Some(slice::from_ref(&sampler)));
            context.OMSetBlendState(blend, None, 0xFFFFFFFF);
            context.PSSetShaderResources(0, Some(slice::from_ref(&srv)));
            context.DrawInstanced(vertex_count, 1, 0, 0);
            context.PSSetShaderResources(0, Some(&null_srv));
        }
        Ok(())
    }

    /// A blurred element must not dissolve at its own border. The downsample reflects taps that
    /// leave the element's box back inside it, so the blur reads the element's own edge pixels
    /// instead of the transparent surround; without that reflection the border fades out, and on
    /// a light page that reads as a white rim.
    ///
    /// This runs the real four passes twice — once with the element's box, once with the zero box
    /// the pass used to be handed — and reads the pixels back.
    #[test]
    fn hlsl_blur_edge_offscreen_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[blur_edge] WARP device unavailable, skipping");
            return Ok(());
        };

        /// The opaque square inside the source texture, and the element's box it stands for.
        const SQUARE: (u32, u32, u32, u32) = (16, 16, 48, 48);
        const RADIUS: f32 = 12.0;
        /// Solid red, fully opaque — B, G, R, A, the order a BGRA target stores.
        const RED: [u8; 4] = [0, 0, 255, 255];

        let half = TEST_SIZE / 2;
        // The blur passes sample the source as a shader resource; it is filled from the CPU (an
        // empty `create_color_target` texture would be transparent everywhere).
        let source_srv = {
            let square = create_source_square(&device, SQUARE, RED)?;
            let mut srv = None;
            unsafe { device.CreateShaderResourceView(&square, None, Some(&mut srv)) }?;
            srv.unwrap()
        };
        let (ping_texture, ping_rtv, ping_srv) = create_color_target(&device, half, half)?;
        let (pong_texture, pong_rtv, pong_srv) = create_color_target(&device, half, half)?;
        let (output, output_rtv, _) = create_color_target(&device, TEST_SIZE, TEST_SIZE)?;
        let staging = {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: TEST_SIZE,
                Height: TEST_SIZE,
                MipLevels: 1,
                ArraySize: 1,
                Format: RENDER_TARGET_FORMAT,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut staging = None;
            unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging)) }?;
            staging.unwrap()
        };
        let _ = &ping_texture;
        let _ = &pong_texture;

        let globals = GlobalParams {
            gamma_ratios: [1.0, 1.0, 1.0, 1.0],
            viewport_size: [TEST_SIZE as f32, TEST_SIZE as f32],
            grayscale_enhanced_contrast: 0.0,
            subpixel_enhanced_contrast: 0.0,
            is_bgr: 0,
            _pad: [0; 3],
        };
        let globals_buffer = create_constant_buffer(&device, std::mem::size_of::<GlobalParams>())?;
        update_buffer(&context, &globals_buffer, &[globals])?;
        let params_buffer = create_constant_buffer(&device, std::mem::size_of::<BlurParams>())?;
        let sampler = create_blur_sampler(&device)?.expect("blur sampler");
        let replace = create_blend_state_no_blend(&device)?;
        let composite_blend = create_blend_state_for_path_sprite(&device)?;
        let downsample_vertex = create_vertex_shader(
            &device,
            RawShaderBytes::new(ShaderModule::BlurDownsample, ShaderTarget::Vertex)?.as_bytes(),
        )?;
        let downsample_fragment = create_fragment_shader(
            &device,
            RawShaderBytes::new(ShaderModule::BlurDownsample, ShaderTarget::Fragment)?.as_bytes(),
        )?;
        let blur_vertex = create_vertex_shader(
            &device,
            RawShaderBytes::new(ShaderModule::Blur, ShaderTarget::Vertex)?.as_bytes(),
        )?;
        let blur_fragment = create_fragment_shader(
            &device,
            RawShaderBytes::new(ShaderModule::Blur, ShaderTarget::Fragment)?.as_bytes(),
        )?;
        let composite_vertex = create_vertex_shader(
            &device,
            RawShaderBytes::new(ShaderModule::BlurComposite, ShaderTarget::Vertex)?.as_bytes(),
        )?;
        let composite_fragment = create_fragment_shader(
            &device,
            RawShaderBytes::new(ShaderModule::BlurComposite, ShaderTarget::Fragment)?.as_bytes(),
        )?;

        let viewport = |size: u32| D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: size as f32,
            Height: size as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let half_viewport = viewport(half);
        let full_viewport = viewport(TEST_SIZE);

        // The same kernel the renderer derives: sigma is halved because the blur runs at half
        // resolution.
        let sigma = RADIUS * 0.5;
        let ideal_taps = (3.0 * sigma).ceil();
        let tap_count = ideal_taps.clamp(1.0, 32.0);
        let tap_step = (ideal_taps / tap_count).max(1.0);

        let element_bounds = bounds(
            point(ScaledPixels(SQUARE.0 as f32), ScaledPixels(SQUARE.1 as f32)),
            size(
                ScaledPixels((SQUARE.2 - SQUARE.0) as f32),
                ScaledPixels((SQUARE.3 - SQUARE.1) as f32),
            ),
        );

        let run = |mirror_into: Bounds<ScaledPixels>,
                   content_mask: ContentMask<ScaledPixels>|
         -> Result<()> {
            let rtv = output_rtv.clone().expect("output rtv");
            let srv = &source_srv;
            // The composite blends into the target, so each run starts from transparent black.
            unsafe { context.ClearRenderTargetView(&rtv, &[0.0; 4]) };

            // 1. Downsample the source into the half-res ping, reflecting taps that leave the
            //    element's box back inside it.
            draw_blur_pass(
                &context,
                &device,
                &downsample_vertex,
                &downsample_fragment,
                &replace,
                ping_rtv.as_ref().expect("ping rtv"),
                srv,
                &sampler,
                &globals_buffer,
                &params_buffer,
                BlurParams {
                    downsample: 1.0,
                    bounds: mirror_into,
                    ..Default::default()
                },
                &half_viewport,
                D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
                3,
            )?;
            // 2./3. One axis each of the separable gaussian, ping -> pong -> ping.
            for direction in [[1.0 / half as f32, 0.0], [0.0, 1.0 / half as f32]] {
                let (target, source) = if direction[0] > 0.0 {
                    (pong_rtv.as_ref(), ping_srv.as_ref())
                } else {
                    (ping_rtv.as_ref(), pong_srv.as_ref())
                };
                draw_blur_pass(
                    &context,
                    &device,
                    &blur_vertex,
                    &blur_fragment,
                    &replace,
                    target.expect("blur target"),
                    source.expect("blur source"),
                    &sampler,
                    &globals_buffer,
                    &params_buffer,
                    BlurParams {
                        direction,
                        sigma,
                        tap_count,
                        tap_step,
                        ..Default::default()
                    },
                    &half_viewport,
                    D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
                    3,
                )?;
            }
            // 4. Composite the blurred result over the target, dilated like the content path.
            let composite_bounds = element_bounds.dilate(ScaledPixels(3.0 * RADIUS));
            draw_blur_pass(
                &context,
                &device,
                &composite_vertex,
                &composite_fragment,
                &composite_blend,
                &rtv,
                ping_srv.as_ref().expect("ping srv"),
                &sampler,
                &globals_buffer,
                &params_buffer,
                BlurParams {
                    bounds: composite_bounds,
                    content_mask: content_mask.bounds,
                    content_mask_radii: corner_radii_array(content_mask.corner_radii),
                    opacity: 1.0,
                    clip_rounded: 0.0,
                    ..Default::default()
                },
                &full_viewport,
                D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
                4,
            )?;
            Ok(())
        };

        let close =
            |px: [u8; 4], expected: [u8; 4]| (0..4).all(|i| px[i].abs_diff(expected[i]) <= 2);
        let whole_texture = ContentMask {
            bounds: bounds(
                point(ScaledPixels(0.0), ScaledPixels(0.0)),
                size(
                    ScaledPixels(TEST_SIZE as f32),
                    ScaledPixels(TEST_SIZE as f32),
                ),
            ),
            corner_radii: Corners::default(),
        };

        // With the reflection the blur only ever sees the element's own (opaque) content, so no
        // tap can fade it out: the whole composite is the element's colour at full alpha.
        run(element_bounds, whole_texture)?;
        dump_device_errors(&device);
        for (x, y, what) in [
            (32, 32, "the middle"),
            (17, 32, "just inside the border"),
            (13, 32, "just outside the border"),
            (2, 32, "far outside it"),
        ] {
            let px = read_pixel(&context, &output, &staging, x, y)?;
            assert!(
                close(px, RED),
                "with the box mirrored, {what} ({x}, {y}) should be the element's own colour at \
                 full alpha, got {px:?}"
            );
        }
        // The mirror fills the box's corners as well — which is exactly why the rounded clip
        // below has to be able to cut them back off.
        let filled_corner = read_pixel(&context, &output, &staging, 17, 17)?;
        assert!(
            close(filled_corner, RED),
            "the mirror reaches into the corner of the box, got {filled_corner:?}"
        );

        // Without it — the zero box the pass used to be handed — the same taps read the
        // transparent surround instead, and the border dissolves toward whatever is behind.
        run(Bounds::default(), whole_texture)?;
        let inside = read_pixel(&context, &output, &staging, 17, 32)?;
        assert!(
            (20..=250).contains(&inside[3]),
            "without the mirror the border is partially faded, got {inside:?}"
        );
        // 13.5px out with sigma 12, the gaussian has shed all but ~13% of the coverage.
        let far = read_pixel(&context, &output, &staging, 2, 32)?;
        assert!(
            far[3] < 45 && far[3] < inside[3],
            "and further out it keeps dissolving (inside {inside:?}, far {far:?})"
        );

        // The clip the element is painted under is part of its shape: a rounded `overflow_hidden`
        // ancestor clips the blurred output like anything else it contains. The corner is inside
        // the element's box (the mirror fills it — see the run above) but outside the rounding,
        // so it must be clipped away, while the mid-edge stays solid.
        run(
            element_bounds,
            ContentMask {
                bounds: element_bounds,
                corner_radii: Corners {
                    top_left: ScaledPixels(RADIUS),
                    top_right: ScaledPixels(RADIUS),
                    bottom_right: ScaledPixels(RADIUS),
                    bottom_left: ScaledPixels(RADIUS),
                },
            },
        )?;
        let corner = read_pixel(&context, &output, &staging, 17, 17)?;
        assert!(
            corner[3] < 20,
            "a rounded clip must cut the corner the mirror would otherwise fill, got {corner:?}"
        );
        let edge = read_pixel(&context, &output, &staging, 17, 32)?;
        assert!(
            close(edge, RED),
            "while the mid-edge stays the element's own colour at full alpha, got {edge:?}"
        );

        Ok(())
    }

    /// Renders a horizontal `gray25 -> white` linear gradient with the default Oklab interpolation
    /// and asserts two points on the ramp. The stops are decoded from sRGB, mixed in Oklab, and
    /// re-encoded on the way out: sampled at their pixel centers, the quarter point should land
    /// near 109 and the center near 156. A missing decode, a duplicated transfer function, or a
    /// linear-light mix instead of Oklab moves those to ~94 and ~134 — 15 and 22 code values off,
    /// far outside the windows asserted here.
    ///
    /// Both stops are `Hsla`s that convert to exact 0.25 / 1.0 in the shader's `hsla_to_rgba`, and
    /// the ±5 window covers the gradient dither (±2/255 on RGB) plus the alpha dither blending a
    /// fraction of the clear color into the result.
    #[test]
    fn hlsl_gradient_offscreen_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[gradient_offscreen] WARP device unavailable, skipping");
            return Ok(());
        };
        let (texture, rtv, staging) = make_render_target(&device)?;

        let globals = GlobalParams {
            gamma_ratios: [1.0, 1.0, 1.0, 1.0],
            viewport_size: [TEST_SIZE as f32, TEST_SIZE as f32],
            grayscale_enhanced_contrast: 0.0,
            subpixel_enhanced_contrast: 0.0,
            is_bgr: 0,
            _pad: [0; 3],
        };
        // Must be a real constant buffer, same reason as in `hlsl_quad_offscreen_readback`.
        let globals_buffer = unsafe {
            let desc = D3D11_BUFFER_DESC {
                ByteWidth: std::mem::size_of::<GlobalParams>() as u32,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                ..Default::default()
            };
            let mut buffer = None;
            device.CreateBuffer(&desc, None, Some(&mut buffer))?;
            buffer.unwrap()
        };
        update_buffer(&context, &globals_buffer, &[globals])?;

        let blend = create_blend_state(&device)?;
        let mut pipeline =
            PipelineState::<Quad>::new(&device, "test_gradient", ShaderModule::Quad, 4, blend)?;

        // Horizontal ramp over [8,8]-[56,56], identity transform. Pixels sample at their centers,
        // so for column `x` the ramp parameter is `t = ((x + 0.5) - 32 + 24) / 48`.
        let quad = Quad {
            bounds: bounds(
                point(ScaledPixels(8.0), ScaledPixels(8.0)),
                size(ScaledPixels(48.0), ScaledPixels(48.0)),
            ),
            content_mask: ContentMask {
                bounds: bounds(
                    point(ScaledPixels(0.0), ScaledPixels(0.0)),
                    size(ScaledPixels(64.0), ScaledPixels(64.0)),
                ),
                corner_radii: Corners::default(),
            },
            background: linear_gradient(
                90.0,
                linear_color_stop(hsla(0.0, 0.0, 0.25, 1.0), 0.0),
                linear_color_stop(hsla(0.0, 0.0, 1.0, 1.0), 1.0),
            ),
            ..Default::default()
        };
        pipeline.update_buffer(&device, &context, &[quad])?;

        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: TEST_SIZE as f32,
            Height: TEST_SIZE as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let bg = [0.25, 0.125, 0.5, 1.0];
        unsafe {
            context.ClearRenderTargetView(&rtv, &bg);
            context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv))), None);
        }
        pipeline.draw_range(&device, &context, &[viewport], &[Some(globals_buffer)], 4, 0, 1)?;
        dump_device_errors(&device);

        let quarter = read_pixel(&context, &texture, &staging, 20, 32)
            .expect("readback of quarter pixel failed");
        for channel in &quarter[..3] {
            assert!(
                (104..=114).contains(channel),
                "quarter point (t = 0.26) should be ~109 grey, got {quarter:?} (B,G,R,A)"
            );
        }
        assert!(
            (250..=255).contains(&quarter[3]),
            "gradient pixels should stay opaque, got {quarter:?} (B,G,R,A)"
        );

        let center =
            read_pixel(&context, &texture, &staging, 32, 32).expect("readback of center pixel failed");
        for channel in &center[..3] {
            assert!(
                (151..=161).contains(channel),
                "center (t = 0.51) should be ~156 grey, got {center:?} (B,G,R,A)"
            );
        }
        Ok(())
    }

    /// Renders a radial gradient with the default elliptical ending shape and asserts points along
    /// the diagonal. The ramp is parametrised so the ellipse meets the quad's farthest corner at
    /// t = 1: `t = length(offset / (farthest_side * sqrt(2)))`. With a 48x48 quad centered at
    /// (26, 26), the sampled pixel centers land at t = 0.02, 0.52 and 0.94.
    #[test]
    fn hlsl_radial_gradient_offscreen_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[radial_offscreen] WARP device unavailable, skipping");
            return Ok(());
        };
        let (texture, rtv, staging) = make_render_target(&device)?;

        let globals = GlobalParams {
            gamma_ratios: [1.0, 1.0, 1.0, 1.0],
            viewport_size: [TEST_SIZE as f32, TEST_SIZE as f32],
            grayscale_enhanced_contrast: 0.0,
            subpixel_enhanced_contrast: 0.0,
            is_bgr: 0,
            _pad: [0; 3],
        };
        let globals_buffer = unsafe {
            let desc = D3D11_BUFFER_DESC {
                ByteWidth: std::mem::size_of::<GlobalParams>() as u32,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                ..Default::default()
            };
            let mut buffer = None;
            device.CreateBuffer(&desc, None, Some(&mut buffer))?;
            buffer.unwrap()
        };
        update_buffer(&context, &globals_buffer, &[globals])?;

        let blend = create_blend_state(&device)?;
        let mut pipeline =
            PipelineState::<Quad>::new(&device, "test_radial", ShaderModule::Quad, 4, blend)?;

        let quad = Quad {
            bounds: bounds(
                point(ScaledPixels(2.0), ScaledPixels(2.0)),
                size(ScaledPixels(48.0), ScaledPixels(48.0)),
            ),
            content_mask: ContentMask {
                bounds: bounds(
                    point(ScaledPixels(0.0), ScaledPixels(0.0)),
                    size(ScaledPixels(64.0), ScaledPixels(64.0)),
                ),
                corner_radii: Corners::default(),
            },
            background: radial_gradient(
                point(0.5, 0.5),
                linear_color_stop(hsla(0.0, 0.0, 0.25, 1.0), 0.0),
                linear_color_stop(hsla(0.0, 0.0, 1.0, 1.0), 1.0),
            ),
            ..Default::default()
        };
        pipeline.update_buffer(&device, &context, &[quad])?;

        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: TEST_SIZE as f32,
            Height: TEST_SIZE as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let bg = [0.25, 0.125, 0.5, 1.0];
        unsafe {
            context.ClearRenderTargetView(&rtv, &bg);
            context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv))), None);
        }
        pipeline.draw_range(&device, &context, &[viewport], &[Some(globals_buffer)], 4, 0, 1)?;
        dump_device_errors(&device);

        // Center of the gradient: the first color stop, barely past t = 0.
        let center_pixel =
            read_pixel(&context, &texture, &staging, 26, 26).expect("readback of center failed");
        for channel in &center_pixel[..3] {
            assert!(
                (62..=72).contains(channel),
                "radial center should be ~67 grey, got {center_pixel:?} (B,G,R,A)"
            );
        }

        // Halfway along the diagonal.
        let mid_pixel =
            read_pixel(&context, &texture, &staging, 38, 38).expect("readback of mid failed");
        for channel in &mid_pixel[..3] {
            assert!(
                (153..=163).contains(channel),
                "radial mid-diagonal should be ~158 grey, got {mid_pixel:?} (B,G,R,A)"
            );
        }

        // Close to the farthest corner, which the ending shape meets at t = 1.
        let corner_pixel =
            read_pixel(&context, &texture, &staging, 48, 48).expect("readback of corner failed");
        for channel in &corner_pixel[..3] {
            assert!(
                (237..=247).contains(channel),
                "radial near-corner should be ~242 grey, got {corner_pixel:?} (B,G,R,A)"
            );
        }
        Ok(())
    }

    /// Renders the same ramp with a circular ending shape on a non-square quad, where a circle and
    /// an ellipse diverge. The circle's radius is the distance to the farthest corner (30.46 px on
    /// this 56x24 quad), so the sampled points land at t = 0.64 and 0.54 — an elliptical ending
    /// shape would put them at t = 0.49 and 0.42, ~29 and ~24 code values darker.
    #[test]
    fn hlsl_radial_circle_offscreen_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[radial_circle_offscreen] WARP device unavailable, skipping");
            return Ok(());
        };
        let (texture, rtv, staging) = make_render_target(&device)?;

        let globals = GlobalParams {
            gamma_ratios: [1.0, 1.0, 1.0, 1.0],
            viewport_size: [TEST_SIZE as f32, TEST_SIZE as f32],
            grayscale_enhanced_contrast: 0.0,
            subpixel_enhanced_contrast: 0.0,
            is_bgr: 0,
            _pad: [0; 3],
        };
        let globals_buffer = unsafe {
            let desc = D3D11_BUFFER_DESC {
                ByteWidth: std::mem::size_of::<GlobalParams>() as u32,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                ..Default::default()
            };
            let mut buffer = None;
            device.CreateBuffer(&desc, None, Some(&mut buffer))?;
            buffer.unwrap()
        };
        update_buffer(&context, &globals_buffer, &[globals])?;

        let blend = create_blend_state(&device)?;
        let mut pipeline =
            PipelineState::<Quad>::new(&device, "test_radial_circle", ShaderModule::Quad, 4, blend)?;

        let quad = Quad {
            bounds: bounds(
                point(ScaledPixels(4.0), ScaledPixels(20.0)),
                size(ScaledPixels(56.0), ScaledPixels(24.0)),
            ),
            content_mask: ContentMask {
                bounds: bounds(
                    point(ScaledPixels(0.0), ScaledPixels(0.0)),
                    size(ScaledPixels(64.0), ScaledPixels(64.0)),
                ),
                corner_radii: Corners::default(),
            },
            background: radial_gradient(
                point(0.5, 0.5),
                linear_color_stop(hsla(0.0, 0.0, 0.25, 1.0), 0.0),
                linear_color_stop(hsla(0.0, 0.0, 1.0, 1.0), 1.0),
            )
            .radial_shape(RadialShape::Circle),
            ..Default::default()
        };
        pipeline.update_buffer(&device, &context, &[quad])?;

        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: TEST_SIZE as f32,
            Height: TEST_SIZE as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let bg = [0.25, 0.125, 0.5, 1.0];
        unsafe {
            context.ClearRenderTargetView(&rtv, &bg);
            context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv))), None);
        }
        pipeline.draw_range(&device, &context, &[viewport], &[Some(globals_buffer)], 4, 0, 1)?;
        dump_device_errors(&device);

        let left_pixel =
            read_pixel(&context, &texture, &staging, 12, 32).expect("readback of left failed");
        for channel in &left_pixel[..3] {
            assert!(
                (176..=186).contains(channel),
                "circle left point should be ~181 grey, got {left_pixel:?} (B,G,R,A)"
            );
        }

        let right_pixel =
            read_pixel(&context, &texture, &staging, 48, 32).expect("readback of right failed");
        for channel in &right_pixel[..3] {
            assert!(
                (157..=167).contains(channel),
                "circle right point should be ~162 grey, got {right_pixel:?} (B,G,R,A)"
            );
        }
        Ok(())
    }

    /// Renders the same ramp once per CSS ending-shape size keyword and asserts the same pixel in
    /// each. The quad spans [4,20]-[60,44] with its center at `point(0.25, 0.5)`, so the nearest
    /// side is 14px from the center and the farthest 42px: at pixel center (30.5, 32.5) the four
    /// keywords land at t = 0.89, 0.30, 0.63 and 0.21 — 233, 116, 180 and 100 in code values, each
    /// at least 20 apart, so a wrong keyword cannot pass by accident.
    #[test]
    fn hlsl_radial_size_offscreen_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[radial_size_offscreen] WARP device unavailable, skipping");
            return Ok(());
        };
        let (texture, rtv, staging) = make_render_target(&device)?;

        let globals = GlobalParams {
            gamma_ratios: [1.0, 1.0, 1.0, 1.0],
            viewport_size: [TEST_SIZE as f32, TEST_SIZE as f32],
            grayscale_enhanced_contrast: 0.0,
            subpixel_enhanced_contrast: 0.0,
            is_bgr: 0,
            _pad: [0; 3],
        };
        let globals_buffer = unsafe {
            let desc = D3D11_BUFFER_DESC {
                ByteWidth: std::mem::size_of::<GlobalParams>() as u32,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                ..Default::default()
            };
            let mut buffer = None;
            device.CreateBuffer(&desc, None, Some(&mut buffer))?;
            buffer.unwrap()
        };
        update_buffer(&context, &globals_buffer, &[globals])?;

        let blend = create_blend_state(&device)?;
        let mut pipeline =
            PipelineState::<Quad>::new(&device, "test_radial_size", ShaderModule::Quad, 4, blend)?;

        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: TEST_SIZE as f32,
            Height: TEST_SIZE as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let bg = [0.25, 0.125, 0.5, 1.0];
        unsafe {
            context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv.clone()))), None);
        }

        for (keyword, expected) in [
            (RadialSize::ClosestSide, 233),
            (RadialSize::FarthestSide, 116),
            (RadialSize::ClosestCorner, 180),
            (RadialSize::FarthestCorner, 100),
        ] {
            let quad = Quad {
                bounds: bounds(
                    point(ScaledPixels(4.0), ScaledPixels(20.0)),
                    size(ScaledPixels(56.0), ScaledPixels(24.0)),
                ),
                content_mask: ContentMask {
                    bounds: bounds(
                        point(ScaledPixels(0.0), ScaledPixels(0.0)),
                        size(ScaledPixels(64.0), ScaledPixels(64.0)),
                    ),
                    corner_radii: Corners::default(),
                },
                background: radial_gradient(
                    point(0.25, 0.5),
                    linear_color_stop(hsla(0.0, 0.0, 0.25, 1.0), 0.0),
                    linear_color_stop(hsla(0.0, 0.0, 1.0, 1.0), 1.0),
                )
                .radial_size(keyword),
                ..Default::default()
            };
            pipeline.update_buffer(&device, &context, &[quad])?;
            unsafe {
                context.ClearRenderTargetView(&rtv, &bg);
            }
            pipeline.draw_range(
                &device,
                &context,
                &[viewport],
                &[Some(globals_buffer.clone())],
                4,
                0,
                1,
            )?;
            dump_device_errors(&device);

            let pixel =
                read_pixel(&context, &texture, &staging, 30, 32).expect("readback failed");
            for channel in &pixel[..3] {
                assert!(
                    (expected - 5..=expected + 5).contains(channel),
                    "{keyword:?} should be ~{expected} grey, got {pixel:?} (B,G,R,A)"
                );
            }
        }

        // A circular ending shape takes a single radius, so at the same center the farthest side
        // is 42px along *both* axes: 8.5px above the center the circle is at t = 0.20 while the
        // ellipse (whose y radius is only 12px) is at t = 0.71 — 99 vs 195 in code values.
        let quad = Quad {
            bounds: bounds(
                point(ScaledPixels(4.0), ScaledPixels(20.0)),
                size(ScaledPixels(56.0), ScaledPixels(24.0)),
            ),
            content_mask: ContentMask {
                bounds: bounds(
                    point(ScaledPixels(0.0), ScaledPixels(0.0)),
                    size(ScaledPixels(64.0), ScaledPixels(64.0)),
                ),
                corner_radii: Corners::default(),
            },
            background: radial_gradient(
                point(0.25, 0.5),
                linear_color_stop(hsla(0.0, 0.0, 0.25, 1.0), 0.0),
                linear_color_stop(hsla(0.0, 0.0, 1.0, 1.0), 1.0),
            )
            .radial_size(RadialSize::FarthestSide)
            .radial_shape(RadialShape::Circle),
            ..Default::default()
        };
        pipeline.update_buffer(&device, &context, &[quad])?;
        unsafe {
            context.ClearRenderTargetView(&rtv, &bg);
        }
        pipeline.draw_range(&device, &context, &[viewport], &[Some(globals_buffer)], 4, 0, 1)?;
        dump_device_errors(&device);

        let pixel = read_pixel(&context, &texture, &staging, 18, 40).expect("readback failed");
        for channel in &pixel[..3] {
            assert!(
                (94..=104).contains(channel),
                "circle farthest-side should be ~99 grey, got {pixel:?} (B,G,R,A)"
            );
        }
        Ok(())
    }

    /// Draws two differently colored quads in one instanced call and asserts a pixel inside each.
    /// The field offsets can agree while the *stride* between instances still disagrees — a struct
    /// that grows on one side only. That kind of mismatch leaves instance 0 correct and every later
    /// instance reading garbage, so it needs its own check.
    #[test]
    fn hlsl_two_instance_stride_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[two_instance_offscreen] WARP device unavailable, skipping");
            return Ok(());
        };
        let (texture, rtv, staging) = make_render_target(&device)?;

        let globals = GlobalParams {
            gamma_ratios: [1.0, 1.0, 1.0, 1.0],
            viewport_size: [TEST_SIZE as f32, TEST_SIZE as f32],
            grayscale_enhanced_contrast: 0.0,
            subpixel_enhanced_contrast: 0.0,
            is_bgr: 0,
            _pad: [0; 3],
        };
        let globals_buffer = unsafe {
            let desc = D3D11_BUFFER_DESC {
                ByteWidth: std::mem::size_of::<GlobalParams>() as u32,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                ..Default::default()
            };
            let mut buffer = None;
            device.CreateBuffer(&desc, None, Some(&mut buffer))?;
            buffer.unwrap()
        };
        update_buffer(&context, &globals_buffer, &[globals])?;

        let blend = create_blend_state(&device)?;
        let mut pipeline =
            PipelineState::<Quad>::new(&device, "test_two_instances", ShaderModule::Quad, 4, blend)?;

        let content_mask = ContentMask {
            bounds: bounds(
                point(ScaledPixels(0.0), ScaledPixels(0.0)),
                size(ScaledPixels(64.0), ScaledPixels(64.0)),
            ),
            corner_radii: Corners::default(),
        };
        let red = Quad {
            bounds: bounds(
                point(ScaledPixels(4.0), ScaledPixels(4.0)),
                size(ScaledPixels(24.0), ScaledPixels(24.0)),
            ),
            content_mask,
            background: Background::from(rgba(0xff0000ff)),
            ..Default::default()
        };
        let green = Quad {
            bounds: bounds(
                point(ScaledPixels(36.0), ScaledPixels(36.0)),
                size(ScaledPixels(24.0), ScaledPixels(24.0)),
            ),
            content_mask,
            background: Background::from(rgba(0x00ff00ff)),
            ..Default::default()
        };
        pipeline.update_buffer(&device, &context, &[red, green])?;

        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: TEST_SIZE as f32,
            Height: TEST_SIZE as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let bg = [0.25, 0.125, 0.5, 1.0];
        unsafe {
            context.ClearRenderTargetView(&rtv, &bg);
            context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv))), None);
        }
        pipeline.draw_range(&device, &context, &[viewport], &[Some(globals_buffer)], 4, 0, 2)?;
        dump_device_errors(&device);

        let first = read_pixel(&context, &texture, &staging, 16, 16).expect("readback failed");
        assert_eq!(
            first,
            [0, 0, 255, 255],
            "instance 0 should be solid red (B,G,R,A), got {first:?}"
        );
        let second = read_pixel(&context, &texture, &staging, 48, 48).expect("readback failed");
        assert_eq!(
            second,
            [0, 255, 0, 255],
            "instance 1 should be solid green (B,G,R,A) — a stride mismatch reads garbage here, got {second:?}"
        );
        Ok(())
    }
}
