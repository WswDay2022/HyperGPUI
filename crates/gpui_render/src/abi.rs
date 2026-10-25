//! Host-side ABI for data uploaded to the unified shaders.
//!
//! The scene primitives themselves live in `hgpui::scene` and are uploaded directly; these
//! helper types cover the renderer-local payloads that used to be duplicated in every backend.

use hgpui::{Background, Bounds, Point, ScaledPixels};

/// Per-frame globals shared by every pipeline.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GlobalUniforms {
    pub viewport_size: [f32; 2],
    pub premultiplied_alpha: u32,
    pub padding: u32,
    /// `[enabled, 0, 0, 0]`; see the shader's `window_mask_alpha`.
    pub window_mask_params: [f32; 4],
    pub window_mask: hgpui::ContentMask<ScaledPixels>,
}

/// Font rasterization parameters.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FontRasterizationUniforms {
    pub gamma_ratios: [f32; 4],
    pub grayscale_enhanced_contrast: f32,
    pub subpixel_enhanced_contrast: f32,
    pub uses_blue_green_red_subpixel_order: u32,
    pub padding: u32,
}

/// One path sprite (the texture-space quad produced by rasterizing vector paths).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PathSprite {
    pub bounds: Bounds<ScaledPixels>,
}

/// One vertex of a rasterized vector path.
///
/// Layout mirrors the three shader backends: 144 bytes with the content mask carried alongside
/// the path's own gradient box.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathRasterizationVertex {
    pub xy_position: Point<ScaledPixels>,
    pub curve_position: Point<f32>,
    pub color: Background,
    /// The path's own bounds: the gradient box.
    pub bounds: Bounds<ScaledPixels>,
    /// The content mask's bounds, used for clip distances and the legacy fallback.
    pub content_mask: Bounds<ScaledPixels>,
    /// Reference to the scene's content-mask chain arena (see `Scene::mask_chains`). The chain
    /// carries the mask's corner radii and fades, so they are not duplicated per vertex.
    pub mask_chain: u32,
    /// Pads the stride to 144 bytes (16-byte-aligned D3D11 raw-view offsets).
    pub padding: [u32; 3],
}

/// Parameters shared by the blur downsample / gaussian / composite passes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlurUniforms {
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: Bounds<ScaledPixels>,
    pub content_mask_radii: [f32; 4],
    /// Packed reference into the frame's content-mask chain arena; the chain carries the mask's
    /// corner radii and fades.
    pub mask_chain: u32,
    pub padding0: u32,
    pub padding1: u32,
    pub padding2: u32,
    pub corner_radii: [f32; 4],
    pub direction: [f32; 2],
    pub sigma: f32,
    pub opacity: f32,
    pub tap_count: f32,
    pub tap_step: f32,
    pub clip_rounded: f32,
    pub downsample: f32,
}

/// Uniform for the surface (YUV/RGBA texture) pipeline.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceUniforms {
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: hgpui::ContentMask<ScaledPixels>,
    /// 0 = RGBA, 1 = YUV (biplanar Y + CbCr).
    pub color_format: u32,
    pub opacity: f32,
    pub padding0: u32,
    pub padding1: u32,
    pub padding2: u32,
    pub padding3: u32,
    pub padding4: u32,
    pub padding5: u32,
}

const _: () = {
    assert!(std::mem::size_of::<GlobalUniforms>() == 80);
    assert!(std::mem::size_of::<FontRasterizationUniforms>() == 32);
    assert!(std::mem::size_of::<PathSprite>() == 16);
    assert!(std::mem::size_of::<PathRasterizationVertex>() == 144);
    assert!(std::mem::size_of::<BlurUniforms>() == 112);
    assert!(std::mem::size_of::<SurfaceUniforms>() == 96);
};
