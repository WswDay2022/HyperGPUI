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
use hgpui_render::artifacts::Dx11DrawConstants;

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
    smoothed_shadow_pipeline: PipelineState<Shadow>,
    quad_pipeline: PipelineState<Quad>,
    smoothed_quad_pipeline: PipelineState<Quad>,
    path_rasterization_pipeline: PipelineState<PathRasterizationSprite>,
    path_sprite_pipeline: PipelineState<PathSprite>,
    underline_pipeline: PipelineState<Underline>,
    mono_sprites: PipelineState<MonochromeSprite>,
    subpixel_sprites: PipelineState<SubpixelSprite>,
    poly_sprites: PipelineState<PolychromeSprite>,
    smoothed_poly_sprites: PipelineState<PolychromeSprite>,
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
    font_params_buffer: Option<ID3D11Buffer>,
    draw_constants_buffer: Option<ID3D11Buffer>,
    /// Per-frame arena of every content-mask chain (see `Scene::mask_chains`). Bound as a raw
    /// buffer at `t5` for the whole frame; primitives index it through their `mask_chain` field.
    mask_buffer: Option<ID3D11Buffer>,
    mask_view: Option<ID3D11ShaderResourceView>,
    mask_capacity: usize,
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

    fn pre_draw(&self, clear_color: &[f32; 4], scene: &Scene) -> Result<()> {
        let resources = self.resources.as_ref().expect("resources missing");
        let device_context = &self
            .devices
            .as_ref()
            .expect("devices missing")
            .device_context;
        update_buffer(
            device_context,
            self.globals.global_params_buffer.as_ref().unwrap(),
            &[GlobalUniforms {
                viewport_size: [resources.viewport.Width, resources.viewport.Height],
                premultiplied_alpha: 0,
                padding: 0,
                window_mask_params: if scene.window_mask.is_some() {
                    [1.0, 0.0, 0.0, 0.0]
                } else {
                    [0.0; 4]
                },
                window_mask: scene.window_mask.unwrap_or_default(),
            }],
        )?;
        update_buffer(
            device_context,
            self.globals.font_params_buffer.as_ref().unwrap(),
            &[FontRasterizationUniforms {
                gamma_ratios: self.font_info.gamma_ratios,
                grayscale_enhanced_contrast: self.font_info.grayscale_enhanced_contrast,
                subpixel_enhanced_contrast: self.font_info.subpixel_enhanced_contrast,
                uses_blue_green_red_subpixel_order: self.font_info.is_bgr as u32,
                padding: 0,
            }],
        )?;
        update_buffer(
            device_context,
            self.globals.draw_constants_buffer.as_ref().unwrap(),
            &[Dx11DrawConstants::default()],
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
        result.ok().context("Presenting swap chain failed")?;
        if let Some(composition) = self.direct_composition.as_ref() {
            composition.commit()?;
        }
        Ok(())
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
        self.pre_draw(
            &match background_appearance {
                WindowBackgroundAppearance::Opaque => [1.0f32; 4],
                _ => [0.0f32; 4],
            },
            scene,
        )?;

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

        // The content-mask chain arena is shared by every primitive pipeline for the whole
        // frame; no other pass binds t5, so one bind here is enough.
        if let Some(mask_view) = self.globals.mask_view.clone() {
            let binding = Some(mask_view);
            unsafe {
                ctx.PSSetShaderResources(5, Some(slice::from_ref(&binding)));
                ctx.VSSetShaderResources(5, Some(slice::from_ref(&binding)));
            }
        }

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
            let batch_label = batch.label();
            let _annotation = annotation
                .as_ref()
                .map(|annotation| Annotation::new(annotation, HSTRING::from(batch_label.clone())));
            let batch_result = match batch {
                PrimitiveBatch::Shadows { range, smoothed } => {
                    self.draw_shadows(range.start, range.len(), smoothed)
                }
                PrimitiveBatch::Quads { range, smoothed } => {
                    self.draw_quads(range.start, range.len(), smoothed)
                }
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
                PrimitiveBatch::PolychromeSprites {
                    texture_id,
                    range,
                    smoothed,
                } => self.draw_polychrome_sprites(texture_id, range.start, range.len(), smoothed),
                PrimitiveBatch::Surfaces(range) => self.draw_surfaces(&scene.surfaces[range]),
                PrimitiveBatch::BackdropFilters(range) => {
                    let result = (|| {
                        for filter in &scene.backdrop_filters[range] {
                            self.dx_blur_and_composite(
                                &current_srv,
                                &current_rtv,
                                filter.bounds,
                                filter.content_mask,
                                filter.mask_chain,
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
                                boundary.mask_chain,
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
            };
            batch_result.with_context(|| {
                format!(
                    "scene too large (batch {batch_label}):\
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

        // Present the offscreen scene by drawing it into the swapchain. This must be a draw, not
        // `CopyResource`: a flip-model swapchain's back buffer cannot be used as a copy
        // destination, and copying into the texture reference acquired at startup leaves the
        // presented frame frozen while the UI keeps rendering internally.
        if use_offscreen {
            let resources = self.resources.as_ref().context("resources missing")?;
            self.dx_blit(
                &resources.blur.scene_color_srv,
                &resources.render_target_view,
            )?;
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

        self.globals.update_content_masks(
            &devices.device,
            &devices.device_context,
            &scene.mask_chains,
        )?;

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

    fn draw_shadows(&mut self, start: usize, len: usize, smoothed: bool) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let devices = self.devices.as_ref().context("devices missing")?;
        let pipeline = if smoothed {
            &self.pipelines.smoothed_shadow_pipeline
        } else {
            &self.pipelines.shadow_pipeline
        };
        pipeline.draw_range(
            &devices.device,
            &devices.device_context,
            slice::from_ref(
                &self
                    .resources
                    .as_ref()
                    .context("resources missing")?
                    .viewport,
            ),
            &self.globals.constant_buffers(None),
            4,
            start as u32,
            len as u32,
        )
    }

    fn draw_quads(&mut self, start: usize, len: usize, smoothed: bool) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let devices = self.devices.as_ref().context("devices missing")?;
        let pipeline = if smoothed {
            &self.pipelines.smoothed_quad_pipeline
        } else {
            &self.pipelines.quad_pipeline
        };
        pipeline.draw_range(
            &devices.device,
            &devices.device_context,
            slice::from_ref(
                &self
                    .resources
                    .as_ref()
                    .context("resources missing")?
                    .viewport,
            ),
            &self.globals.constant_buffers(None),
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
                curve_position: v.st_position,
                color: path.color,
                bounds: path.bounds,
                content_mask: path.content_mask.bounds,
                mask_chain: path.mask_chain,
                padding: [0; 3],
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
            &self.globals.constant_buffers(None),
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
            &self.globals.constant_buffers(None),
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
            &self.globals.constant_buffers(None),
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
            &self.globals.constant_buffers(None),
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
            &self.globals.constant_buffers(None),
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
        smoothed: bool,
    ) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        let texture_view = self.atlas.get_texture_view(texture_id);
        let pipeline = if smoothed {
            &self.pipelines.smoothed_poly_sprites
        } else {
            &self.pipelines.poly_sprites
        };
        pipeline.draw_range_with_texture(
            &devices.device,
            &devices.device_context,
            &texture_view,
            slice::from_ref(&resources.viewport),
            &self.globals.constant_buffers(None),
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
        unsafe {
            // Unbind any SRV at slot 0 so the target texture isn't simultaneously bound as input.
            ctx.PSSetShaderResources(3, Some(&null_srv));
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
            let constant_buffers =
                self.globals
                    .constant_buffers(Some(&self.pipelines.blur_params_buffer));
            ctx.VSSetConstantBuffers(0, Some(&constant_buffers));
            ctx.PSSetConstantBuffers(0, Some(&constant_buffers));
            // Blur passes sample with a clamp sampler: gaussian taps that leave the texture at
            // the window edge must not wrap around to the opposite side.
            ctx.PSSetSamplers(4, Some(slice::from_ref(&self.globals.blur_sampler)));
            ctx.PSSetShaderResources(3, Some(slice::from_ref(source_srv)));
            ctx.OMSetBlendState(blend, None, 0xFFFFFFFF);
            ctx.DrawInstanced(vertex_count, 1, 0, 0);
            // Unbind the source so the target can be rebound as a render target next.
            ctx.PSSetShaderResources(3, Some(&null_srv));
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
        mask_chain: u32,
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
                mask_chain,
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

    /// Present the offscreen scene texture by drawing a full-screen quad into the swapchain.
    fn dx_blit(
        &self,
        source_srv: &Option<ID3D11ShaderResourceView>,
        target_rtv: &Option<ID3D11RenderTargetView>,
    ) -> Result<()> {
        let full_vp = self
            .resources
            .as_ref()
            .context("resources missing")?
            .viewport;
        self.dx_blur_pass(
            &self.pipelines.blur_downsample_vertex,
            &self.pipelines.blur_downsample_fragment,
            &self.pipelines.blur_blend_replace,
            target_rtv,
            source_srv,
            BlurParams {
                downsample: 0.0,
                ..Default::default()
            },
            &full_vp,
            D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            3,
            true,
        )
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
        let smoothed_shadow_pipeline = PipelineState::new(
            device,
            "smoothed_shadow_pipeline",
            ShaderModule::SmoothedShadow,
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
        let smoothed_quad_pipeline = PipelineState::new(
            device,
            "smoothed_quad_pipeline",
            ShaderModule::SmoothedQuad,
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
        let smoothed_poly_sprites = PipelineState::new(
            device,
            "smoothed_polychrome_sprite_pipeline",
            ShaderModule::SmoothedPolychromeSprite,
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
            smoothed_shadow_pipeline,
            quad_pipeline,
            smoothed_quad_pipeline,
            path_rasterization_pipeline,
            path_sprite_pipeline,
            underline_pipeline,
            mono_sprites,
            subpixel_sprites,
            poly_sprites,
            smoothed_poly_sprites,
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

    /// Commit pending DirectComposition property changes (e.g. a newly presented swapchain
    /// frame). Without this the compositor keeps showing the last committed frame.
    pub fn commit(&self) -> Result<()> {
        unsafe {
            self.comp_device.Commit()?;
        }
        Ok(())
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
    fn constant_buffers(&self, blur: Option<&ID3D11Buffer>) -> [Option<ID3D11Buffer>; 4] {
        [
            self.global_params_buffer.clone(),
            self.font_params_buffer.clone(),
            blur.cloned(),
            self.draw_constants_buffer.clone(),
        ]
    }

    pub fn new(device: &ID3D11Device) -> Result<Self> {
        let global_params_buffer =
            Some(create_constant_buffer(device, std::mem::size_of::<GlobalUniforms>())?);

        let font_params_buffer = create_constant_buffer(device, std::mem::size_of::<FontRasterizationUniforms>())?;
        let draw_constants_buffer = create_constant_buffer(device, std::mem::size_of::<Dx11DrawConstants>())?;

        let sampler = unsafe {
            // Clamp rather than wrap: atlas tiles are packed edge to edge, so a wrap at the
            // atlas border replays content from the opposite side. Per-tile texel clamping in
            // the shader keeps samples inside their own tile; this only guards the outer edge.
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
            output
        };

        let blur_sampler = create_blur_sampler(device)?;

        // Always keep one mask resident so the binding is valid even on frames with no clips.
        let mask_capacity = 8;
        let mask_buffer = create_buffer(
            device,
            std::mem::size_of::<ContentMask<ScaledPixels>>(),
            mask_capacity,
        )?;
        let mask_view = create_buffer_view(device, &mask_buffer)?;

        Ok(Self {
            global_params_buffer,
            font_params_buffer: Some(font_params_buffer),
            draw_constants_buffer: Some(draw_constants_buffer),
            mask_buffer: Some(mask_buffer),
            mask_view,
            mask_capacity,
            sampler,
            blur_sampler,
        })
    }

    /// Uploads the frame's content-mask chain arena (`Scene::mask_chains`) into the raw buffer
    /// indexed by every primitive's `mask_chain` field.
    fn update_content_masks(
        &mut self,
        device: &ID3D11Device,
        device_context: &ID3D11DeviceContext,
        masks: &[ContentMask<ScaledPixels>],
    ) -> Result<()> {
        let capacity = self.mask_capacity.max(1);
        if masks.len() > capacity {
            let new_capacity = masks.len().next_power_of_two();
            let buffer = create_buffer(
                device,
                std::mem::size_of::<ContentMask<ScaledPixels>>(),
                new_capacity,
            )?;
            let view = create_buffer_view(device, &buffer)?;
            self.mask_buffer = Some(buffer);
            self.mask_view = view;
            self.mask_capacity = new_capacity;
        }
        let buffer = self.mask_buffer.as_ref().context("mask buffer missing")?;
        let mut data = masks.to_vec();
        if data.is_empty() {
            // A sentinel that covers the whole scene; chain reference 0 never indexes it, but a
            // valid first element keeps accidental reads from clipping everything away.
            data.push(ContentMask {
                bounds: Bounds {
                    origin: point(ScaledPixels(-1.0e9), ScaledPixels(-1.0e9)),
                    size: size(ScaledPixels(2.0e9), ScaledPixels(2.0e9)),
                },
                corner_radii: Default::default(),
                fade_out: Default::default(),
            });
        }
        update_buffer(device_context, buffer, &data)
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

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct GlobalUniforms {
    viewport_size: [f32; 2],
    premultiplied_alpha: u32,
    padding: u32,
    window_mask_params: [f32; 4],
    window_mask: ContentMask<ScaledPixels>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct FontRasterizationUniforms {
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
    subpixel_enhanced_contrast: f32,
    uses_blue_green_red_subpixel_order: u32,
    padding: u32,
}

/// Mirrors `BlurUniforms` from `hgpui_render`; bound as the b2 constant buffer and updated per
/// blur pass via `update_buffer`.
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
    /// Packed reference into the frame's content-mask chain arena; the chain carries the mask's
    /// corner radii and fades.
    mask_chain: u32,
    padding0: u32,
    padding1: u32,
    padding2: u32,
    corner_radii: [f32; 4],
    direction: [f32; 2],
    sigma: f32,
    opacity: f32,
    tap_count: f32,
    /// Spacing between taps in pixels (gaussian passes only); >1 lets `tap_count` taps span very
    /// large radii without truncating the gaussian, matching the wgpu backend.
    tap_step: f32,
    /// 1.0 clips the composite to the rounded rect (backdrop); 0.0 lets content blur spread past
    /// its bounds (the blur itself no longer fades at the edge — see `mirror_into_rect`).
    clip_rounded: f32,
    /// 1.0 = snapped 2:1 box downsample (anchor the half-res grid to a fixed 2px grid at the
    /// origin, so a stationary element blurs identically at every window size); 0.0 = 1:1 copy
    /// (the scene blit, which must not downsample). Downsample pass only.
    downsample: f32,
}

impl Default for BlurParams {
    fn default() -> Self {
        BlurParams {
            bounds: Bounds::default(),
            content_mask: Bounds::default(),
            content_mask_radii: [0.0; 4],
            mask_chain: 0,
            padding0: 0,
            padding1: 0,
            padding2: 0,
            corner_radii: [0.0; 4],
            direction: [0.0, 0.0],
            sigma: 0.0,
            opacity: 1.0,
            tap_count: 0.0,
            tap_step: 0.0,
            clip_rounded: 0.0,
            downsample: 0.0,
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
            device_context.PSSetSamplers(4, Some(sampler));
            device_context.VSSetShaderResources(3, Some(texture));
            device_context.PSSetShaderResources(3, Some(texture));

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
        let stride = std::mem::size_of::<T>() as u32;
        let view = create_buffer_view_range(
            device,
            &self.buffer,
            first_instance * stride,
            instance_count * stride,
        )
        .with_context(|| {
            format!(
                "sprite buffer view ({}) first={} count={} stride={} buffer_elems={}",
                self.label, first_instance, instance_count, stride, self.buffer_size
            )
        })?;
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
        let stride = std::mem::size_of::<T>() as u32;
        let view = create_buffer_view_range(
            device,
            &self.buffer,
            first_instance * stride,
            instance_count * stride,
        )
        .with_context(|| {
            format!(
                "instances buffer view ({}) first={} count={} stride={} buffer_elems={}",
                self.label, first_instance, instance_count, stride, self.buffer_size
            )
        })?;
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
            device_context.PSSetSamplers(4, Some(sampler));
            device_context.VSSetShaderResources(3, Some(texture));
            device_context.PSSetShaderResources(3, Some(texture));
            device_context.DrawInstanced(4, instance_count, 0, 0);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
struct PathRasterizationSprite {
    xy_position: Point<ScaledPixels>,
    curve_position: Point<f32>,
    color: Background,
    /// The path's own bounds: the gradient box. (Used to be `clipped_bounds`, which made the
    /// gradient shift/scale whenever an ancestor clip cut the path.)
    bounds: Bounds<ScaledPixels>,
    /// The content mask the path is painted under. The shader clips with the padded AA margin
    /// and cuts the mask's distance field in the fragment, so rounded mask corners apply to
    /// vector paths too.
    content_mask: Bounds<ScaledPixels>,
    /// Packed reference into the frame's content-mask chain arena.
    mask_chain: u32,
    /// Pads the stride to 144 bytes.
    padding: [u32; 3],
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
        MiscFlags: D3D11_RESOURCE_MISC_BUFFER_ALLOW_RAW_VIEWS.0 as u32,
        StructureByteStride: 0,
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
    let mut buffer_desc = D3D11_BUFFER_DESC::default();
    unsafe { buffer.GetDesc(&mut buffer_desc) };
    let desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
        Format: DXGI_FORMAT_R32_TYPELESS,
        ViewDimension: D3D11_SRV_DIMENSION_BUFFEREX,
        Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
            BufferEx: D3D11_BUFFEREX_SRV {
                FirstElement: 0,
                NumElements: buffer_desc.ByteWidth / 4,
                Flags: D3D11_BUFFEREX_SRV_FLAG_RAW.0 as u32,
            },
        },
    };
    let mut view = None;
    unsafe { device.CreateShaderResourceView(buffer, Some(&desc), Some(&mut view)) }?;
    Ok(view)
}

#[inline]
fn create_buffer_view_range(
    device: &ID3D11Device,
    buffer: &ID3D11Buffer,
    first_byte: u32,
    byte_len: u32,
) -> Result<Option<ID3D11ShaderResourceView>> {
    assert_eq!(first_byte % 4, 0, "raw view offsets must be word aligned");
    assert_eq!(byte_len % 4, 0, "raw view sizes must be word aligned");
    let desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
        Format: DXGI_FORMAT_R32_TYPELESS,
        ViewDimension: D3D11_SRV_DIMENSION_BUFFEREX,
        Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
            BufferEx: D3D11_BUFFEREX_SRV {
                FirstElement: first_byte / 4,
                NumElements: byte_len / 4,
                Flags: D3D11_BUFFEREX_SRV_FLAG_RAW.0 as u32,
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
        device_context.VSSetShaderResources(2, Some(buffer_view));
        device_context.PSSetShaderResources(2, Some(buffer_view));
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
    //! Bytecode generated from the shared Rust shader sources at build time.

    use anyhow::Result;
    use hgpui_render::artifacts::{Dx11Bytecode, Dx11Shader, NATIVE_SHADERS, NativeShader};

    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub(crate) enum ShaderModule {
        Quad,
        SmoothedQuad,
        Shadow,
        SmoothedShadow,
        Underline,
        PathRasterization,
        PathSprite,
        MonochromeSprite,
        SubpixelSprite,
        PolychromeSprite,
        SmoothedPolychromeSprite,
        EmojiRasterization,
        BlurDownsample,
        Blur,
        BlurComposite,
    }

    impl ShaderModule {
        pub(crate) fn label(self) -> &'static str {
            match self {
                ShaderModule::Quad => "quad",
                ShaderModule::SmoothedQuad => "smoothed_quad",
                ShaderModule::Shadow => "shadow",
                ShaderModule::SmoothedShadow => "smoothed_shadow",
                ShaderModule::Underline => "underline",
                ShaderModule::PathRasterization => "path_rasterization",
                ShaderModule::PathSprite => "path_sprite",
                ShaderModule::MonochromeSprite => "monochrome_sprite",
                ShaderModule::SubpixelSprite => "subpixel_sprite",
                ShaderModule::PolychromeSprite => "polychrome_sprite",
                ShaderModule::SmoothedPolychromeSprite => "smoothed_polychrome_sprite",
                ShaderModule::EmojiRasterization => "emoji_rasterization",
                ShaderModule::BlurDownsample => "blur_downsample",
                ShaderModule::Blur => "blur",
                ShaderModule::BlurComposite => "blur_composite",
            }
        }

        pub(crate) fn native(self) -> &'static NativeShader {
            NATIVE_SHADERS
                .iter()
                .find(|shader| shader.label == self.label())
                .unwrap_or_else(|| panic!("missing generated native shader {}", self.label()))
        }

        pub(crate) fn bytecode(self) -> Result<Dx11Bytecode> {
            let shader = self.native();
            match shader.dx11 {
                Dx11Shader::Sm50(bytecode) => Ok(bytecode),
                Dx11Shader::NativeWindowsBuildRequired => anyhow::bail!(
                    "{} has no DX11 bytecode: build the Windows target on a Windows host",
                    shader.label
                ),
            }
        }
    }

    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub(crate) enum ShaderTarget {
        Vertex,
        Fragment,
    }

    pub(crate) struct RawShaderBytes {
        inner: &'static [u8],
    }

    impl RawShaderBytes {
        pub(crate) fn new(module: ShaderModule, target: ShaderTarget) -> Result<Self> {
            let bytecode = module.bytecode()?;
            Ok(Self {
                inner: match target {
                    ShaderTarget::Vertex => bytecode.vertex,
                    ShaderTarget::Fragment => bytecode.fragment,
                },
            })
        }

        pub(crate) fn as_bytes(&self) -> &'static [u8] {
            self.inner
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
        create_blend_state, create_blend_state_for_path_rasterization,
        create_blend_state_for_path_sprite, create_blend_state_no_blend,
        create_buffer, create_buffer_view, create_buffer_view_range, corner_radii_array, create_blur_sampler,
        create_color_target, create_constant_buffer,
        create_fragment_shader, create_vertex_shader, set_pipeline_state, update_buffer,
        BlurParams,
        FontRasterizationUniforms, GlobalUniforms, PathRasterizationSprite,
        PipelineState,
        RawShaderBytes, ShaderTarget, RENDER_TARGET_FORMAT,
    };
    use crate::directx_renderer::shader_resources::ShaderModule;
    use anyhow::Result;
    use hgpui::{
        bounds, hsla, linear_color_stop, linear_gradient, point, px, radial_gradient, rgba, size,
        AtlasTextureId, AtlasTextureKind, AtlasTile, Background, BorderStyle, Bounds, ColorSpace,
        ContentMask, Corners, DevicePixels, Edges, PolychromeSprite, Quad, RadialShape, RadialSize,
        ScaledPixels, Shadow, TileId, TransformationMatrix, red, transparent_black,
    };
    use std::slice;
    use windows::core::Interface;
    use windows::Win32::{
        Foundation::HMODULE,
        Graphics::{
            Direct3D::{
                D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_PRIMITIVE_TOPOLOGY,
                D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
            },
            Direct3D11::{
                D3D11CreateDevice, D3D11_BIND_RENDER_TARGET,
                D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_DEBUG, D3D11_MAP_READ,
                D3D11_MAPPED_SUBRESOURCE, D3D11_MESSAGE, D3D11_SDK_VERSION,
                D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
                D3D11_USAGE_STAGING, D3D11_VIEWPORT, ID3D11BlendState,
                ID3D11Buffer, ID3D11Device, ID3D11DeviceContext, ID3D11InfoQueue,
                ID3D11PixelShader, ID3D11RenderTargetView, ID3D11SamplerState,
                ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
            },
            Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC},
        },
    };

    const TEST_SIZE: u32 = 64;

    /// Same as [`create_warp_device`], but on the real GPU. Used to catch behaviour that the
    /// WARP software rasterizer does not reproduce.
    fn create_hardware_device() -> Option<(ID3D11Device, ID3D11DeviceContext)> {
        unsafe {
            let mut device: Option<ID3D11Device> = None;
            let mut device_context: Option<ID3D11DeviceContext> = None;
            let mut result = D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut device_context),
            );
            if result.is_err() {
                device = None;
                device_context = None;
                result = D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_DEBUG | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
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

        let globals = install_globals(&device, &context)?;

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

                fade_out: Default::default(),
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
        pipeline.draw_range(&device, &context, &[viewport], &globals, 4, 0, 1)?;
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

    /// Borders and rounded corners must follow a CSS transform, and the mask-equals-own-cutout
    /// skip must not depend on the transform being identity: when the mask is the quad's own
    /// untransformed rounded rect, the quad's own local-space SDF already cuts exactly that
    /// outline, so the mask must be skipped even for translated/skewed quads — otherwise the
    /// mask slices the moved quad at its untransformed rect and the trailing edge is eaten.
    #[test]
    fn hlsl_quad_transformed_border_and_cutout_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[quad_transform] WARP device unavailable, skipping");
            return Ok(());
        };
        let (texture, rtv, staging) = make_render_target(&device)?;

        let globals = install_globals(&device, &context)?;

        let blend = create_blend_state(&device)?;
        let mut pipeline =
            PipelineState::<Quad>::new(&device, "test_quad_transform", ShaderModule::Quad, 4, blend)?;

        let quad_bounds = bounds(
            point(ScaledPixels(8.0), ScaledPixels(8.0)),
            size(ScaledPixels(48.0), ScaledPixels(48.0)),
        );
        let radius = ScaledPixels(8.0);
        let rounded_corners = Corners {
            top_left: radius,
            top_right: radius,
            bottom_left: radius,
            bottom_right: radius,
        };
        let viewport_mask = ContentMask {
            bounds: bounds(
                point(ScaledPixels(0.0), ScaledPixels(0.0)),
                size(ScaledPixels(64.0), ScaledPixels(64.0)),
            ),
            corner_radii: Corners::default(),

            fade_out: Default::default(),
        };

        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: TEST_SIZE as f32,
            Height: TEST_SIZE as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let bg = [0.25, 0.125, 0.5, 1.0];
        let clear = [128u8, 32, 64, 255];
        let red_px = [0u8, 0, 255, 255];

        let draw_quad = |pipeline: &mut PipelineState<Quad>, quad: Quad| -> Result<()> {
            pipeline.update_buffer(&device, &context, &[quad])?;
            unsafe {
                context.ClearRenderTargetView(&rtv, &bg);
                context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv.clone()))), None);
            }
            pipeline.draw_range(
                &device,
                &context,
                &[viewport],
                &globals,
                4,
                0,
                1,
            )?;
            dump_device_errors(&device);
            Ok(())
        };

        let translate = TransformationMatrix::unit().translate(point(
            ScaledPixels(8.0),
            ScaledPixels(0.0),
        ));

        // Case 1: border-only quad, translated right by 8px. All four border bands must be
        // present at the moved position, and the interior must stay clear.
        let border_quad = Quad {
            bounds: quad_bounds,
            content_mask: viewport_mask,
            background: Background::from(transparent_black()),
            border_color: red().into(),
            border_widths: Edges {
                top: ScaledPixels(4.0),
                right: ScaledPixels(4.0),
                bottom: ScaledPixels(4.0),
                left: ScaledPixels(4.0),
            },
            transformation: translate,
            ..Default::default()
        };
        draw_quad(&mut pipeline, border_quad)?;
        for (x, y) in [(40, 9), (17, 32), (60, 32), (40, 54)] {
            let px = read_pixel(&context, &texture, &staging, x, y)
                .unwrap_or_else(|err| panic!("readback of ({x}, {y}) failed: {err}"));
            assert_eq!(
                px, red_px,
                "translated border quad must have a border pixel at ({x}, {y})"
            );
        }
        let interior = read_pixel(&context, &texture, &staging, 40, 32)
            .expect("readback of interior failed");
        assert_eq!(
            interior, clear,
            "a border-only quad's interior stays transparent"
        );

        // Case 2: solid rounded quad whose content mask is its own rounded rect, translated by
        // 8px. The trailing edge past the untransformed rect must survive: pre-fix the mask cut
        // everything with scene x > 56, which is exactly pixel (60, 32).
        let own_cutout_quad = Quad {
            bounds: quad_bounds,
            content_mask: ContentMask {
                bounds: quad_bounds,
                corner_radii: rounded_corners,

                fade_out: Default::default(),
            },
            background: Background::from(rgba(0xff0000ff)),
            corner_radii: rounded_corners,
            transformation: translate,
            ..Default::default()
        };
        draw_quad(&mut pipeline, own_cutout_quad)?;
        for (what, x, y) in [("right band", 60, 32), ("rounded corner", 58, 14)] {
            let px = read_pixel(&context, &texture, &staging, x, y)
                .unwrap_or_else(|err| panic!("readback of ({x}, {y}) failed: {err}"));
            assert_eq!(
                px, red_px,
                "translated own-cutout quad lost its {what} at ({x}, {y})"
            );
        }

        // Case 3: the same quad under a center-pivoted skewX(0.3). Pixel (58, 46) sits inside
        // the skewed shape's lower-right region, past the untransformed mask rect the pre-fix
        // code cut at.
        let t = 0.3f32.tan();
        let skew = TransformationMatrix {
            rotation_scale: [[1.0, t], [0.0, 1.0]],
            translation: [-t * 32.0, 0.0],
        };
        let skew_quad = Quad {
            bounds: quad_bounds,
            content_mask: ContentMask {
                bounds: quad_bounds,
                corner_radii: rounded_corners,

                fade_out: Default::default(),
            },
            background: Background::from(rgba(0xff0000ff)),
            corner_radii: rounded_corners,
            transformation: skew,
            ..Default::default()
        };
        draw_quad(&mut pipeline, skew_quad)?;
        for (x, y) in [(58, 46), (40, 32)] {
            let px = read_pixel(&context, &texture, &staging, x, y)
                .unwrap_or_else(|err| panic!("readback of ({x}, {y}) failed: {err}"));
            assert_eq!(px, red_px, "skewed own-cutout quad missing at ({x}, {y})");
        }

        // Case 4: border-only quad rotated 22.5° about its center. The border must follow the
        // rotated edges: probe points are band midpoints of the untransformed border, mapped
        // through the same matrix the shader applies.
        let rotation = {
            let center = ScaledPixels(32.0);
            TransformationMatrix::unit()
                .translate(point(center, center))
                .compose(TransformationMatrix::unit().rotate(hgpui::radians(0.3927)))
                .translate(point(ScaledPixels(-32.0), ScaledPixels(-32.0)))
        };
        let rotated_border = Quad {
            bounds: quad_bounds,
            content_mask: ContentMask {
                bounds: bounds(
                    point(ScaledPixels(0.0), ScaledPixels(0.0)),
                    size(ScaledPixels(64.0), ScaledPixels(64.0)),
                ),
                corner_radii: Corners::default(),

                fade_out: Default::default(),
            },
            background: Background::from(transparent_black()),
            border_color: red().into(),
            border_widths: Edges {
                top: ScaledPixels(4.0),
                right: ScaledPixels(4.0),
                bottom: ScaledPixels(4.0),
                left: ScaledPixels(4.0),
            },
            transformation: rotation,
            ..Default::default()
        };
        draw_quad(&mut pipeline, rotated_border)?;
        for local in [
            point(px(10.0), px(32.0)),
            point(px(54.0), px(32.0)),
            point(px(32.0), px(10.0)),
            point(px(32.0), px(54.0)),
        ] {
            let scene = rotation.apply(local);
            let (x, y) = (
                f32::from(scene.x).floor() as u32,
                f32::from(scene.y).floor() as u32,
            );
            let px_color = read_pixel(&context, &texture, &staging, x, y)
                .unwrap_or_else(|err| panic!("readback of ({x}, {y}) failed: {err}"));
            assert_eq!(
                px_color, red_px,
                "rotated border quad missing its border at local {local:?} (scene ({x}, {y}))"
            );
        }

        Ok(())
    }

    /// The checkerboard pattern must antialias its cell edges: with the pattern offset by half
    /// a cell, every vertical cell boundary sits on a pixel center, so a row scan across the
    /// boundary must contain blended pixels instead of a binary on/off staircase.
    #[test]
    fn hlsl_checkerboard_edge_aa_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[checker-aa] WARP device unavailable, skipping");
            return Ok(());
        };
        let (texture, rtv, staging) = make_render_target(&device)?;
        let globals = install_globals(&device, &context)?;
        let blend = create_blend_state(&device)?;
        let mut pipeline =
            PipelineState::<Quad>::new(&device, "test_checker_aa", ShaderModule::Quad, 4, blend)?;

        // Half-pixel offset puts the cell boundaries on pixel centers (x = 16.5 + 8k).
        let quad = Quad {
            bounds: bounds(
                point(ScaledPixels(16.5), ScaledPixels(16.5)),
                size(ScaledPixels(31.0), ScaledPixels(31.0)),
            ),
            content_mask: ContentMask {
                bounds: bounds(
                    point(ScaledPixels(0.0), ScaledPixels(0.0)),
                    size(ScaledPixels(64.0), ScaledPixels(64.0)),
                ),
                corner_radii: Corners::default(),

                fade_out: Default::default(),
            },
            background: hgpui::checkerboard(rgba(0xff0000ff), 8.0),
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
            context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv.clone()))), None);
        }
        pipeline.draw_range(
            &device,
            &context,
            &[viewport],
            &globals,
            4,
            0,
            1,
        )?;
        dump_device_errors(&device);

        let blended = (0..TEST_SIZE)
            .filter(|&x| {
                let r = read_pixel(&context, &texture, &staging, x, 33).unwrap()[2];
                (72..=248).contains(&r)
            })
            .count();
        assert!(
            blended == 0,
            "upstream checkerboard has hard cell boundaries (found {blended} blended pixels)"
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
        globals: &[Option<ID3D11Buffer>; 4],
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
        let params_buffer = Some(params_buffer.clone());
        let sampler = Some(sampler.clone());
        let constant_buffers = [
            globals[0].clone(),
            globals[1].clone(),
            params_buffer,
            globals[3].clone(),
        ];
        unsafe {
            // Unbind the source slot before setting the target: a texture that is still bound as
            // output (the target of the previous pass) cannot be bound as input at the same time.
            let null_srv: [Option<ID3D11ShaderResourceView>; 1] = [None];
            context.PSSetShaderResources(3, Some(&null_srv));
            context.OMSetRenderTargets(Some(slice::from_ref(&rtv)), None);
            context.RSSetViewports(Some(slice::from_ref(viewport)));
            context.IASetPrimitiveTopology(topology);
            context.VSSetShader(vertex, None);
            context.PSSetShader(fragment, None);
            context.VSSetConstantBuffers(0, Some(&constant_buffers));
            context.PSSetConstantBuffers(0, Some(&constant_buffers));
            context.PSSetSamplers(4, Some(slice::from_ref(&sampler)));
            context.OMSetBlendState(blend, None, 0xFFFFFFFF);
            context.PSSetShaderResources(3, Some(slice::from_ref(&srv)));
            context.DrawInstanced(vertex_count, 1, 0, 0);
            context.PSSetShaderResources(3, Some(&null_srv));
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

        let globals = test_globals(&device, &context)?;
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
                &globals,
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
                    &globals,
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
                &globals,
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

            fade_out: Default::default(),
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

                fade_out: Default::default(),
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

        let globals = install_globals(&device, &context)?;

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

                fade_out: Default::default(),
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
        pipeline.draw_range(&device, &context, &[viewport], &globals, 4, 0, 1)?;
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

        let globals = install_globals(&device, &context)?;

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

                fade_out: Default::default(),
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
        pipeline.draw_range(&device, &context, &[viewport], &globals, 4, 0, 1)?;
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

        let globals = install_globals(&device, &context)?;

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

                fade_out: Default::default(),
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
        pipeline.draw_range(&device, &context, &[viewport], &globals, 4, 0, 1)?;
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

        let globals = install_globals(&device, &context)?;

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

                    fade_out: Default::default(),
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
                &globals,
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

                fade_out: Default::default(),
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
        pipeline.draw_range(&device, &context, &[viewport], &globals, 4, 0, 1)?;
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

        let globals = install_globals(&device, &context)?;

        let blend = create_blend_state(&device)?;
        let mut pipeline =
            PipelineState::<Quad>::new(&device, "test_two_instances", ShaderModule::Quad, 4, blend)?;

        let content_mask = ContentMask {
            bounds: bounds(
                point(ScaledPixels(0.0), ScaledPixels(0.0)),
                size(ScaledPixels(64.0), ScaledPixels(64.0)),
            ),
            corner_radii: Corners::default(),

            fade_out: Default::default(),
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
        pipeline.draw_range(&device, &context, &[viewport], &globals, 4, 0, 2)?;
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

    /// The path instance struct must match the HLSL `PathRasterizationSprite` offsets exactly
    /// (FXC reports 0/8/16/96/112/128, total 144). A drift here silently misreads the mask, the
    /// gradient box and the clip chain for every path vertex.
    #[test]
    fn path_rasterization_sprite_layout() {
        use std::mem::{offset_of, size_of};
        assert_eq!(size_of::<PathRasterizationSprite>(), 144);
        assert_eq!(offset_of!(PathRasterizationSprite, xy_position), 0);
        assert_eq!(offset_of!(PathRasterizationSprite, curve_position), 8);
        assert_eq!(offset_of!(PathRasterizationSprite, color), 16);
        assert_eq!(offset_of!(PathRasterizationSprite, bounds), 96);
        assert_eq!(offset_of!(PathRasterizationSprite, content_mask), 112);
        assert_eq!(offset_of!(PathRasterizationSprite, mask_chain), 128);
        assert_eq!(offset_of!(PathRasterizationSprite, padding), 132);
    }

    // --- Antialiasing / clipping precision tests -------------------------------------------
    //
    // Every test below drives the real HLSL entry point through WARP and reads the pixels
    // back. They pin the behaviours the precision pass fixed:
    //   * straight edges of quads/underlines now use their distance field (the early-out no
    //     longer fires for the whole interior, and AA_MARGIN gives the ramp fragments outside
    //     the shape),
    //   * mask edges are shaped by the mask's distance field on both sides of straight edges,
    //   * vector paths honour a rounded content mask and resolve gradients against their own
    //     bounds,
    //   * closest-side/corner radial gradients work with an off-box centre,
    //   * transparent-over-transparent and zero-width dashes no longer produce NaN.

    fn test_viewport() -> D3D11_VIEWPORT {
        D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: TEST_SIZE as f32,
            Height: TEST_SIZE as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }
    }

    /// Constant buffers shared by the shader-level WARP tests: globals (b0), font (b1), an empty
    /// blur slot (b2), and Naga's draw constants (b3).
    fn test_globals(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
    ) -> Result<[Option<ID3D11Buffer>; 4]> {
        let globals_buffer =
            create_constant_buffer(device, std::mem::size_of::<GlobalUniforms>())?;
        let font_buffer =
            create_constant_buffer(device, std::mem::size_of::<FontRasterizationUniforms>())?;
        let draw_constants_buffer = create_constant_buffer(device, 16)?;
        update_buffer(
            context,
            &globals_buffer,
            &[GlobalUniforms {
                viewport_size: [TEST_SIZE as f32, TEST_SIZE as f32],
                premultiplied_alpha: 0,
                padding: 0,
                window_mask_params: [0.0; 4],
                window_mask: ContentMask::default(),
            }],
        )?;
        update_buffer(
            context,
            &font_buffer,
            &[FontRasterizationUniforms {
                gamma_ratios: [1.0, 1.0, 1.0, 1.0],
                grayscale_enhanced_contrast: 0.0,
                subpixel_enhanced_contrast: 0.0,
                uses_blue_green_red_subpixel_order: 0,
                padding: 0,
            }],
        )?;
        update_buffer(
            context,
            &draw_constants_buffer,
            &[hgpui_render::artifacts::Dx11DrawConstants::default()],
        )?;
        Ok([
            Some(globals_buffer),
            Some(font_buffer),
            None,
            Some(draw_constants_buffer),
        ])
    }

    fn install_globals(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
    ) -> Result<[Option<ID3D11Buffer>; 4]> {
        test_globals(device, context)
    }

    fn viewport_mask() -> ContentMask<ScaledPixels> {
        ContentMask {
            bounds: bounds(
                point(ScaledPixels(0.0), ScaledPixels(0.0)),
                size(ScaledPixels(TEST_SIZE as f32), ScaledPixels(TEST_SIZE as f32)),
            ),
            corner_radii: Corners::default(),

            fade_out: Default::default(),
        }
    }

    /// A drawn test target plus the resources that must stay alive while the queued draw is
    /// still in flight.
    struct QuadTarget {
        texture: ID3D11Texture2D,
        staging: ID3D11Texture2D,
        _pipeline: PipelineState<Quad>,
        _globals: [Option<ID3D11Buffer>; 4],
        _rtv: ID3D11RenderTargetView,
        _mask_buffer: Option<ID3D11Buffer>,
        _mask_view: Option<ID3D11ShaderResourceView>,
    }

    fn render_quad(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        quad: Quad,
        clear: [f32; 4],
    ) -> Result<QuadTarget> {
        let (texture, rtv, staging) = make_render_target(device)?;
        let globals = install_globals(device, context)?;
        let blend = create_blend_state(device)?;
        let mut pipeline =
            PipelineState::<Quad>::new(device, "test_quad_precision", ShaderModule::Quad, 4, blend)?;
        pipeline.update_buffer(device, context, &[quad])?;
        unsafe {
            context.ClearRenderTargetView(&rtv, &clear);
            context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv.clone()))), None);
        }
        pipeline.draw_range(device, context, &[test_viewport()], &globals, 4, 0, 1)?;
        dump_device_errors(device);
        Ok(QuadTarget {
            texture,
            staging,
            _pipeline: pipeline,
            _globals: globals,
            _rtv: rtv,
            _mask_buffer: None,
            _mask_view: None,
        })
    }

    /// Renders a quad with a real content-mask chain bound at `t5`, exactly like the production
    /// renderer (the chain arena is one raw structured buffer indexed by each instance).
    fn render_quad_with_mask_chain(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        quad: Quad,
        masks: &[ContentMask<ScaledPixels>],
        clear: [f32; 4],
    ) -> Result<QuadTarget> {
        let (texture, rtv, staging) = make_render_target(device)?;
        let globals = install_globals(device, context)?;
        let blend = create_blend_state(device)?;
        let mut pipeline =
            PipelineState::<Quad>::new(device, "test_mask_chain", ShaderModule::Quad, 4, blend)?;
        pipeline.update_buffer(device, context, &[quad])?;

        let mask_buffer = create_buffer(
            device,
            std::mem::size_of::<ContentMask<ScaledPixels>>(),
            masks.len().max(1),
        )?;
        update_buffer(context, &mask_buffer, masks)?;
        let mask_view = create_buffer_view(device, &mask_buffer)?;

        unsafe {
            context.ClearRenderTargetView(&rtv, &clear);
            context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv.clone()))), None);
            context.PSSetShaderResources(5, Some(slice::from_ref(&mask_view)));
            context.VSSetShaderResources(5, Some(slice::from_ref(&mask_view)));
        }
        pipeline.draw_range(device, context, &[test_viewport()], &globals, 4, 0, 1)?;
        dump_device_errors(device);
        Ok(QuadTarget {
            texture,
            staging,
            _pipeline: pipeline,
            _globals: globals,
            _rtv: rtv,
            _mask_buffer: Some(mask_buffer),
            _mask_view: mask_view,
        })
    }

    fn assert_red_close(px: [u8; 4], expected: u8, what: &str) {
        assert!(
            (i16::from(expected) - 3..=i16::from(expected) + 3).contains(&i16::from(px[2])),
            "{what}: expected red ~{expected}, got {px:?} (B,G,R,A)"
        );
    }

    /// A quad whose edges land on fractional device pixels must taper over one pixel on every
    /// straight edge. The old early-out returned coverage 1.0 for every *interior* fragment
    /// (its condition reduced to `corner_to_point < 0`), so only the arcs were antialiased;
    /// the margin then gives the ramp a fragment outside the shape as well.
    #[test]
    fn hlsl_quad_fractional_straight_edge_aa_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[quad_fractional_aa] WARP device unavailable, skipping");
            return Ok(());
        };
        // [16.25, 48.75] on both axes: every edge covers 0.75 of its boundary pixel.
        let quad_bounds = bounds(
            point(ScaledPixels(16.25), ScaledPixels(16.25)),
            size(ScaledPixels(32.5), ScaledPixels(32.5)),
        );
        let quad = Quad {
            bounds: quad_bounds,
            // Upstream antialiases a plain quad through its content mask.
            content_mask: ContentMask {
                bounds: quad_bounds,
                corner_radii: Corners::default(),
                fade_out: Default::default(),
            },
            background: Background::from(rgba(0xff0000ff)),
            ..Default::default()
        };
        let target = render_quad(&device, &context, quad, [0.0, 0.0, 0.0, 1.0])?;

        for (x, y, what) in [
            (16, 32, "left edge"),
            (48, 32, "right edge"),
            (32, 16, "top edge"),
            (32, 48, "bottom edge"),
        ] {
            let px = read_pixel(&context, &target.texture, &target.staging, x, y)?;
            // 0.75 * 255 = 191; the ramp is exact for a straight edge at unit scale.
            assert_red_close(px, 191, what);
        }
        for (x, y) in [(15, 32), (49, 32), (32, 15), (32, 49)] {
            let px = read_pixel(&context, &target.texture, &target.staging, x, y)?;
            assert_eq!(px[2], 0, "pixel ({x}, {y}) outside the quad must stay clear, got {px:?}");
        }
        let center = read_pixel(&context, &target.texture, &target.staging, 32, 32)?;
        assert_eq!(center[2], 255, "the interior must be fully covered, got {center:?}");
        Ok(())
    }

    /// The coverage ramp must stay one *device* pixel wide under a CSS scale. A scale of 0.25
    /// maps one device pixel to four local units; the old local-unit early-out fired at one
    /// local unit (0.25 device pixels) inside the edge and forced full coverage there, leaving
    /// a hard step inside the antialiased band.
    #[test]
    fn hlsl_quad_scaled_straight_edge_aa_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[quad_scaled_aa] WARP device unavailable, skipping");
            return Ok(());
        };
        // local [8,56] -> scene [14.3, 26.3] on both axes.
        let quad = Quad {
            bounds: bounds(
                point(ScaledPixels(8.0), ScaledPixels(8.0)),
                size(ScaledPixels(48.0), ScaledPixels(48.0)),
            ),
            // The mask is in scene space; it must cover the transformed quad.
            content_mask: ContentMask {
                bounds: bounds(
                    point(ScaledPixels(14.3), ScaledPixels(14.3)),
                    size(ScaledPixels(12.0), ScaledPixels(12.0)),
                ),
                corner_radii: Corners::default(),
                fade_out: Default::default(),
            },
            background: Background::from(rgba(0xff0000ff)),
            transformation: TransformationMatrix {
                rotation_scale: [[0.25, 0.0], [0.0, 0.25]],
                translation: [12.3, 12.3],
            },
            ..Default::default()
        };
        let target = render_quad(&device, &context, quad, [0.0, 0.0, 0.0, 1.0])?;

        // On the left/top the boundary pixel's centre is 0.2 device px inside -> coverage
        // 0.7 -> 179. On the right/bottom the centre is 0.2 px outside -> coverage 0.3 -> 77;
        // both need the margin (an outside fragment) to exist at all.
        for (x, y, expected, what) in [
            (14, 20, 179, "scaled left edge"),
            (26, 20, 77, "scaled right edge"),
            (20, 14, 179, "scaled top edge"),
            (20, 26, 77, "scaled bottom edge"),
        ] {
            let px = read_pixel(&context, &target.texture, &target.staging, x, y)?;
            assert_red_close(px, expected, what);
        }
        let inside = read_pixel(&context, &target.texture, &target.staging, 20, 20)?;
        assert_eq!(inside[2], 255, "scaled interior must be fully covered, got {inside:?}");
        let outside = read_pixel(&context, &target.texture, &target.staging, 13, 20)?;
        assert_eq!(outside[2], 0, "scaled left outside must stay clear, got {outside:?}");
        Ok(())
    }

    /// A straight content-mask edge on a half pixel must taper over one pixel, exactly like a
    /// rounded arc. The clip is padded by AA_MARGIN and the fragment applies the mask's distance
    /// field for unrounded masks too; before this, the exact clip plus the "unrounded mask =>
    /// coverage 1" short-circuit made every straight mask edge a hard step.
    #[test]
    fn hlsl_quad_mask_straight_edge_aa_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[mask_straight_aa] WARP device unavailable, skipping");
            return Ok(());
        };
        let quad = Quad {
            bounds: bounds(
                point(ScaledPixels(8.0), ScaledPixels(8.0)),
                size(ScaledPixels(48.0), ScaledPixels(48.0)),
            ),
            // The unrounded mask's right edge sits on the centre of pixel 40.
            content_mask: ContentMask {
                bounds: bounds(
                    point(ScaledPixels(8.0), ScaledPixels(8.0)),
                    size(ScaledPixels(32.5), ScaledPixels(48.0)),
                ),
                corner_radii: Corners::default(),

                fade_out: Default::default(),
            },
            background: Background::from(rgba(0xff0000ff)),
            ..Default::default()
        };
        let target = render_quad(&device, &context, quad, [0.0, 0.0, 0.0, 1.0])?;

        let edge = read_pixel(&context, &target.texture, &target.staging, 40, 32)?;
        assert_red_close(edge, 128, "half-covered mask edge");
        let outside = read_pixel(&context, &target.texture, &target.staging, 41, 32)?;
        assert_eq!(outside[2], 0, "pixel past the mask must be clipped, got {outside:?}");
        let inside = read_pixel(&context, &target.texture, &target.staging, 39, 32)?;
        assert_eq!(inside[2], 255, "pixel inside the mask must be solid, got {inside:?}");
        Ok(())
    }

    /// A wavy-free underline's ends must taper: the geometry is grown by AA_MARGIN and the
    /// fragment cuts the extra margin with the underline's own rectangle.
    #[test]
    fn hlsl_underline_end_aa_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[underline_end_aa] WARP device unavailable, skipping");
            return Ok(());
        };
        let (texture, rtv, staging) = make_render_target(&device)?;
        let globals = install_globals(&device, &context)?;
        let blend = create_blend_state(&device)?;
        let mut pipeline = PipelineState::<hgpui::Underline>::new(
            &device,
            "test_underline_precision",
            ShaderModule::Underline,
            4,
            blend,
        )?;
        let underline_bounds = bounds(
            point(ScaledPixels(16.5), ScaledPixels(16.0)),
            size(ScaledPixels(31.0), ScaledPixels(32.0)),
        );
        let underline = hgpui::Underline {
            order: 0,
            mask_chain: 0,
            bounds: underline_bounds,
            content_mask: ContentMask {
                bounds: underline_bounds,
                corner_radii: Corners::default(),
                fade_out: Default::default(),
            },
            color: red().into(),
            thickness: ScaledPixels(0.0),
            wavy: false.into(),
            transformation: TransformationMatrix::unit(),
            padding: [0; 2],
        };
        pipeline.update_buffer(&device, &context, &[underline])?;
        unsafe {
            context.ClearRenderTargetView(&rtv, &[0.0, 0.0, 0.0, 1.0]);
            context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv.clone()))), None);
        }
        pipeline.draw_range(&device, &context, &[test_viewport()], &globals, 4, 0, 1)?;
        dump_device_errors(&device);

        let left = read_pixel(&context, &texture, &staging, 16, 32)?;
        assert_red_close(left, 128, "underline left end");
        let right = read_pixel(&context, &texture, &staging, 47, 32)?;
        assert_red_close(right, 128, "underline right end");
        let middle = read_pixel(&context, &texture, &staging, 32, 32)?;
        assert_eq!(middle[2], 255, "underline body must be solid, got {middle:?}");
        Ok(())
    }

    /// A polychrome sprite's own corner SDF must taper on fractional edges. The quad is grown
    /// by AA_MARGIN and the tile lookup is clamped with `saturate`, so the extra fragments
    /// replay the edge texel with partial coverage instead of a hard snapped edge.
    #[test]
    fn hlsl_polychrome_sprite_fractional_edge_aa_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[poly_edge_aa] WARP device unavailable, skipping");
            return Ok(());
        };
        let (texture, rtv, staging) = make_render_target(&device)?;
        let globals = install_globals(&device, &context)?;
        let blend = create_blend_state(&device)?;
        let mut pipeline = PipelineState::<PolychromeSprite>::new(
            &device,
            "test_poly_precision",
            ShaderModule::PolychromeSprite,
            64,
            blend,
        )?;

        // A 4x4 opaque white atlas tile.
        let pixels = [255u8; 4 * 4 * 4];
        let atlas_srv = unsafe {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: 4,
                Height: 4,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let data = D3D11_SUBRESOURCE_DATA {
                pSysMem: pixels.as_ptr().cast(),
                SysMemPitch: 4 * 4,
                SysMemSlicePitch: 0,
            };
            let mut atlas = None;
            device.CreateTexture2D(&desc, Some(&data), Some(&mut atlas))?;
            let mut srv = None;
            device.CreateShaderResourceView(&atlas.unwrap(), None, Some(&mut srv))?;
            srv.unwrap()
        };

        let sprite = PolychromeSprite {
            order: 0,
            grayscale: false.into(),
            opacity: 1.0,
            corner_smoothing: 0.0,
            // [16.25, 48.75]: each edge covers 0.75 of its boundary pixel.
            bounds: bounds(
                point(ScaledPixels(16.25), ScaledPixels(16.25)),
                size(ScaledPixels(32.5), ScaledPixels(32.5)),
            ),
            content_mask: viewport_mask(),
            corner_radii: Corners::default(),
            tile: AtlasTile {
                texture_id: AtlasTextureId {
                    index: 0,
                    kind: AtlasTextureKind::Polychrome,
                },
                tile_id: TileId(0),
                padding: 0,
                bounds: bounds(
                    point(DevicePixels(0), DevicePixels(0)),
                    size(DevicePixels(4), DevicePixels(4)),
                ),
            },
            transformation: TransformationMatrix::unit(),
            mask_chain: 0,
            padding1: 0,
        };
        pipeline.update_buffer(&device, &context, &[sprite])?;
        let sampler = create_blur_sampler(&device)?;
        let atlas_binding: Option<ID3D11ShaderResourceView> = Some(atlas_srv.clone());
        unsafe {
            context.ClearRenderTargetView(&rtv, &[0.0, 0.0, 0.0, 1.0]);
            context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv.clone()))), None);
            context.PSSetSamplers(4, Some(slice::from_ref(&sampler)));
            context.PSSetShaderResources(3, Some(slice::from_ref(&atlas_binding)));
        }
        pipeline.draw(
            &context,
            &[test_viewport()],
            &globals,
            D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
            4,
            1,
        )?;
        dump_device_errors(&device);

        for (x, y, what) in [
            (16, 32, "left edge"),
            (48, 32, "right edge"),
            (32, 16, "top edge"),
            (32, 48, "bottom edge"),
        ] {
            let px = read_pixel(&context, &texture, &staging, x, y)?;
            assert_red_close(px, 191, what);
        }
        let center = read_pixel(&context, &texture, &staging, 32, 32)?;
        assert_eq!(center, [255, 255, 255, 255], "sprite body must be white, got {center:?}");
        let outside = read_pixel(&context, &texture, &staging, 15, 32)?;
        assert_eq!(outside, [0, 0, 0, 255], "sprite must not paint outside, got {outside:?}");
        Ok(())
    }

    /// Creates an RGBA texture and its shader view from raw bytes.
    fn create_rgba_texture(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> Result<ID3D11ShaderResourceView> {
        unsafe {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let data = D3D11_SUBRESOURCE_DATA {
                pSysMem: pixels.as_ptr().cast(),
                SysMemPitch: width * 4,
                SysMemSlicePitch: 0,
            };
            let mut texture = None;
            device.CreateTexture2D(&desc, Some(&data), Some(&mut texture))?;
            let mut srv = None;
            device.CreateShaderResourceView(&texture.unwrap(), None, Some(&mut srv))?;
            Ok(srv.unwrap())
        }
    }

    /// The tile lookup must stay registered to the sprite's original bounds and must stay
    /// inside its own texels. The quad is grown by `AA_MARGIN`, so driving UVs from the vertex'
    /// unit position stretches the image across the grown quad; and bilinear taps within half a
    /// texel of the tile border otherwise read the neighbouring atlas tile, which is what shows
    /// up as dark fringes around images and as stray strokes beside glyphs scaled by a CSS
    /// transform.
    #[test]
    fn hlsl_polychrome_sprite_tile_sampling_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[poly_tile] WARP device unavailable, skipping");
            return Ok(());
        };
        let globals = install_globals(&device, &context)?;
        let blend = create_blend_state(&device)?;
        let mut pipeline = PipelineState::<PolychromeSprite>::new(
            &device,
            "test_poly_tile",
            ShaderModule::PolychromeSprite,
            64,
            blend,
        )?;
        let sampler = create_blur_sampler(&device)?;
        let (texture, rtv, staging) = make_render_target(&device)?;

        let draw = |pipeline: &mut PipelineState<PolychromeSprite>,
                    srv: &ID3D11ShaderResourceView,
                    sprite: PolychromeSprite|
         -> Result<()> {
            pipeline.update_buffer(&device, &context, &[sprite])?;
            let atlas_binding: Option<ID3D11ShaderResourceView> = Some(srv.clone());
            unsafe {
                context.ClearRenderTargetView(&rtv, &[0.0, 0.0, 0.0, 1.0]);
                context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv.clone()))), None);
                context.PSSetSamplers(4, Some(slice::from_ref(&sampler)));
                context.PSSetShaderResources(3, Some(slice::from_ref(&atlas_binding)));
            }
            pipeline.draw(
                &context,
                &[test_viewport()],
                &globals,
                D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
                4,
                1,
            )?;
            dump_device_errors(&device);
            Ok(())
        };

        // Case 1: an 8x8 tile at the atlas origin, blue except for a red column at x = 2.
        // Drawn 1:1 into [8, 8, 8x8], texel `i` covers scene x = 8 + i .. 9 + i, so pixel
        // (9, 12) must stay blue and pixel (10, 12) must be red. Stretching the tile over the
        // AA-grown quad shifts the red column left into pixel 9.
        let mut registration_pixels = vec![0u8; 16 * 16 * 4];
        for y in 0..16 {
            for x in 0..16 {
                let i = (y * 16 + x) * 4;
                // R8G8B8A8 memory order.
                let color = if x == 2 {
                    [255, 0, 0, 255] // red
                } else {
                    [0, 0, 255, 255] // blue
                };
                registration_pixels[i..i + 4].copy_from_slice(&color);
            }
        }
        let registration_srv = create_rgba_texture(&device, 16, 16, &registration_pixels)?;
        let sprite = |sprite_bounds: Bounds<ScaledPixels>, tile_origin: (i32, i32)| PolychromeSprite {
            order: 0,
            grayscale: false.into(),
            opacity: 1.0,
            corner_smoothing: 0.0,
            bounds: sprite_bounds,
            content_mask: viewport_mask(),
            corner_radii: Corners::default(),
            tile: AtlasTile {
                texture_id: AtlasTextureId {
                    index: 0,
                    kind: AtlasTextureKind::Polychrome,
                },
                tile_id: TileId(0),
                padding: 0,
                bounds: bounds(
                    point(DevicePixels(tile_origin.0), DevicePixels(tile_origin.1)),
                    size(DevicePixels(8), DevicePixels(8)),
                ),
            },
            transformation: TransformationMatrix::unit(),
            mask_chain: 0,
            padding1: 0,
        };
        draw(
            &mut pipeline,
            &registration_srv,
            sprite(
                bounds(
                    point(ScaledPixels(8.0), ScaledPixels(8.0)),
                    size(ScaledPixels(8.0), ScaledPixels(8.0)),
                ),
                (0, 0),
            ),
        )?;
        let texel_one = read_pixel(&context, &texture, &staging, 9, 12)?;
        assert_eq!(
            texel_one,
            [255, 0, 0, 255],
            "texel 1 must stay blue (B,G,R,A); the tile was stretched by the AA margin: {texel_one:?}"
        );
        let texel_two = read_pixel(&context, &texture, &staging, 10, 12)?;
        assert_eq!(
            texel_two,
            [0, 0, 255, 255],
            "texel 2 must be red (B,G,R,A) at its true position: {texel_two:?}"
        );

        // Case 2: the tile sits at (1, 1) with a green atlas background around it. At a
        // fractional origin/scale a bilinear tap one texel outside the tile would read the
        // green surround; clamped tile coordinates must replay the tile's own edge texel.
        let mut bleed_pixels = vec![0u8; 16 * 16 * 4];
        for y in 0..16 {
            for x in 0..16 {
                let i = (y * 16 + x) * 4;
                let color = if (1..9).contains(&x) && (1..9).contains(&y) {
                    [255, 255, 255, 255] // white tile
                } else {
                    [0, 255, 0, 255] // green surround
                };
                bleed_pixels[i..i + 4].copy_from_slice(&color);
            }
        }
        let bleed_srv = create_rgba_texture(&device, 16, 16, &bleed_pixels)?;
        draw(
            &mut pipeline,
            &bleed_srv,
            sprite(
                bounds(
                    point(ScaledPixels(20.5), ScaledPixels(20.5)),
                    size(ScaledPixels(32.25), ScaledPixels(32.25)),
                ),
                (1, 1),
            ),
        )?;
        for (x, y, what) in [
            (21, 32, "left edge"),
            (52, 32, "right edge"),
            (32, 21, "top edge"),
            (32, 52, "bottom edge"),
        ] {
            let px = read_pixel(&context, &texture, &staging, x, y)?;
            let (b, g, r) = (px[0] as i32, px[1] as i32, px[2] as i32);
            assert!(
                (g - r).abs() <= 8 && (g - b).abs() <= 8,
                "{what} sampled the neighbouring atlas tile ({px:?} B,G,R,A)"
            );
        }

        Ok(())
    }

    /// A rounded parent content mask must clip on its arc, not on the mask's bounding box: the
    /// corner region outside the arc stays clear, the mask's straight edges keep their one-pixel
    /// ramp, and the interior is untouched.
    #[test]
    fn hlsl_quad_rounded_content_mask_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[rounded_mask] WARP device unavailable, skipping");
            return Ok(());
        };
        let radius = ScaledPixels(8.0);
        let quad = Quad {
            bounds: bounds(
                point(ScaledPixels(0.0), ScaledPixels(0.0)),
                size(ScaledPixels(64.0), ScaledPixels(64.0)),
            ),
            content_mask: ContentMask {
                bounds: bounds(
                    point(ScaledPixels(8.25), ScaledPixels(8.25)),
                    size(ScaledPixels(47.5), ScaledPixels(47.5)),
                ),
                corner_radii: Corners {
                    top_left: radius,
                    top_right: radius,
                    bottom_right: radius,
                    bottom_left: radius,
                },
                fade_out: Default::default(),
            },
            background: Background::from(rgba(0xff0000ff)),
            ..Default::default()
        };
        let target = render_quad(&device, &context, quad, [0.0, 0.0, 0.0, 1.0])?;

        let corner = read_pixel(&context, &target.texture, &target.staging, 9, 9)?;
        assert_eq!(
            corner[2], 0,
            "the mask corner arc must clip the pixel outside the arc, got {corner:?}"
        );
        let inside_corner = read_pixel(&context, &target.texture, &target.staging, 13, 13)?;
        assert_eq!(
            inside_corner[2], 255,
            "the mask interior must stay opaque, got {inside_corner:?}"
        );
        let edge = read_pixel(&context, &target.texture, &target.staging, 8, 32)?;
        assert_red_close(edge, 191, "rounded-mask straight edge");
        let center = read_pixel(&context, &target.texture, &target.staging, 32, 32)?;
        assert_eq!(center[2], 255, "mask center must stay opaque, got {center:?}");
        Ok(())
    }

    /// A rounded parent must keep clipping a child that is inset from it.
    ///
    /// Intersecting two rounded rectangles on the CPU produces one rounded rectangle and loses
    /// the parent's arc as soon as the child is inset: the child's corner no longer coincides
    /// with the parent's, so `ContentMask::intersect` leaves the shared corner square and content
    /// leaks through the parent's rounded corner. Primitives therefore carry the *raw* ancestor
    /// chain; the fragment shader intersects the distance fields instead of the rectangles.
    #[test]
    fn hlsl_mask_chain_nested_rounded_parent_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[mask_chain] WARP device unavailable, skipping");
            return Ok(());
        };
        let quad_bounds = bounds(
            point(ScaledPixels(0.0), ScaledPixels(0.0)),
            size(ScaledPixels(64.0), ScaledPixels(64.0)),
        );

        // Parent: 4..60 rounded by 16. Child: 5..59, square. The child's corner at (5, 5) is
        // ~5 px outside the parent (TL circle centre (20, 20), radius 16). A single intersected
        // mask would be the square child, so (6, 6) would leak; the chain must clip it.
        let parent = ContentMask {
            bounds: bounds(
                point(ScaledPixels(4.0), ScaledPixels(4.0)),
                size(ScaledPixels(56.0), ScaledPixels(56.0)),
            ),
            corner_radii: Corners {
                top_left: ScaledPixels(16.0),
                top_right: ScaledPixels(16.0),
                bottom_right: ScaledPixels(16.0),
                bottom_left: ScaledPixels(16.0),
            },
            fade_out: Default::default(),
        };
        let child = ContentMask {
            bounds: bounds(
                point(ScaledPixels(5.0), ScaledPixels(5.0)),
                size(ScaledPixels(54.0), ScaledPixels(54.0)),
            ),
            corner_radii: Corners::default(),
            fade_out: Default::default(),
        };
        let quad = Quad {
            bounds: quad_bounds,
            // The lossy single mask is what the pre-chain code would have used.
            content_mask: child,
            mask_chain: (2 << 24) | 0,
            background: Background::from(rgba(0xff0000ff)),
            ..Default::default()
        };
        let target = render_quad_with_mask_chain(
            &device,
            &context,
            quad,
            &[parent, child],
            [0.0, 0.0, 0.0, 1.0],
        )?;
        for (x, y, expected, what) in [
            (6, 6, 0, "inset child corner must be cut by the parent's arc"),
            (10, 10, 255, "inside the parent arc must stay opaque"),
            (32, 32, 255, "interior must stay opaque"),
            (6, 58, 0, "bottom-left parent arc must clip the child"),
            (58, 58, 0, "bottom-right parent arc must clip the child"),
            (4, 32, 0, "child's left edge must still clip"),
        ] {
            let px = read_pixel(&context, &target.texture, &target.staging, x, y)?;
            assert_eq!(
                px[2], expected,
                "{what} at ({x}, {y}), got {px:?} (B,G,R,A)"
            );
        }

        // Same bounds, parent radius 16 and child radius 8: the larger cut governs, so the
        // parent's arc must win even though the child's radius is smaller. A "smallest radius
        // wins" intersection would leave (9, 15) red.
        let concentric_parent = ContentMask {
            bounds: bounds(
                point(ScaledPixels(8.0), ScaledPixels(8.0)),
                size(ScaledPixels(48.0), ScaledPixels(48.0)),
            ),
            corner_radii: Corners {
                top_left: ScaledPixels(16.0),
                top_right: ScaledPixels(16.0),
                bottom_right: ScaledPixels(16.0),
                bottom_left: ScaledPixels(16.0),
            },
            fade_out: Default::default(),
        };
        let concentric_child = ContentMask {
            bounds: bounds(
                point(ScaledPixels(8.0), ScaledPixels(8.0)),
                size(ScaledPixels(48.0), ScaledPixels(48.0)),
            ),
            corner_radii: Corners {
                top_left: ScaledPixels(8.0),
                top_right: ScaledPixels(8.0),
                bottom_right: ScaledPixels(8.0),
                bottom_left: ScaledPixels(8.0),
            },
            fade_out: Default::default(),
        };
        let quad = Quad {
            bounds: quad_bounds,
            content_mask: concentric_child,
            mask_chain: (2 << 24) | 0,
            background: Background::from(rgba(0xff0000ff)),
            ..Default::default()
        };
        let target = render_quad_with_mask_chain(
            &device,
            &context,
            quad,
            &[concentric_parent, concentric_child],
            [0.0, 0.0, 0.0, 1.0],
        )?;
        for (x, y, expected, what) in [
            (9, 15, 0, "parent radius 16 must clip more than child radius 8"),
            (15, 9, 0, "parent radius 16 must clip more than child radius 8"),
            (14, 14, 255, "inside both arcs must stay opaque"),
            (32, 32, 255, "interior must stay opaque"),
        ] {
            let px = read_pixel(&context, &target.texture, &target.staging, x, y)?;
            assert_eq!(
                px[2], expected,
                "{what} at ({x}, {y}), got {px:?} (B,G,R,A)"
            );
        }
        Ok(())
    }

    /// A scaled primitive's edge transition must be one *device* pixel wide. The distance field
    /// is evaluated in local space, so a 0.5 scale halves a local-unit ramp; before the
    /// derivative-based coverage, the pixel just outside the intended edge on screen could still
    /// be fully covered because it was inside by half a local unit.
    #[test]
    fn hlsl_quad_scaled_rounded_edge_aa_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[scaled_rounded_aa] WARP device unavailable, skipping");
            return Ok(());
        };
        let quad = Quad {
            bounds: bounds(
                point(ScaledPixels(8.0), ScaledPixels(8.0)),
                size(ScaledPixels(48.0), ScaledPixels(48.0)),
            ),
            // A mask larger than the transformed quad keeps the mask out of the measurement.
            content_mask: viewport_mask(),
            background: Background::from(rgba(0xff0000ff)),
            border_color: red().into(),
            border_widths: Edges {
                top: ScaledPixels(4.0),
                right: ScaledPixels(4.0),
                bottom: ScaledPixels(4.0),
                left: ScaledPixels(4.0),
            },
            // scene = local * 0.5 + (16.25, 16.25); the left edge lands at x = 20.25.
            transformation: TransformationMatrix {
                rotation_scale: [[0.5, 0.0], [0.0, 0.5]],
                translation: [16.25, 16.25],
            },
            ..Default::default()
        };
        let target = render_quad(&device, &context, quad, [0.0, 0.0, 0.0, 1.0])?;

        for (x, expected, what) in [
            (19, 0, "0.75 px outside the scaled edge"),
            (20, 191, "0.25 px inside the scaled edge"),
            (21, 255, "fully inside the scaled edge"),
        ] {
            let px = read_pixel(&context, &target.texture, &target.staging, x, 32)?;
            assert_red_close(px, expected, what);
        }
        Ok(())
    }

    /// Direct3D 11 raw buffer views are used as sprite instance buffers. Their offsets may be
    /// subject to an alignment rule stricter than four bytes: the polychrome instance stride is
    /// 152 bytes, so the second instance starts at byte 152 (not 16-byte aligned). Verify which
    /// offsets the driver accepts.
    #[test]
    fn raw_buffer_view_offset_alignment() -> Result<()> {
        let Some((device, _context)) = create_hardware_device().or_else(create_warp_device) else {
            eprintln!("[raw_srv] Direct3D device unavailable, skipping");
            return Ok(());
        };
        // 16 instances of the 152-byte polychrome sprite = 2432 bytes.
        let buffer = create_buffer(&device, 152, 16)?;
        for (first_element, expected) in [(0u32, true), (38, false), (76, true), (114, false)] {
            let result = create_buffer_view_range(&device, &buffer, first_element * 4, 38 * 4);
            eprintln!(
                "[raw_srv] first_element={first_element} byte_offset={} result={:?}",
                first_element * 4,
                result.as_ref().err()
            );
            if expected {
                assert!(
                    result.is_ok(),
                    "expected raw view at byte {} to succeed: {result:?}",
                    first_element * 4
                );
            } else {
                assert!(
                    result.is_err(),
                    "raw view at byte {} must be rejected (offset not 16-byte aligned)",
                    first_element * 4
                );
            }
        }
        Ok(())
    }

    /// `DrawInstanced` has a start-instance location. If `SV_InstanceID` includes it, batches
    /// can share one full-buffer raw view instead of creating per-range views at byte offsets
    /// that D3D11 rejects unless they are 16-byte aligned.
    #[test]
    fn quad_instance_start_location_readback() -> Result<()> {
        let Some((device, context)) = create_hardware_device().or_else(create_warp_device) else {
            eprintln!("[instance_start] Direct3D device unavailable, skipping");
            return Ok(());
        };
        let (texture, rtv, _srv) = create_color_target(&device, TEST_SIZE, TEST_SIZE)?;
        let rtv = rtv.expect("color target render target view");
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
        let globals = install_globals(&device, &context)?;
        let blend = create_blend_state(&device)?;
        let mut pipeline = PipelineState::<Quad>::new(
            &device,
            "test_instance_start",
            ShaderModule::Quad,
            4,
            blend.clone(),
        )?;
        let quad = |x: f32, color: u32| Quad {
            bounds: bounds(
                point(ScaledPixels(x), ScaledPixels(8.0)),
                size(ScaledPixels(16.0), ScaledPixels(16.0)),
            ),
            content_mask: viewport_mask(),
            background: Background::from(rgba(color)),
            ..Default::default()
        };
        pipeline.update_buffer(&device, &context, &[quad(8.0, 0xff0000ff), quad(40.0, 0x00ff00ff)])?;
        set_pipeline_state(
            &context,
            &[pipeline.view.clone()],
            D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
            &[test_viewport()],
            &pipeline.vertex,
            &pipeline.fragment,
            &globals,
            &blend,
        );
        unsafe {
            context.ClearRenderTargetView(&rtv, &[0.0, 0.0, 0.0, 1.0]);
            context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv.clone()))), None);
            context.DrawInstanced(4, 1, 0, 1);
        }
        dump_device_errors(&device);
        // `SV_InstanceID` does NOT include `StartInstanceLocation`: the first quad is drawn
        // even though the draw starts at instance 1. That is why batches bind a range view
        // (with a 16-byte-aligned offset) instead of indexing the full buffer.
        let first = read_pixel(&context, &texture, &staging, 16, 16)?;
        assert_eq!(
            first,
            [0, 0, 255, 255],
            "SV_InstanceID must not be offset by StartInstanceLocation, got {first:?} (B,G,R,A)"
        );
        let second = read_pixel(&context, &texture, &staging, 48, 16)?;
        assert_eq!(
            second,
            [0, 0, 0, 255],
            "the second quad must not be drawn when only one instance is drawn, got {second:?} (B,G,R,A)"
        );
        Ok(())
    }

    /// The global window mask is evaluated directly by the fragment shaders. A red quad filling
    /// the viewport must be clipped by the mask's rounded corner even though the quad's own
    /// content mask is the full viewport.
    #[test]
    fn hlsl_window_mask_rounds_corners() -> Result<()> {
        let Some((device, context)) = create_hardware_device().or_else(create_warp_device) else {
            eprintln!("[window_mask] Direct3D device unavailable, skipping");
            return Ok(());
        };
        let (texture, rtv, _srv) = create_color_target(&device, TEST_SIZE, TEST_SIZE)?;
        let rtv = rtv.expect("color target render target view");
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
        let radius = ScaledPixels(16.0);
        let globals_buffer =
            create_constant_buffer(&device, std::mem::size_of::<GlobalUniforms>())?;
        update_buffer(
            &context,
            &globals_buffer,
            &[GlobalUniforms {
                viewport_size: [TEST_SIZE as f32, TEST_SIZE as f32],
                premultiplied_alpha: 0,
                padding: 0,
                window_mask_params: [1.0, 0.0, 0.0, 0.0],
                window_mask: ContentMask {
                    bounds: bounds(
                        point(ScaledPixels(0.0), ScaledPixels(0.0)),
                        size(ScaledPixels(TEST_SIZE as f32), ScaledPixels(TEST_SIZE as f32)),
                    ),
                    corner_radii: Corners {
                        top_left: radius,
                        top_right: radius,
                        bottom_right: radius,
                        bottom_left: radius,
                    },
                    fade_out: Default::default(),
                },
            }],
        )?;
        let font_buffer =
            create_constant_buffer(&device, std::mem::size_of::<FontRasterizationUniforms>())?;
        update_buffer(
            &context,
            &font_buffer,
            &[FontRasterizationUniforms::default()],
        )?;
        let draw_constants_buffer = create_constant_buffer(&device, 16)?;
        update_buffer(
            &context,
            &draw_constants_buffer,
            &[hgpui_render::artifacts::Dx11DrawConstants::default()],
        )?;
        let globals = [
            Some(globals_buffer),
            Some(font_buffer),
            None,
            Some(draw_constants_buffer),
        ];

        let blend = create_blend_state(&device)?;
        let mut pipeline = PipelineState::<Quad>::new(
            &device,
            "test_window_mask",
            ShaderModule::Quad,
            4,
            blend,
        )?;
        let quad = Quad {
            bounds: bounds(
                point(ScaledPixels(0.0), ScaledPixels(0.0)),
                size(ScaledPixels(TEST_SIZE as f32), ScaledPixels(TEST_SIZE as f32)),
            ),
            content_mask: viewport_mask(),
            background: Background::from(rgba(0xff0000ff)),
            ..Default::default()
        };
        pipeline.update_buffer(&device, &context, &[quad])?;
        unsafe {
            context.ClearRenderTargetView(&rtv, &[0.0, 0.0, 0.0, 1.0]);
            context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv.clone()))), None);
        }
        pipeline.draw_range(&device, &context, &[test_viewport()], &globals, 4, 0, 1)?;
        dump_device_errors(&device);

        let corner = read_pixel(&context, &texture, &staging, 2, 2)?;
        assert_eq!(
            corner[2], 0,
            "the window mask corner must clip the quad, got {corner:?}"
        );
        let inside = read_pixel(&context, &texture, &staging, 8, 8)?;
        assert_eq!(
            inside[2], 255,
            "the window mask interior must stay opaque, got {inside:?}"
        );
        let center = read_pixel(&context, &texture, &staging, 32, 32)?;
        assert_eq!(center[2], 255, "window mask centre must stay opaque, got {center:?}");
        Ok(())
    }

    /// When a primitive's content mask is its own rounded cutout, the mask's distance field is
    /// the same field the primitive already uses for its outline. Applying both squares the
    /// coverage of every edge pixel (0.5 × 0.5 = 0.25), which shows up as a dark one-pixel rim
    /// around rounded elements. The mask must contribute only its edge fade in that case.
    #[test]
    fn hlsl_quad_own_mask_coverage_not_squared() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[own_mask] WARP device unavailable, skipping");
            return Ok(());
        };
        let quad_bounds = bounds(
            point(ScaledPixels(8.25), ScaledPixels(8.25)),
            size(ScaledPixels(48.0), ScaledPixels(48.0)),
        );
        // A border exercises the path that computes the shape's own outer distance field on a
        // straight edge (an unrounded, unbordered quad delegates that to the mask instead).
        let border_widths = Edges {
            top: ScaledPixels(4.0),
            right: ScaledPixels(4.0),
            bottom: ScaledPixels(4.0),
            left: ScaledPixels(4.0),
        };
        let quad = Quad {
            bounds: quad_bounds,
            content_mask: ContentMask {
                bounds: quad_bounds,
                corner_radii: Corners::default(),
                fade_out: Default::default(),
            },
            background: Background::from(rgba(0xff0000ff)),
            border_color: red().into(),
            border_widths,
            ..Default::default()
        };
        let target = render_quad(&device, &context, quad, [0.0, 0.0, 0.0, 1.0])?;

        // Bounds left edge is x = 8.25; pixel 8's centre is 0.25 px inside, so the shape's own
        // coverage is 0.75 (191). If the own mask is multiplied in as well the pixel drops to
        // 0.5625 (143).
        let edge = read_pixel(&context, &target.texture, &target.staging, 8, 32)?;
        assert_red_close(edge, 191, "own-mask straight edge");
        let inside = read_pixel(&context, &target.texture, &target.staging, 32, 32)?;
        assert_eq!(inside[2], 255, "own-mask interior must stay opaque, got {inside:?}");
        Ok(())
    }

    /// Renders vector-path geometry through the real path pipelines. A constant `curve_position`
    /// makes the curve field's derivative zero, which the shader treats as "solid coverage", so
    /// the only shaping left is the content mask / gradient under test.
    struct PathTarget {
        texture: ID3D11Texture2D,
        staging: ID3D11Texture2D,
        _pipeline: PipelineState<PathRasterizationSprite>,
        _globals: [Option<ID3D11Buffer>; 4],
        _rtv: ID3D11RenderTargetView,
        _mask_buffer: Option<ID3D11Buffer>,
        _mask_view: Option<ID3D11ShaderResourceView>,
    }

    fn render_path(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        color: Background,
        path_bounds: Bounds<ScaledPixels>,
        content_mask: ContentMask<ScaledPixels>,
        st_scale: Option<f32>,
    ) -> Result<PathTarget> {
        let (texture, rtv, staging) = make_render_target(device)?;
        let globals = install_globals(device, context)?;
        let blend = create_blend_state_for_path_rasterization(device)?;
        let mut pipeline = PipelineState::<PathRasterizationSprite>::new(
            device,
            "test_path_precision",
            ShaderModule::PathRasterization,
            6,
            blend,
        )?;
        // Give the vertices a real one-entry mask chain: rounded corners/fades live in the
        // chain now, not in per-vertex fields.
        let mask_buffer = create_buffer(
            device,
            std::mem::size_of::<ContentMask<ScaledPixels>>(),
            1,
        )?;
        update_buffer(context, &mask_buffer, &[content_mask])?;
        let mask_view = create_buffer_view(device, &mask_buffer)?;
        let mask_chain = 1 << 24;
        let vertex = |x: f32, y: f32| PathRasterizationSprite {
            xy_position: point(ScaledPixels(x), ScaledPixels(y)),
            curve_position: match st_scale {
                // st = (x/s, y/s) makes the curve field `st.y - st.x^2` the parabola
                // y = x^2 / s through the scene, which is how the real path rasterizer feeds
                // the exact curve into the shader.
                Some(s) => point(x / s, y / s),
                // A constant st is the degenerate case: alpha falls back to a solid fill.
                None => point(0.0, 0.0),
            },
            color,
            bounds: path_bounds,
            content_mask: content_mask.bounds,
            mask_chain,
            padding: [0; 3],
        };
        // Two triangles covering the whole viewport.
        let vertices = [
            vertex(0.0, 0.0),
            vertex(TEST_SIZE as f32, 0.0),
            vertex(0.0, TEST_SIZE as f32),
            vertex(TEST_SIZE as f32, 0.0),
            vertex(TEST_SIZE as f32, TEST_SIZE as f32),
            vertex(0.0, TEST_SIZE as f32),
        ];
        pipeline.update_buffer(device, context, &vertices)?;
        unsafe {
            context.ClearRenderTargetView(&rtv, &[0.0, 0.0, 0.0, 1.0]);
            context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv.clone()))), None);
            context.PSSetShaderResources(5, Some(slice::from_ref(&mask_view)));
            context.VSSetShaderResources(5, Some(slice::from_ref(&mask_view)));
        }
        pipeline.draw(
            context,
            &[test_viewport()],
            &globals,
            D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            6,
            1,
        )?;
        dump_device_errors(device);
        Ok(PathTarget {
            texture,
            staging,
            _pipeline: pipeline,
            _globals: globals,
            _rtv: rtv,
            _mask_buffer: Some(mask_buffer),
            _mask_view: mask_view,
        })
    }

    /// Vector paths must honour a rounded content mask. Before this fix the renderer clipped
    /// the path to the mask's *bounding rectangle* only (and used that same rectangle as the
    /// gradient box), so paths leaked through every rounded corner.
    #[test]
    fn hlsl_path_rounded_mask_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[path_mask] WARP device unavailable, skipping");
            return Ok(());
        };
        let mask = ContentMask {
            bounds: bounds(
                point(ScaledPixels(16.0), ScaledPixels(16.0)),
                size(ScaledPixels(32.0), ScaledPixels(32.0)),
            ),
            corner_radii: Corners {
                top_left: ScaledPixels(8.0),
                top_right: ScaledPixels(8.0),
                bottom_right: ScaledPixels(8.0),
                bottom_left: ScaledPixels(8.0),
            },

            fade_out: Default::default(),
        };
        let target = render_path(
            &device,
            &context,
            Background::from(rgba(0xff0000ff)),
            bounds(
                point(ScaledPixels(8.0), ScaledPixels(8.0)),
                size(ScaledPixels(48.0), ScaledPixels(48.0)),
            ),
            mask,
            None,
        )?;

        // Outside the rounded arc, inside the mask rectangle: the corner must be cut away.
        let corner = read_pixel(&context, &target.texture, &target.staging, 16, 16)?;
        assert_eq!(corner[2], 0, "path must not fill the mask's rounded corner, got {corner:?}");
        // Just inside the arc: d = sqrt(2)*5.5 - 8 = -0.222 -> coverage 0.722 -> 184.
        let arc = read_pixel(&context, &target.texture, &target.staging, 18, 18)?;
        assert_red_close(arc, 184, "path mask arc");
        let body = read_pixel(&context, &target.texture, &target.staging, 32, 32)?;
        assert_eq!(body[2], 255, "path body must be solid, got {body:?}");
        let straight = read_pixel(&context, &target.texture, &target.staging, 16, 32)?;
        assert_eq!(straight[2], 255, "mask's straight edge pixel must stay covered, got {straight:?}");
        Ok(())
    }

    /// A clipped path's gradient must stay anchored to the path's own bounds, not to the
    /// intersection rectangle the old code passed as `bounds`.
    #[test]
    fn hlsl_path_gradient_bounds_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[path_gradient] WARP device unavailable, skipping");
            return Ok(());
        };
        let mask = ContentMask {
            bounds: bounds(
                point(ScaledPixels(24.0), ScaledPixels(16.0)),
                size(ScaledPixels(16.0), ScaledPixels(32.0)),
            ),
            corner_radii: Corners::default(),

            fade_out: Default::default(),
        };
        let color = linear_gradient(
            90.0,
            linear_color_stop(hsla(0.0, 0.0, 0.0, 1.0), 0.0),
            linear_color_stop(hsla(0.0, 0.0, 1.0, 1.0), 1.0),
        )
        .color_space(ColorSpace::Srgb);
        let target = render_path(
            &device,
            &context,
            color,
            bounds(
                point(ScaledPixels(8.0), ScaledPixels(8.0)),
                size(ScaledPixels(48.0), ScaledPixels(48.0)),
            ),
            mask,
            None,
        )?;

        // t = (28.5 - 8) / 48 = 0.4271 -> 0.4271 * 255 = 109.
        // (Against the clipped [24,40] box it would be (28.5-24)/16 = 0.281 -> 72.)
        for (x, y) in [(28, 32), (28, 20), (28, 44)] {
            let px = read_pixel(&context, &target.texture, &target.staging, x, y)?;
            assert_red_close(px, 38, "path gradient anchored to the path bounds");
        }
        Ok(())
    }

    /// The path fragment's curve antialiasing must actually run. The degenerate-derivative
    /// condition used to be inverted (`if (length(d_st_x)) -> alpha = 1`), so every normal path
    /// painted as a solid triangle and the distance field only ran on degenerate ones (0/0).
    /// With `st = (x/32, y/32)` the curve field's zero set is the parabola y = x^2/32.
    #[test]
    fn hlsl_path_curve_aa_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[path_curve_aa] WARP device unavailable, skipping");
            return Ok(());
        };
        let target = render_path(
            &device,
            &context,
            Background::from(rgba(0xff0000ff)),
            bounds(
                point(ScaledPixels(8.0), ScaledPixels(8.0)),
                size(ScaledPixels(48.0), ScaledPixels(48.0)),
            ),
            viewport_mask(),
            Some(32.0),
        )?;
        // y = x^2/32 is 32 at x = 32; the filled side is y > x^2/32.
        let inside = read_pixel(&context, &target.texture, &target.staging, 32, 40)?;
        assert_red_close(inside, 255, "path curve: inside the parabola");
        let outside = read_pixel(&context, &target.texture, &target.staging, 32, 20)?;
        assert_eq!(outside[2], 0, "path curve: outside the parabola, got {outside:?}");
        // Pixel (32, 33): st = (1.015625, 1.046875), f = -0.01538, |grad| = 0.07075 ->
        // distance = -0.217 px -> coverage 0.717 -> 183.
        let edge = read_pixel(&context, &target.texture, &target.staging, 32, 33)?;
        assert_red_close(edge, 183, "path curve: antialiased edge");
        Ok(())
    }

    /// `closest-corner` with a centre outside the box used to take a signed minimum, producing
    /// a negative per-axis radius; the `radii > 0` guard then fell back to the last stop for
    /// the whole element. With unsigned side distances the ellipse is sized to the nearest
    /// corner, so the left-edge midpoint sits at t = 0.771 -> 0.25 + 0.771 * 0.75 = 0.828 -> 211.
    /// (A closest-*side* ellipse with an off-box centre happens to be past t = 1 everywhere in
    /// the box, so the last stop was the right answer there; closest-corner is where the
    /// fallback was visibly wrong.)
    #[test]
    fn hlsl_radial_offbox_closest_corner_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[radial_offbox] WARP device unavailable, skipping");
            return Ok(());
        };
        let quad = Quad {
            bounds: bounds(
                point(ScaledPixels(4.0), ScaledPixels(20.0)),
                size(ScaledPixels(56.0), ScaledPixels(24.0)),
            ),
            content_mask: viewport_mask(),
            background: radial_gradient(
                point(-0.1, 0.5),
                linear_color_stop(hsla(0.0, 0.0, 0.25, 1.0), 0.0),
                linear_color_stop(hsla(0.0, 0.0, 1.0, 1.0), 1.0),
            )
            .radial_size(RadialSize::ClosestCorner)
            .color_space(ColorSpace::Srgb),
            ..Default::default()
        };
        let target = render_quad(&device, &context, quad, [0.0, 0.0, 0.0, 1.0])?;
        let px = read_pixel(&context, &target.texture, &target.staging, 4, 32)?;
        assert_red_close(px, 198, "off-box closest-corner radial gradient");
        Ok(())
    }

    /// A transparent background over a transparent border is 0/0 in straight-alpha `over`.
    /// The guard must keep the NaN out of the framebuffer: the clear colour has to survive.
    #[test]
    fn hlsl_transparent_border_over_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[transparent_over] WARP device unavailable, skipping");
            return Ok(());
        };
        let quad = Quad {
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
        // Pure blue clear (RGBA; the BGRA target stores it as [255, 0, 0, 255]): a NaN written
        // into the border band would land as 0 (black), distinguishable from the clear colour.
        let target = render_quad(&device, &context, quad, [0.0, 0.0, 1.0, 1.0])?;
        for (x, y) in [(17, 32), (47, 32), (32, 17), (32, 47)] {
            let px = read_pixel(&context, &target.texture, &target.staging, x, y)?;
            assert_eq!(
                px,
                [255, 0, 0, 255],
                "a fully transparent border must leave the clear colour at ({x}, {y}), got {px:?}"
            );
        }
        Ok(())
    }

    /// An inset shadow's element edge is both its outer shape and its clip, so the geometry is
    /// grown by AA_MARGIN; without that the fragment stage's element-edge ramp had no fragment
    /// outside the element and the edge snapped inward by up to half a pixel.
    #[test]
    fn hlsl_inset_shadow_element_edge_aa_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[inset_shadow_aa] WARP device unavailable, skipping");
            return Ok(());
        };
        let (texture, rtv, staging) = make_render_target(&device)?;
        let globals = install_globals(&device, &context)?;
        let blend = create_blend_state(&device)?;
        let mut pipeline = PipelineState::<Shadow>::new(
            &device,
            "test_shadow_precision",
            ShaderModule::Shadow,
            4,
            blend,
        )?;
        // Element edge at 16.75: pixel 16's centre is 0.25 px outside the element, so its
        // element-edge coverage is 0.25. The hole sits 4 px inside.
        let shadow = Shadow {
            order: 0,
            blur_radius: ScaledPixels(0.0),
            bounds: bounds(
                point(ScaledPixels(20.75), ScaledPixels(20.75)),
                size(ScaledPixels(22.5), ScaledPixels(22.5)),
            ),
            corner_radii: Corners::default(),
            content_mask: viewport_mask(),
            color: red().into(),
            element_bounds: bounds(
                point(ScaledPixels(16.75), ScaledPixels(16.75)),
                size(ScaledPixels(30.5), ScaledPixels(30.5)),
            ),
            element_corner_radii: Corners::default(),
            inset_and_mask_chain: Shadow::pack_inset_and_mask_chain(true, 0),
            corner_smoothing: 0.0,
        };
        pipeline.update_buffer(&device, &context, &[shadow])?;
        unsafe {
            context.ClearRenderTargetView(&rtv, &[0.0, 0.0, 0.0, 1.0]);
            context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv.clone()))), None);
        }
        pipeline.draw_range(&device, &context, &[test_viewport()], &globals, 4, 0, 1)?;
        dump_device_errors(&device);

        let edge = read_pixel(&context, &texture, &staging, 16, 32)?;
        assert_red_close(edge, 0, "inset shadow element edge (upstream clips to element bounds)");
        let inside = read_pixel(&context, &texture, &staging, 17, 32)?;
        assert_red_close(inside, 255, "inset shadow inside the element");
        let outside = read_pixel(&context, &texture, &staging, 15, 32)?;
        assert_eq!(outside[2], 0, "inset shadow must not escape the element, got {outside:?}");
        Ok(())
    }

    /// A zero-sized checkerboard used to divide the local position by zero (NaN). The guarded
    /// branch paints nothing, so the clear colour must survive untouched.
    #[test]
    fn hlsl_zero_size_checkerboard_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[checker_zero] WARP device unavailable, skipping");
            return Ok(());
        };
        let quad = Quad {
            bounds: bounds(
                point(ScaledPixels(16.0), ScaledPixels(16.0)),
                size(ScaledPixels(32.0), ScaledPixels(32.0)),
            ),
            content_mask: viewport_mask(),
            background: hgpui::checkerboard(rgba(0xff0000ff), 0.0),
            ..Default::default()
        };
        // Blue clear (BGRA [255, 0, 0, 255]).
        let target = render_quad(&device, &context, quad, [0.0, 0.0, 1.0, 1.0])?;
        let px = read_pixel(&context, &target.texture, &target.staging, 32, 32)?;
        assert_eq!(px, [255, 0, 0, 255], "zero-size checkerboard must be transparent, got {px:?}");
        Ok(())
    }

    /// A dashed border with a zero-width side divided by zero in the dash layout (infinite
    /// velocity, then 0/0 for the period). The guarded layout must paint the corner sliver as a
    /// solid border instead of a NaN.
    #[test]
    fn hlsl_dashed_zero_width_side_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[dash_zero_width] WARP device unavailable, skipping");
            return Ok(());
        };
        let quad = Quad {
            bounds: bounds(
                point(ScaledPixels(16.0), ScaledPixels(16.0)),
                size(ScaledPixels(32.0), ScaledPixels(32.0)),
            ),
            content_mask: viewport_mask(),
            background: Background::from(transparent_black()),
            border_color: red().into(),
            border_widths: Edges {
                top: ScaledPixels(0.0),
                right: ScaledPixels(0.0),
                bottom: ScaledPixels(0.0),
                left: ScaledPixels(4.0),
            },
            border_style: BorderStyle::Dashed,
            ..Default::default()
        };
        let target = render_quad(&device, &context, quad, [0.0, 0.0, 0.0, 1.0])?;
        // (18, 20) is in the left border band and used to pick the zero-width top side as
        // "horizontal", taking dash_velocity to infinity and the period to NaN.
        let px = read_pixel(&context, &target.texture, &target.staging, 18, 20)?;
        assert_red_close(px, 255, "zero-width dashed side sliver");
        Ok(())
    }

    /// Upstream corner smoothing uses a superellipse corner. At smoothing 1.0 the
    /// corner reaches farther toward the box corner than the circular radius, so a
    /// pixel just outside the circle must gain coverage only in the smoothed pipeline.
    #[test]
    fn hlsl_corner_smoothing_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[corner_smoothing] WARP device unavailable, skipping");
            return Ok(());
        };
        let (texture, rtv, staging) = make_render_target(&device)?;
        let globals = install_globals(&device, &context)?;

        let radii = Corners {
            top_left: ScaledPixels(16.0),
            top_right: ScaledPixels(16.0),
            bottom_right: ScaledPixels(16.0),
            bottom_left: ScaledPixels(16.0),
        };
        let quad_bounds = bounds(
            point(ScaledPixels(16.0), ScaledPixels(16.0)),
            size(ScaledPixels(32.0), ScaledPixels(32.0)),
        );
        let quad = Quad {
            bounds: quad_bounds,
            content_mask: ContentMask {
                bounds: quad_bounds,
                corner_radii: radii,
                fade_out: Default::default(),
            },
            background: Background::from(rgba(0xff0000ff)),
            corner_radii: radii,
            corner_smoothing: 1.0,
            ..Default::default()
        };

        let mut normal = PipelineState::<Quad>::new(
            &device,
            "test_corner_normal",
            ShaderModule::Quad,
            4,
            create_blend_state(&device)?,
        )?;
        normal.update_buffer(&device, &context, &[quad])?;
        unsafe {
            context.ClearRenderTargetView(&rtv, &[0.0, 0.0, 0.0, 1.0]);
            context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv.clone()))), None);
        }
        normal.draw_range(&device, &context, &[test_viewport()], &globals, 4, 0, 1)?;
        let circular_count = {
            let mut count = 0;
            for y in 16..32 {
                for x in 16..32 {
                    if read_pixel(&context, &texture, &staging, x, y)?[2] > 8 {
                        count += 1;
                    }
                }
            }
            count
        };

        let mut smoothed = PipelineState::<Quad>::new(
            &device,
            "test_corner_smoothed",
            ShaderModule::SmoothedQuad,
            4,
            create_blend_state(&device)?,
        )?;
        smoothed.update_buffer(&device, &context, &[quad])?;
        unsafe {
            context.ClearRenderTargetView(&rtv, &[0.0, 0.0, 0.0, 1.0]);
            context.OMSetRenderTargets(Some(slice::from_ref(&Some(rtv.clone()))), None);
        }
        smoothed.draw_range(&device, &context, &[test_viewport()], &globals, 4, 0, 1)?;
        let smoothed_count = {
            let mut count = 0;
            for y in 16..32 {
                for x in 16..32 {
                    if read_pixel(&context, &texture, &staging, x, y)?[2] > 8 {
                        count += 1;
                    }
                }
            }
            count
        };

        assert!(
            smoothed_count > circular_count,
            "smoothed corner must cover more pixels than the circular corner: \
             circular={circular_count}, smoothed={smoothed_count}"
        );
        Ok(())
    }

    /// The dash length/gap are per-instance values, not a hard-coded 2:1 pattern.
    /// Two quads with different settings must produce different top-border runs.
    #[test]
    fn hlsl_dashed_length_gap_readback() -> Result<()> {
        let Some((device, context)) = create_warp_device() else {
            eprintln!("[dash_length_gap] WARP device unavailable, skipping");
            return Ok(());
        };
        let quad_bounds = bounds(
            point(ScaledPixels(16.0), ScaledPixels(16.0)),
            size(ScaledPixels(32.0), ScaledPixels(32.0)),
        );
        let make_quad = |length: f32, gap: f32| Quad {
            bounds: quad_bounds,
            content_mask: viewport_mask(),
            background: Background::from(transparent_black()),
            border_color: red().into(),
            border_widths: Edges {
                top: ScaledPixels(4.0),
                right: ScaledPixels(4.0),
                bottom: ScaledPixels(4.0),
                left: ScaledPixels(4.0),
            },
            border_style: BorderStyle::Dashed,
            border_dashed_length: length,
            border_dashed_gap: gap,
            ..Default::default()
        };

        let count_red = |target: &QuadTarget| -> Result<usize> {
            let mut count = 0;
            for x in 16..48 {
                if read_pixel(&context, &target.texture, &target.staging, x, 18)?[2] > 128 {
                    count += 1;
                }
            }
            Ok(count)
        };

        let short = render_quad(&device, &context, make_quad(1.0, 1.0), [0.0, 0.0, 0.0, 1.0])?;
        let long = render_quad(&device, &context, make_quad(4.0, 1.0), [0.0, 0.0, 0.0, 1.0])?;
        let short_count = count_red(&short)?;
        let long_count = count_red(&long)?;
        assert!(short_count > 0, "short dashes must paint something");
        assert!(long_count > 0, "long dashes must paint something");
        assert_ne!(
            short_count, long_count,
            "dash length must change the top-border run lengths"
        );
        Ok(())
    }
}
