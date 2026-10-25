//! Typed shader contracts shared by the build script and the three backends.

use crate::abi::{
    BlurUniforms, FontRasterizationUniforms, GlobalUniforms, PathRasterizationVertex, PathSprite,
    SurfaceUniforms,
};

/// Shader-declared data that can be uploaded without serialization.
///
/// # Safety
/// Implementors need a stable C-compatible layout and must match the WGSL declaration.
pub unsafe trait BufferData: Sized {
    const WGSL_TYPE: &'static str;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageAbi {
    pub wgsl_type: &'static str,
    pub rust_stride: usize,
}

pub const fn storage_abi<T: BufferData>() -> StorageAbi {
    StorageAbi {
        wgsl_type: T::WGSL_TYPE,
        rust_stride: std::mem::size_of::<T>(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataLayout {
    Instances,
    TexturedInstances,
    MonochromeSprites,
    SubpixelSprites,
    NativeOnly,
    Surface,
    Blur,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VertexCount {
    Rectangle,
    FullscreenTriangle,
    Dynamic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrimitiveTopology {
    TriangleList,
    TriangleStrip,
}

#[derive(Clone, Copy)]
pub struct Pipeline {
    pub label: &'static str,
    pub vertex_entry: &'static str,
    pub fragment_entry: &'static str,
    pub topology: PrimitiveTopology,
    pub data_layout: DataLayout,
    pub vertex_count: VertexCount,
}

macro_rules! define_pipelines {
    ($($name:ident: $label:literal, $vertex:ident, $fragment:ident, $topology:ident, $layout:ident, $vertices:ident;)*) => {
        $(
            pub const $name: Pipeline = Pipeline {
                label: $label,
                vertex_entry: stringify!($vertex),
                fragment_entry: stringify!($fragment),
                topology: PrimitiveTopology::$topology,
                data_layout: DataLayout::$layout,
                vertex_count: VertexCount::$vertices,
            };
        )*

        pub const ALL: &[Pipeline] = &[$($name),*];
    };
}

define_pipelines! {
    QUADS: "quad", quad_vertex, quad_fragment, TriangleStrip, Instances, Rectangle;
    SMOOTHED_QUADS: "smoothed_quad", vertex_smoothed_quad, fragment_smoothed_quad, TriangleStrip, Instances, Rectangle;
    SHADOWS: "shadow", shadow_vertex, shadow_fragment, TriangleStrip, Instances, Rectangle;
    SMOOTHED_SHADOWS: "smoothed_shadow", vertex_smoothed_shadow, fragment_smoothed_shadow, TriangleStrip, Instances, Rectangle;
    PATH_RASTERIZATION: "path_rasterization", path_rasterization_vertex, path_rasterization_fragment, TriangleList, Instances, Dynamic;
    PATHS: "path_sprite", path_sprite_vertex, path_sprite_fragment, TriangleStrip, TexturedInstances, Rectangle;
    UNDERLINES: "underline", underline_vertex, underline_fragment, TriangleStrip, Instances, Rectangle;
    MONOCHROME_SPRITES: "monochrome_sprite", monochrome_sprite_vertex, monochrome_sprite_fragment, TriangleStrip, MonochromeSprites, Rectangle;
    SUBPIXEL_SPRITES: "subpixel_sprite", subpixel_sprite_vertex, subpixel_sprite_fragment, TriangleStrip, SubpixelSprites, Rectangle;
    POLYCHROME_SPRITES: "polychrome_sprite", polychrome_sprite_vertex, polychrome_sprite_fragment, TriangleStrip, TexturedInstances, Rectangle;
    SMOOTHED_POLYCHROME_SPRITES: "smoothed_polychrome_sprite", vertex_smoothed_polychrome_sprite, fragment_smoothed_polychrome_sprite, TriangleStrip, TexturedInstances, Rectangle;
    SURFACES: "surface", surface_vertex, surface_fragment, TriangleStrip, Surface, Rectangle;
    BLUR_DOWNSAMPLE: "blur_downsample", blur_vertex, blur_downsample_fragment, TriangleList, Blur, FullscreenTriangle;
    BLUR: "blur", blur_vertex, blur_fragment, TriangleList, Blur, FullscreenTriangle;
    BLUR_COMPOSITE: "blur_composite", blur_composite_vertex, blur_composite_fragment, TriangleStrip, Blur, Rectangle;
}

pub const EMOJI_RASTERIZATION: Pipeline = Pipeline {
    label: "emoji_rasterization",
    vertex_entry: "emoji_rasterization_vertex",
    fragment_entry: "emoji_rasterization_fragment",
    topology: PrimitiveTopology::TriangleStrip,
    data_layout: DataLayout::NativeOnly,
    vertex_count: VertexCount::Rectangle,
};

pub const GLOBAL_BIND_GROUP: u32 = 0;
pub const DATA_BIND_GROUP: u32 = 1;
pub const GLOBAL_UNIFORMS_BINDING: u32 = 0;
pub const FONT_RASTERIZATION_BINDING: u32 = 1;
pub const DATA_BUFFER_BINDING: u32 = 0;
pub const PRIMARY_TEXTURE_BINDING: u32 = 1;
pub const SECONDARY_TEXTURE_BINDING: u32 = 2;
pub const PRIMARY_SAMPLER_BINDING: u32 = 2;
pub const SURFACE_SAMPLER_BINDING: u32 = 3;
pub const RECTANGLE_VERTEX_COUNT: u32 = 4;
pub const FULLSCREEN_TRIANGLE_VERTEX_COUNT: u32 = 3;
pub const DX11_DRAW_CONSTANTS_REGISTER: u32 = 3;

macro_rules! buffer_data {
    ($($rust:ty => $wgsl:literal),* $(,)?) => {
        $(
            unsafe impl BufferData for $rust {
                const WGSL_TYPE: &'static str = $wgsl;
            }
        )*
    };
}

buffer_data! {
    GlobalUniforms => "GlobalUniforms",
    FontRasterizationUniforms => "FontRasterizationUniforms",
    SurfaceUniforms => "SurfaceUniforms",
    BlurUniforms => "BlurUniforms",
    hgpui::Quad => "Quad",
    hgpui::Shadow => "Shadow",
    hgpui::Underline => "Underline",
    hgpui::MonochromeSprite => "MonochromeSprite",
    hgpui::SubpixelSprite => "SubpixelSprite",
    hgpui::PolychromeSprite => "PolychromeSprite",
    PathSprite => "PathSprite",
    PathRasterizationVertex => "PathRasterizationVertex",
}

pub const SCENE_STORAGE_ABI: &[StorageAbi] = &[
    storage_abi::<hgpui::Quad>(),
    storage_abi::<hgpui::Shadow>(),
    storage_abi::<hgpui::Underline>(),
    storage_abi::<hgpui::MonochromeSprite>(),
    storage_abi::<hgpui::SubpixelSprite>(),
    storage_abi::<hgpui::PolychromeSprite>(),
    storage_abi::<PathSprite>(),
    storage_abi::<PathRasterizationVertex>(),
];

/// CPU layout metadata consumed by the shader compiler.
#[doc(hidden)]
pub struct SceneBufferLayout {
    pub name: &'static str,
    pub size: usize,
    pub fields: &'static [(&'static str, usize)],
}

macro_rules! layout {
    ($ty:ty, $name:literal, $($field:ident),+ $(,)?) => {
        SceneBufferLayout {
            name: $name,
            size: std::mem::size_of::<$ty>(),
            fields: &[$((stringify!($field), std::mem::offset_of!($ty, $field))),+],
        }
    };
}

/// Mirror of `hgpui::Background` used only for layout introspection; the real fields are
/// crate-private in `hgpui`, but the layout is part of the GPU ABI.
#[repr(C)]
pub struct AbiHsla {
    h: f32,
    s: f32,
    l: f32,
    a: f32,
}

#[repr(C)]
pub struct AbiLinearColorStop {
    color: AbiHsla,
    percentage: f32,
}

#[repr(C)]
pub struct AbiBackground {
    tag: u32,
    color_space: u32,
    solid: AbiHsla,
    gradient_angle_or_pattern_height: f32,
    colors: [AbiLinearColorStop; 2],
    radial_center_y: f32,
    radial_size: u32,
    padding: u32,
}

#[doc(hidden)]
pub const SCENE_BUFFER_LAYOUTS: &[SceneBufferLayout] = &[
    layout!(hgpui::Bounds<hgpui::ScaledPixels>, "Bounds", origin, size),
    layout!(hgpui::Corners<hgpui::ScaledPixels>, "Corners", top_left, top_right, bottom_right, bottom_left),
    layout!(hgpui::Edges<hgpui::ScaledPixels>, "Edges", top, right, bottom, left),
    layout!(AbiBackground, "Background", tag, color_space, solid, gradient_angle_or_pattern_height, colors, radial_center_y, radial_size, padding),
    layout!(hgpui::ContentMask<hgpui::ScaledPixels>, "ContentMask", bounds, corner_radii, fade_out),
    layout!(hgpui::TransformationMatrix, "TransformationMatrix", rotation_scale, translation),
    layout!(hgpui::Quad, "Quad", order, border_style, border_dashed_length, border_dashed_gap, bounds, content_mask, background, border_color, corner_radii, border_widths, corner_smoothing, padding, transformation),
    layout!(hgpui::Shadow, "Shadow", order, blur_radius, bounds, corner_radii, content_mask, color, element_bounds, element_corner_radii, inset, corner_smoothing),
    layout!(hgpui::Underline, "Underline", order, padding, bounds, content_mask, color, thickness, wavy, transformation),
    layout!(hgpui::MonochromeSprite, "MonochromeSprite", order, padding, bounds, content_mask, color, tile, transformation),
    layout!(hgpui::SubpixelSprite, "SubpixelSprite", order, padding, bounds, content_mask, color, tile, transformation),
    layout!(hgpui::PolychromeSprite, "PolychromeSprite", order, grayscale, opacity, corner_smoothing, bounds, content_mask, corner_radii, tile, transformation),
    layout!(PathSprite, "PathSprite", bounds),
    layout!(PathRasterizationVertex, "PathRasterizationVertex", xy_position, curve_position, color, bounds, content_mask, content_mask_corner_radii, content_mask_fade_out),
    layout!(GlobalUniforms, "GlobalUniforms", viewport_size, premultiplied_alpha, padding, window_mask_params, window_mask),
    layout!(FontRasterizationUniforms, "FontRasterizationUniforms", gamma_ratios, grayscale_enhanced_contrast, subpixel_enhanced_contrast, uses_blue_green_red_subpixel_order, padding),
    layout!(BlurUniforms, "BlurUniforms", bounds, content_mask, content_mask_radii, content_mask_fade_out, corner_radii, direction, sigma, opacity, tap_count, tap_step, clip_rounded, downsample),
    layout!(SurfaceUniforms, "SurfaceUniforms", bounds, content_mask, color_format, opacity, padding0, padding1, padding2, padding3, padding4, padding5),
];

/// Scene structs that are laid out as plain scalars/vectors on the GPU.
pub fn is_scalar_wrapper<T>() -> bool {
    std::mem::size_of::<T>() == 4
}

const _: () = {
    assert!(std::mem::size_of::<AbiBackground>() == std::mem::size_of::<hgpui::Background>());
    assert!(std::mem::size_of::<hgpui::Background>() % 8 == 0);
    assert!(std::mem::size_of::<hgpui::Quad>() == 304);
    assert!(std::mem::offset_of!(hgpui::Quad, transformation) == 280);
    assert!(std::mem::offset_of!(hgpui::TransformationMatrix, translation) == 16);
    assert!(std::mem::size_of::<hgpui::Shadow>() == 208);
    assert!(std::mem::size_of::<hgpui::Underline>() == 120);
    assert!(std::mem::size_of::<hgpui::MonochromeSprite>() == 144);
    assert!(std::mem::size_of::<hgpui::SubpixelSprite>() == 144);
    assert!(std::mem::size_of::<hgpui::PolychromeSprite>() == 152);
};
