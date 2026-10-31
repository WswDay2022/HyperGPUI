#[wgsl_rs::wgsl]
mod source {
    use wgsl_rs::std::*;

    // Defined here as a literal so the Rust-to-WGSL translator emits it, not `std`.
    #[allow(clippy::approx_constant)]
    pub const PI: f32 = 3.141592653589793;
    pub const HALF_TURN_DEGREES: f32 = 180.0;
    pub const FULL_TURN_DEGREES: f32 = 360.0;
    pub const CSS_GRADIENT_OFFSET_DEGREES: f32 = 90.0;
    pub const PIXEL_ANTIALIAS_RADIUS: f32 = 0.5;
    pub const GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS: f32 = 3.0;
    pub const PATTERN_COMPONENT_SCALE: f32 = 255.0;
    pub const PATTERN_PACKING_RADIX: f32 = 65535.0;
    pub const MIN_NORMALIZED_WEIGHT: f32 = 0.00001;
    pub const MIN_PATH_GRADIENT: f32 = 0.001;
    pub const UNDERLINE_WAVE_FREQUENCY: f32 = 2.0;
    pub const UNDERLINE_WAVE_HEIGHT_RATIO: f32 = 0.8;
    pub const GRADIENT_DITHER_SCALE: f32 = 0.6180339887;
    pub const GRADIENT_DITHER_RGB_STRENGTH: f32 = 2.0 / PATTERN_COMPONENT_SCALE;
    pub const GRADIENT_DITHER_ALPHA_STRENGTH: f32 = 3.0 / PATTERN_COMPONENT_SCALE;
    pub const GRADIENT_DITHER_SEED_A: Vec2f = vec2f(12.9898, 78.233);
    pub const GRADIENT_DITHER_SEED_B: Vec2f = vec2f(39.3460, 11.135);
    pub const GRADIENT_DITHER_MULTIPLIER_A: f32 = 43758.5453;
    pub const GRADIENT_DITHER_MULTIPLIER_B: f32 = 24634.6345;
    pub const DIAGONAL_STRIPE_ANGLE: f32 = PI / 4.0;
    pub const REC_601_LUMA_WEIGHTS: Vec3f = vec3f(0.30, 0.59, 0.11);
    pub const LINEAR_RGB_LUMA_WEIGHTS: Vec3f = vec3f(0.2126, 0.7152, 0.0722);
    pub const ERROR_FUNCTION_LINEAR_COEFFICIENT: f32 = 0.278393;
    pub const ERROR_FUNCTION_QUADRATIC_COEFFICIENT: f32 = 0.230389;
    pub const ERROR_FUNCTION_CUBIC_COEFFICIENT: f32 = 0.000972;
    pub const ERROR_FUNCTION_QUARTIC_COEFFICIENT: f32 = 0.078108;
    pub const DARK_TEXT_CONTRAST_SCALE: f32 = 4.0;
    pub const DARK_TEXT_BRIGHTNESS_CUTOFF: f32 = 0.75;
    pub const LINEAR_SRGB_TO_CONE_RESPONSE: Mat3x3f = mat3x3f(
        vec3f(0.4122214708, 0.2119034982, 0.0883024619),
        vec3f(0.5363325363, 0.6806995451, 0.2817188376),
        vec3f(0.0514459929, 0.1073969566, 0.6299787005),
    );
    pub const CONE_RESPONSE_TO_OKLAB: Mat3x3f = mat3x3f(
        vec3f(0.2104542553, 1.9779984951, 0.0259040371),
        vec3f(0.7936177850, -2.4285922050, 0.7827717662),
        vec3f(-0.0040720468, 0.4505938995, -0.8086757660),
    );
    pub const OKLAB_TO_CONE_RESPONSE: Mat3x3f = mat3x3f(
        vec3f(1.0, 1.0, 1.0),
        vec3f(0.3963377774, -0.1055613458, -0.0894841775),
        vec3f(0.2158037573, -0.0638541728, -1.2914855480),
    );
    pub const CONE_RESPONSE_TO_LINEAR_SRGB: Mat3x3f = mat3x3f(
        vec3f(4.0767416621, -1.2684380046, -0.0041960863),
        vec3f(-3.3077115913, 2.6097574011, -0.7034186147),
        vec3f(0.2309699292, -0.3413193965, 1.7076147010),
    );

    #[repr(u32)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Wgsl)]
    pub enum ShaderBool {
        Disabled = 0,
        Enabled = 1,
    }

    #[repr(u32)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Wgsl)]
    pub enum BackgroundTag {
        Solid = 0,
        LinearGradient = 1,
        PatternSlash = 2,
        Checkerboard = 3,
        RadialGradient = 4,
        RadialGradientCircle = 5,
    }

    #[repr(u32)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Wgsl)]
    pub enum ColorSpace {
        Srgb = 0,
        Oklab = 1,
    }

    #[repr(u32)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Wgsl)]
    pub enum BorderStyle {
        Solid = 0,
        Dashed = 1,
    }

    #[repr(u32)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Wgsl)]
    pub enum SurfaceColorFormat {
        Rgba = 0,
        Yuv = 1,
    }

    #[repr(u32)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Wgsl)]
    pub enum BlurCompositeClip {
        None = 0,
        RoundedBounds = 1,
    }

    #[repr(u32)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Wgsl)]
    pub enum DownsampleMode {
        Copy = 0,
        HalfResolution = 1,
    }

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Wgsl)]
    pub struct GlobalUniforms {
        pub viewport_size: Vec2f,
        pub premultiplied_alpha: ShaderBool,
        pub padding: u32,
        /// `x` = 1 when `window_mask` is active, 0 otherwise; the rest is padding.
        pub window_mask_params: Vec4f,
        /// The window's outermost rounded clip in device pixels, evaluated by every fragment
        /// shader in addition to the primitive's own content mask. This keeps the true window
        /// arc for descendants whose intersected mask lost it (inset titlebar buttons).
        pub window_mask: ContentMask,
    }

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Wgsl)]
    pub struct FontRasterizationUniforms {
        pub gamma_ratios: Vec4f,
        pub grayscale_enhanced_contrast: f32,
        pub subpixel_enhanced_contrast: f32,
        pub uses_blue_green_red_subpixel_order: ShaderBool,
        pub padding: u32,
    }

    uniform!(group(0), binding(0), GLOBALS: GlobalUniforms);
    uniform!(group(0), binding(1), FONT_RASTERIZATION: FontRasterizationUniforms);

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Wgsl)]
    pub struct Bounds {
        pub origin: Vec2f,
        pub size: Vec2f,
    }

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Wgsl)]
    pub struct ContentMask {
        pub bounds: Bounds,
        pub corner_radii: Corners,
        pub fade_out: Edges,
    }

    // Flat arena of every distinct ancestor-mask chain used by the frame (`Scene::mask_chains`).
    // A primitive's `mask_chain` field packs the chain length in the top byte and its offset into
    // this arena in the low 24 bits; zero means "no chain".
    storage!(group(0), binding(2), CONTENT_MASKS: RuntimeArray<ContentMask>);

    impl ContentMask {
        /// The mask's coverage at the given window-space position: 1 inside,
        /// 0 outside, and a linear ramp to 0 across each edge's fade distance.
        pub fn alpha(content_mask: ContentMask, position: Vec2f) -> f32 {
            ContentMask::alpha_with_own(content_mask, position, false)
        }

        /// Like [`ContentMask::alpha`], but when the mask is the primitive's own cutout the
        /// shape's distance field is skipped: the primitive already applies it for its own
        /// outline, and multiplying the two half-covered edge pixels squares the coverage
        /// (0.5 × 0.5), which reads as a dark one-pixel rim on rounded elements. Edge fades
        /// still apply because they are not part of the outline.
        pub fn alpha_with_own(
            content_mask: ContentMask,
            position: Vec2f,
            owns_cutout: bool,
        ) -> f32 {
            let bounds = content_mask.bounds;
            let fade = content_mask.fade_out;
            let mut alpha = 1.0;
            if fade.left > 0.0 {
                alpha *= saturate((position.x - bounds.origin.x) / fade.left);
            }
            if fade.right > 0.0 {
                alpha *= saturate((bounds.origin.x + bounds.size.x - position.x) / fade.right);
            }
            if fade.top > 0.0 {
                alpha *= saturate((position.y - bounds.origin.y) / fade.top);
            }
            if fade.bottom > 0.0 {
                alpha *= saturate((bounds.origin.y + bounds.size.y - position.y) / fade.bottom);
            }
            if owns_cutout {
                return alpha;
            }
            alpha * antialiased_coverage(rounded_rectangle_signed_distance(
                position,
                bounds,
                content_mask.corner_radii,
            ))
        }
    }

    impl Bounds {
        pub fn position(bounds: Bounds, unit_position: Vec2f) -> Vec2f {
            bounds.origin + unit_position * bounds.size
        }

        pub fn half_size(bounds: Bounds) -> Vec2f {
            bounds.size / 2.0
        }

        pub fn center(bounds: Bounds) -> Vec2f {
            bounds.origin + Bounds::half_size(bounds)
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Wgsl)]
    pub struct Corners {
        pub top_left: f32,
        pub top_right: f32,
        pub bottom_right: f32,
        pub bottom_left: f32,
    }

    impl Corners {
        pub fn is_zero(corners: Corners) -> bool {
            corners.top_left == 0.0
                && corners.top_right == 0.0
                && corners.bottom_right == 0.0
                && corners.bottom_left == 0.0
        }
    }

    pub fn zero_corners() -> Corners {
        Corners {
            top_left: 0.0,
            top_right: 0.0,
            bottom_right: 0.0,
            bottom_left: 0.0,
        }
    }

    pub fn zero_edges() -> Edges {
        Edges {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        }
    }

    #[derive(Clone, Copy, PartialEq, Wgsl)]
    pub struct Edges {
        pub top: f32,
        pub right: f32,
        pub bottom: f32,
        pub left: f32,
    }

    impl Edges {
        pub fn is_zero(edges: Edges) -> bool {
            edges.top == 0.0 && edges.right == 0.0 && edges.bottom == 0.0 && edges.left == 0.0
        }
    }
    #[derive(Clone, Copy, Wgsl)]
    pub struct Hsla {
        pub h: f32,
        pub s: f32,
        pub l: f32,
        pub a: f32,
    }
    #[derive(Clone, Copy, Wgsl)]
    pub struct LinearColorStop {
        pub color: Hsla,
        pub percentage: f32,
    }
    #[derive(Clone, Copy, Wgsl)]
    pub struct Background {
        pub tag: BackgroundTag,
        pub color_space: ColorSpace,
        pub solid: Hsla,
        pub gradient_angle_or_pattern_height: f32,
        pub colors: [LinearColorStop; 2],
        /// Local radial-gradient center Y, packed into the old `pad` slot.
        pub radial_center_y: f32,
        /// Local radial-gradient size keyword (0 = farthest-corner, 1 = closest-side, ...).
        pub radial_size: u32,
        pub padding: u32,
    }
    #[derive(Clone, Copy, Wgsl)]
    pub struct AtlasTextureId {
        pub index: u32,
        pub kind: u32,
    }
    #[derive(Clone, Copy, Wgsl)]
    pub struct AtlasBounds {
        pub origin: Vec2i,
        pub size: Vec2i,
    }
    #[derive(Clone, Copy, Wgsl)]
    pub struct AtlasTile {
        pub texture_id: AtlasTextureId,
        pub tile_id: u32,
        pub padding: u32,
        pub bounds: AtlasBounds,
    }
    #[derive(Clone, Copy, Wgsl)]
    pub struct TransformationMatrix {
        pub rotation_scale: Mat2x2f,
        pub translation: Vec2f,
    }

    impl TransformationMatrix {
        pub fn transform_position(transform: TransformationMatrix, position: Vec2f) -> Vec2f {
            transpose(transform.rotation_scale) * position + transform.translation
        }
    }

    #[derive(Clone, Copy, Wgsl)]
    pub struct RectangleVertex {
        pub unit_position: Vec2f,
        pub viewport_position: Vec2f,
        pub clip_position: Vec4f,
    }

    #[derive(Clone, Copy, Wgsl)]
    pub struct FullscreenVertex {
        pub texture_coordinates: Vec2f,
        pub clip_position: Vec4f,
    }

    pub fn is_enabled(value: ShaderBool) -> bool {
        value == ShaderBool::Enabled
    }

    pub fn transparent() -> Vec4f {
        vec4f(0.0, 0.0, 0.0, 0.0)
    }

    /// One-device-pixel coverage ramp for a signed distance field.
    ///
    /// Upstream gpui's shaders use the same fixed `0.5` threshold: bounds are snapped to device
    /// pixels, so the distance field is already measured in device pixels. This is deliberately
    /// not derivative-based — `dpdx`/`dpdy` at the outer AA ring read helper lanes whose
    /// attributes are undefined on real GPUs, which produced unstable one-pixel rims.
    pub fn antialiased_coverage(signed_distance: f32) -> f32 {
        saturate(PIXEL_ANTIALIAS_RADIUS - signed_distance)
    }

    /// Largest scale factor of the 2x2 linear part (largest singular value).
    ///
    /// Distance fields of transformed primitives are evaluated in local space; multiplying the
    /// distance by this factor turns it into device pixels. Translations yield 1, so untransformed
    /// and translated UI matches upstream's `0.5 - d` exactly.
    pub fn aa_scale(transformation: TransformationMatrix) -> f32 {
        let axis_x = vec2f(
            transformation.rotation_scale[0usize][0usize],
            transformation.rotation_scale[1usize][0usize],
        );
        let axis_y = vec2f(
            transformation.rotation_scale[0usize][1usize],
            transformation.rotation_scale[1usize][1usize],
        );
        let a = dot(axis_x, axis_x);
        let b = dot(axis_x, axis_y);
        let c = dot(axis_y, axis_y);
        let discriminant = sqrt(max(0.25 * (a - c) * (a - c) + b * b, 0.0));
        sqrt(max(0.5 * (a + c) + discriminant, 0.0))
    }

    /// Coverage of a local-space distance field under `transformation`.
    pub fn antialiased_coverage_transformed(
        signed_distance: f32,
        transformation: TransformationMatrix,
    ) -> f32 {
        antialiased_coverage(signed_distance * aa_scale(transformation))
    }

    pub fn premultiply(color: Vec4f, coverage: f32) -> Vec4f {
        let alpha = color.w * coverage;
        vec4f(color.x * alpha, color.y * alpha, color.z * alpha, alpha)
    }

    pub fn atlas_texture_coordinates(
        unit_position: Vec2f,
        tile: AtlasTile,
        atlas_size: Vec2u,
    ) -> Vec2f {
        let tile_origin = vec2f(tile.bounds.origin.x as f32, tile.bounds.origin.y as f32);
        let tile_size = vec2f(tile.bounds.size.x as f32, tile.bounds.size.y as f32);
        let texture_size = vec2f(atlas_size.x as f32, atlas_size.y as f32);
        (tile_origin + unit_position * tile_size) / texture_size
    }

    /// Atlas lookup that stays on the tile's own texels.
    ///
    /// Bilinear filtering reconstructs between texel centres, so a sample coordinate within
    /// half a texel of the tile border also reads the neighbouring atlas tile (or the atlas'
    /// transparent background), which shows up as dark fringes around images and as stray
    /// strokes beside scaled glyphs. Clamping the *texel* coordinate to the centres of the
    /// tile's outermost texels keeps the footprint inside the tile while leaving 1:1 sampling
    /// exactly where it was. Interpolation at the tile's edge may still produce unit positions
    /// slightly outside 0..1; those clamp to the edge texel instead of spilling.
    pub fn atlas_texture_coordinates_clamped(
        unit_position: Vec2f,
        tile: AtlasTile,
        atlas_size: Vec2u,
    ) -> Vec2f {
        let tile_origin = vec2f(tile.bounds.origin.x as f32, tile.bounds.origin.y as f32);
        let tile_size = vec2f(tile.bounds.size.x as f32, tile.bounds.size.y as f32);
        let texture_size = vec2f(atlas_size.x as f32, atlas_size.y as f32);
        let half_texel = vec2f(0.5, 0.5);
        let texel_position = unit_position * tile_size;
        let clamped = clamp(
            texel_position,
            half_texel,
            max(tile_size - half_texel, half_texel),
        );
        (tile_origin + clamped) / texture_size
    }

    pub fn color_brightness(color: Vec3f) -> f32 {
        dot(color, REC_601_LUMA_WEIGHTS)
    }
    pub fn light_on_dark_contrast(enhanced_contrast: f32, color: Vec3f) -> f32 {
        let darkness = saturate(
            DARK_TEXT_CONTRAST_SCALE * (DARK_TEXT_BRIGHTNESS_CUTOFF - color_brightness(color)),
        );
        enhanced_contrast * darkness
    }

    pub fn enhance_contrast<T>(alpha: T, contrast: f32) -> T
    where
        T: Copy
            + std::ops::Mul<f32, Output = T>
            + std::ops::Add<f32, Output = T>
            + std::ops::Div<T, Output = T>,
    {
        alpha * (contrast + 1.0) / (alpha * contrast + 1.0)
    }

    pub fn apply_alpha_correction<T>(alpha: T, brightness: T, gamma_ratios: Vec4f) -> T
    where
        T: Copy
            + std::ops::Mul<f32, Output = T>
            + std::ops::Mul<T, Output = T>
            + std::ops::Add<f32, Output = T>
            + std::ops::Add<T, Output = T>,
        f32: std::ops::Sub<T, Output = T>,
    {
        let brightness_adjustment = brightness * gamma_ratios.x + gamma_ratios.y;
        let correction =
            brightness_adjustment * alpha + (brightness * gamma_ratios.z + gamma_ratios.w);
        alpha + alpha * (1.0 - alpha) * correction
    }
    pub fn apply_contrast_and_gamma_correction(
        sample: f32,
        color: Vec3f,
        enhanced_contrast_factor: f32,
        gamma_ratios: Vec4f,
    ) -> f32 {
        let enhanced_contrast = light_on_dark_contrast(enhanced_contrast_factor, color);
        apply_alpha_correction::<f32>(
            enhance_contrast::<f32>(sample, enhanced_contrast),
            color_brightness(color),
            gamma_ratios,
        )
    }
    pub fn apply_contrast_and_gamma_correction3(
        sample: Vec3f,
        color: Vec3f,
        enhanced_contrast_factor: f32,
        gamma_ratios: Vec4f,
    ) -> Vec3f {
        let contrasted = enhance_contrast::<Vec3f>(
            sample,
            light_on_dark_contrast(enhanced_contrast_factor, color),
        );
        apply_alpha_correction::<Vec3f>(contrasted, color, gamma_ratios)
    }
    pub fn rectangle_corner(vertex_id: u32) -> Vec2f {
        vec2f((vertex_id & 1u32) as f32, 0.5 * (vertex_id & 2u32) as f32)
    }

    pub fn viewport_to_clip_position(position: Vec2f) -> Vec4f {
        let clip_position =
            position / get!(GLOBALS).viewport_size * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0);
        vec4f(clip_position.x, clip_position.y, 0.0, 1.0)
    }

    pub fn rectangle_vertex(vertex_id: u32, bounds: Bounds) -> RectangleVertex {
        let unit_position = rectangle_corner(vertex_id);
        let viewport_position = Bounds::position(bounds, unit_position);
        RectangleVertex {
            unit_position,
            viewport_position,
            clip_position: viewport_to_clip_position(viewport_position),
        }
    }

    pub fn transformed_rectangle_vertex(
        vertex_id: u32,
        bounds: Bounds,
        transform: TransformationMatrix,
    ) -> RectangleVertex {
        let unit_position = rectangle_corner(vertex_id);
        let viewport_position = TransformationMatrix::transform_position(
            transform,
            Bounds::position(bounds, unit_position),
        );
        RectangleVertex {
            unit_position,
            viewport_position,
            clip_position: viewport_to_clip_position(viewport_position),
        }
    }

    pub fn fullscreen_vertex(vertex_id: u32) -> FullscreenVertex {
        let texture_coordinates = vec2f(
            ((vertex_id << 1u32) & 2u32) as f32,
            (vertex_id & 2u32) as f32,
        );
        FullscreenVertex {
            texture_coordinates,
            clip_position: vec4f(
                texture_coordinates.x * 2.0 - 1.0,
                1.0 - texture_coordinates.y * 2.0,
                0.0,
                1.0,
            ),
        }
    }

    pub fn clip_distances(position: Vec2f, clip_bounds: Bounds) -> Vec4f {
        let distance_from_top_left = position - clip_bounds.origin;
        let distance_from_bottom_right = clip_bounds.origin + clip_bounds.size - position;
        vec4f(
            distance_from_top_left.x,
            distance_from_bottom_right.x,
            distance_from_top_left.y,
            distance_from_bottom_right.y,
        )
    }

    pub fn is_clipped(distances: Vec4f) -> bool {
        min(min(distances.x, distances.y), min(distances.z, distances.w)) < 0.0
    }

    pub fn unclipped_distances() -> Vec4f {
        vec4f(1.0, 1.0, 1.0, 1.0)
    }
    pub fn hue_to_unit_rgb(hue: f32) -> Vec3f {
        let phases = fract(vec3f(hue, hue + 2.0 / 3.0, hue + 1.0 / 3.0));
        saturate(abs(phases * 6.0 - 3.0) - 1.0)
    }

    pub fn hsla_to_rgba(hsla: Hsla) -> Vec4f {
        let chroma = hsla.s * (1.0 - abs(2.0 * hsla.l - 1.0));
        let red_green_blue = (hue_to_unit_rgb(hsla.h) - 0.5) * chroma + hsla.l;
        vec4f(red_green_blue.x, red_green_blue.y, red_green_blue.z, hsla.a)
    }
    pub fn linear_srgb_to_oklab(color: Vec4f) -> Vec4f {
        // Keeps the retired Metal shader's 2.2 transfer curve for pixel parity.
        let linear_srgb = pow(color.rgb(), vec3f(2.2, 2.2, 2.2));
        let cone_response = LINEAR_SRGB_TO_CONE_RESPONSE * linear_srgb;
        let cube_root = vec3f(1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0);
        let oklab = CONE_RESPONSE_TO_OKLAB * pow(cone_response, cube_root);
        vec4f(oklab.x, oklab.y, oklab.z, color.w)
    }
    pub fn oklab_to_linear_srgb(color: Vec4f) -> Vec4f {
        let cone_root = OKLAB_TO_CONE_RESPONSE * color.rgb();
        let cone_response = cone_root * cone_root * cone_root;
        let linear_srgb = CONE_RESPONSE_TO_LINEAR_SRGB * cone_response;
        let srgb = pow(linear_srgb, vec3f(1.0 / 2.2, 1.0 / 2.2, 1.0 / 2.2));
        vec4f(srgb.x, srgb.y, srgb.z, color.w)
    }
    pub fn over(below: Vec4f, above: Vec4f) -> Vec4f {
        let alpha = above.w + below.w * (1.0 - above.w);
        if alpha == 0.0 {
            return transparent();
        }
        let red_green_blue =
            (above.rgb() * above.w + below.rgb() * below.w * (1.0 - above.w)) / alpha;
        vec4f(red_green_blue.x, red_green_blue.y, red_green_blue.z, alpha)
    }
    pub fn gaussian(position: f32, standard_deviation: f32) -> f32 {
        let variance = standard_deviation * standard_deviation;
        exp(-(position * position) / (2.0 * variance)) / (sqrt(2.0 * PI) * standard_deviation)
    }
    pub fn approximate_error_function(value: Vec2f) -> Vec2f {
        let signs = sign(value);
        let absolute_value = abs(value);
        let polynomial = 1.0
            + absolute_value
                * (ERROR_FUNCTION_LINEAR_COEFFICIENT
                    + absolute_value
                        * (ERROR_FUNCTION_QUADRATIC_COEFFICIENT
                            + absolute_value
                                * (ERROR_FUNCTION_CUBIC_COEFFICIENT
                                    + absolute_value * ERROR_FUNCTION_QUARTIC_COEFFICIENT)));
        let polynomial_squared = polynomial * polynomial;
        signs - signs / (polynomial_squared * polynomial_squared)
    }
    pub fn integrated_rounded_rectangle_coverage(
        horizontal_position: f32,
        vertical_position: f32,
        standard_deviation: f32,
        corner_radius: f32,
        half_size: Vec2f,
    ) -> f32 {
        let distance_beyond_corner = min(half_size.y - corner_radius - abs(vertical_position), 0.0);
        let curved_extent = half_size.x - corner_radius
            + sqrt(max(
                0.0,
                corner_radius * corner_radius - distance_beyond_corner * distance_beyond_corner,
            ));
        let integration_bounds = horizontal_position + vec2f(-curved_extent, curved_extent);
        let normalized_bounds = integration_bounds * (sqrt(0.5) / standard_deviation);
        let integral = 0.5 + 0.5 * approximate_error_function(normalized_bounds);
        integral.y - integral.x
    }
    pub fn pick_corner_radius(center_to_point: Vec2f, radii: Corners) -> f32 {
        let left = select(radii.bottom_left, radii.top_left, center_to_point.y < 0.0);
        let right = select(radii.bottom_right, radii.top_right, center_to_point.y < 0.0);
        select(right, left, center_to_point.x < 0.0)
    }
    pub fn rounded_rectangle_signed_distance_from_corner(
        corner_center_to_point: Vec2f,
        corner_radius: f32,
    ) -> f32 {
        if corner_radius == 0.0 {
            max(corner_center_to_point.x, corner_center_to_point.y)
        } else {
            length(max(vec2f(0.0, 0.0), corner_center_to_point))
                + min(0.0, max(corner_center_to_point.x, corner_center_to_point.y))
                - corner_radius
        }
    }
    pub fn rounded_rectangle_signed_distance(
        point: Vec2f,
        bounds: Bounds,
        corner_radii: Corners,
    ) -> f32 {
        let half_size = Bounds::half_size(bounds);
        let center_to_point = point - Bounds::center(bounds);
        let corner_radius = pick_corner_radius(center_to_point, corner_radii);
        rounded_rectangle_signed_distance_from_corner(
            abs(center_to_point) - half_size + corner_radius,
            corner_radius,
        )
    }

    pub fn gaussian_signed_distance_coverage(distance: f32, standard_deviation: f32) -> f32 {
        let normalized = distance / (sqrt(2.0) * standard_deviation);
        saturate(0.5 - 0.5 * approximate_error_function(vec2f(normalized, normalized)).x)
    }

    /// Coverage of the window's own rounded shape at a scene-space position. One when the
    /// application does not give its window a rounded mask.
    pub fn window_mask_alpha(position: Vec2f) -> f32 {
        if get!(GLOBALS).window_mask_params.x < 0.5 {
            return 1.0;
        }
        ContentMask::alpha(get!(GLOBALS).window_mask, position)
    }

    pub fn blend_color(color: Vec4f, alpha_factor: f32) -> Vec4f {
        let alpha = color.w * alpha_factor;
        let multiplier = select(1.0, alpha, is_enabled(get!(GLOBALS).premultiplied_alpha));
        vec4f(
            color.x * multiplier,
            color.y * multiplier,
            color.z * multiplier,
            alpha,
        )
    }

    #[derive(Clone, Copy, Wgsl)]
    pub struct Paint {
        pub background: Background,
        pub bounds: Bounds,
    }

    impl Paint {
        pub fn new(background: Background, bounds: Bounds) -> Paint {
            Paint { background, bounds }
        }
    }

    #[derive(Clone, Copy, Wgsl)]
    pub struct PreparedPaint {
        pub solid: Vec4f,
        pub color0: Vec4f,
        pub color1: Vec4f,
    }

    impl PreparedPaint {
        pub fn new(solid: Vec4f, color0: Vec4f, color1: Vec4f) -> PreparedPaint {
            PreparedPaint {
                solid,
                color0,
                color1,
            }
        }
    }

    pub fn prepare_paint(paint: Paint) -> PreparedPaint {
        let mut prepared = PreparedPaint::new(transparent(), transparent(), transparent());

        if paint.background.tag == BackgroundTag::LinearGradient
            || paint.background.tag == BackgroundTag::RadialGradient
            || paint.background.tag == BackgroundTag::RadialGradientCircle
        {
            prepared.color0 = hsla_to_rgba(paint.background.colors[0usize].color);
            prepared.color1 = hsla_to_rgba(paint.background.colors[1usize].color);
            // Upstream gpui interpolates sRGB stops in their native (non-linear) space.
            // Only the Oklab color space needs the stop colors converted up front.
            if paint.background.color_space == ColorSpace::Oklab {
                prepared.color0 = linear_srgb_to_oklab(prepared.color0);
                prepared.color1 = linear_srgb_to_oklab(prepared.color1);
            }
        } else {
            prepared.solid = hsla_to_rgba(paint.background.solid);
        }

        prepared
    }

    pub fn linear_gradient_ratio(paint: Paint, position: Vec2f) -> f32 {
        let radians = (paint.background.gradient_angle_or_pattern_height % FULL_TURN_DEGREES
            - CSS_GRADIENT_OFFSET_DEGREES)
            * PI
            / HALF_TURN_DEGREES;
        let mut direction = vec2f(cos(radians), sin(radians));
        if paint.bounds.size.x > paint.bounds.size.y {
            direction.y *= paint.bounds.size.y / paint.bounds.size.x;
        } else {
            direction.x *= paint.bounds.size.x / paint.bounds.size.y;
        }

        let half_size = Bounds::half_size(paint.bounds);
        let mut ratio = dot(position - Bounds::center(paint.bounds), direction) / length(direction);
        if abs(direction.x) > abs(direction.y) {
            ratio = (ratio + half_size.x) / paint.bounds.size.x;
        } else {
            ratio = (ratio + half_size.y) / paint.bounds.size.y;
        }

        let first_stop = paint.background.colors[0usize].percentage;
        let last_stop = paint.background.colors[1usize].percentage;
        saturate((ratio - first_stop) / (last_stop - first_stop))
    }

    pub fn gradient_dither(position: Vec2f) -> Vec4f {
        let seed = position * GRADIENT_DITHER_SCALE;
        let noise_a = fract(sin(dot(seed, GRADIENT_DITHER_SEED_A)) * GRADIENT_DITHER_MULTIPLIER_A);
        let noise_b = fract(sin(dot(seed, GRADIENT_DITHER_SEED_B)) * GRADIENT_DITHER_MULTIPLIER_B);
        let triangular_noise = noise_a + noise_b - 1.0;
        vec4f(
            triangular_noise * GRADIENT_DITHER_RGB_STRENGTH,
            triangular_noise * GRADIENT_DITHER_RGB_STRENGTH,
            triangular_noise * GRADIENT_DITHER_RGB_STRENGTH,
            triangular_noise * GRADIENT_DITHER_ALPHA_STRENGTH,
        )
    }

    pub fn linear_gradient_color(paint: Paint, position: Vec2f, prepared: PreparedPaint) -> Vec4f {
        let ratio = linear_gradient_ratio(paint, position);
        let ratio4 = vec4f(ratio, ratio, ratio, ratio);
        let interpolated = mix(prepared.color0, prepared.color1, ratio4);
        let mut color = interpolated;
        if paint.background.color_space == ColorSpace::Oklab {
            color = oklab_to_linear_srgb(interpolated);
        }
        color + gradient_dither(position)
    }

    pub fn slash_pattern_color(paint: Paint, position: Vec2f, solid: Vec4f) -> Vec4f {
        let encoded = paint.background.gradient_angle_or_pattern_height;
        let pattern_width = (encoded / PATTERN_PACKING_RADIX) / PATTERN_COMPONENT_SCALE;
        let pattern_interval = (encoded % PATTERN_PACKING_RADIX) / PATTERN_COMPONENT_SCALE;
        let pattern_height = pattern_width + pattern_interval;
        let period = pattern_height * sin(DIAGONAL_STRIPE_ANGLE);
        let rotation = mat2x2f(
            vec2f(cos(DIAGONAL_STRIPE_ANGLE), -sin(DIAGONAL_STRIPE_ANGLE)),
            vec2f(sin(DIAGONAL_STRIPE_ANGLE), cos(DIAGONAL_STRIPE_ANGLE)),
        );
        let pattern = (rotation * (position - paint.bounds.origin)).x % period;
        let distance =
            min(pattern, period - pattern) - period * (pattern_width / pattern_height) / 2.0;
        let mut color = solid;
        color.w *= antialiased_coverage(distance);
        color
    }

    pub fn checkerboard_color(paint: Paint, position: Vec2f, solid: Vec4f) -> Vec4f {
        let relative = position - paint.bounds.origin;
        let square_size = paint.background.gradient_angle_or_pattern_height;
        let colored = (floor(relative.x / square_size) + floor(relative.y / square_size)) % 2.0;
        let mut color = solid;
        color.w *= saturate(colored);
        color
    }

    pub fn paint_color(paint: Paint, position: Vec2f, prepared: PreparedPaint) -> Vec4f {
        let mut color = prepared.solid;
        #[wgsl_allow(non_literal_match_statement_patterns)]
        match paint.background.tag {
            BackgroundTag::Solid => {
                color = prepared.solid;
            }
            BackgroundTag::LinearGradient => {
                color = linear_gradient_color(paint, position, prepared);
            }
            BackgroundTag::PatternSlash => {
                color = slash_pattern_color(paint, position, prepared.solid);
            }
            BackgroundTag::Checkerboard => {
                color = checkerboard_color(paint, position, prepared.solid);
            }
            BackgroundTag::RadialGradient => {
                color = radial_gradient_color(paint, position, prepared);
            }
            BackgroundTag::RadialGradientCircle => {
                color = radial_gradient_color(paint, position, prepared);
            }
        }
        color
    }
    pub fn gradient_ramp_color(
        paint: Paint,
        t: f32,
        position: Vec2f,
        prepared: PreparedPaint,
    ) -> Vec4f {
        let stop_t = clamp(
            (t - paint.background.colors[0usize].percentage)
                / (paint.background.colors[1usize].percentage
                    - paint.background.colors[0usize].percentage),
            0.0,
            1.0,
        );
        let ratio = vec4f(stop_t, stop_t, stop_t, stop_t);
        let interpolated = mix(prepared.color0, prepared.color1, ratio);
        let mut color = interpolated;
        if paint.background.color_space == ColorSpace::Oklab {
            color = oklab_to_linear_srgb(interpolated);
        }
        color + gradient_dither(position)
    }

    /// Local radial-gradient extension. Kept outside upstream's four paint tags so
    /// `paint_color` can dispatch it without changing the shared linear/pattern paths.
    pub fn radial_gradient_color(paint: Paint, position: Vec2f, prepared: PreparedPaint) -> Vec4f {
        let background = paint.background;
        let bounds = paint.bounds;
        let center = bounds.origin
            + vec2f(
                background.gradient_angle_or_pattern_height,
                background.radial_center_y,
            ) * bounds.size;
        let offset = position - center;
        let is_closest = background.radial_size == 1u32 || background.radial_size == 3u32;
        let is_corner = background.radial_size == 0u32 || background.radial_size == 3u32;
        let center_from_origin = abs(center - bounds.origin);
        let origin_from_center = abs(bounds.origin + bounds.size - center);
        let closest_side = min(center_from_origin, origin_from_center);
        let farthest_side = max(center_from_origin, origin_from_center);
        let side = select(farthest_side, closest_side, is_closest);
        let mut radii = select(side, side * 1.41421356, is_corner);
        if background.tag == BackgroundTag::RadialGradientCircle {
            let side_radius = select(max(side.x, side.y), min(side.x, side.y), is_closest);
            radii = vec2f(
                select(side_radius, length(side), is_corner),
                select(side_radius, length(side), is_corner),
            );
        }
        let mut t = 1.0;
        if radii.x > 0.0 && radii.y > 0.0 {
            t = length(offset / radii);
        }
        gradient_ramp_color(paint, t, position, prepared)
    }

    // --- Local CSS-transform and rounded-mask helpers -------------------------------
    // These are the only shader-side additions over upstream's algorithms. They are
    // opt-in: upstream draw paths never call them.

    pub fn same_origin(a: Bounds, b: Bounds) -> bool {
        a.origin.x == b.origin.x && a.origin.y == b.origin.y
    }

    pub fn same_size(a: Bounds, b: Bounds) -> bool {
        a.size.x == b.size.x && a.size.y == b.size.y
    }

    pub fn content_mask_fade(mask_bounds: Bounds, fade_out: Edges, position: Vec2f) -> f32 {
        let mut alpha = 1.0;
        if fade_out.left > 0.0 {
            alpha *= saturate((position.x - mask_bounds.origin.x) / fade_out.left);
        }
        if fade_out.right > 0.0 {
            alpha *= saturate(
                (mask_bounds.origin.x + mask_bounds.size.x - position.x) / fade_out.right,
            );
        }
        if fade_out.top > 0.0 {
            alpha *= saturate((position.y - mask_bounds.origin.y) / fade_out.top);
        }
        if fade_out.bottom > 0.0 {
            alpha *= saturate(
                (mask_bounds.origin.y + mask_bounds.size.y - position.y) / fade_out.bottom,
            );
        }
        alpha
    }

    /// Whether `mask` is exactly the primitive's own outline.
    pub fn mask_is_own_cutout(mask: ContentMask, own_bounds: Bounds, own_radii: Corners) -> bool {
        same_origin(mask.bounds, own_bounds)
            && same_size(mask.bounds, own_bounds)
            && mask.corner_radii.top_left == own_radii.top_left
            && mask.corner_radii.top_right == own_radii.top_right
            && mask.corner_radii.bottom_left == own_radii.bottom_left
            && mask.corner_radii.bottom_right == own_radii.bottom_right
    }

    /// Exact clip coverage at `position` for a chain of nested content masks.
    ///
    /// Every raw ancestor mask (and the window's own shaped clip) contributes its rounded-rect
    /// signed distance; the clip region is their intersection, so the distances combine with
    /// `max` and antialiasing is applied **once** to the combined field. This is what preserves
    /// a parent's corner arc when a child is inset from it — the CPU-side `intersect` can only
    /// approximate that arc with a single rounded rectangle.
    ///
    /// `owns_cutout` lets a primitive that already antialiases its own outline (a rounded quad,
    /// sprite or shadow) exclude masks identical to that outline. Such a mask carries no extra
    /// clipping information, and multiplying its half-covered edge pixel with the primitive's
    /// own half-covered pixel would darken the rim (0.5 × 0.5).
    /// Combined clip field of every active mask: the largest signed distance plus the product
    /// of the edge fades. Callers that also have their own outline combine this distance with
    /// their own device-space distance via `max` and antialias once, so an ancestor mask whose
    /// edge coincides with the primitive's edge does not darken it (two 0.5 coverages multiplied
    /// would give 0.25).
    #[derive(Clone, Copy, Wgsl)]
    pub struct MaskChainSample {
        pub distance: f32,
        pub fade: f32,
    }

    pub fn content_mask_chain_sample(
        chain: u32,
        position: Vec2f,
        single_mask: ContentMask,
        // Position at which the legacy single mask is evaluated (local space for a quad whose
        // mask is its own outline, scene space otherwise).
        single_mask_position: Vec2f,
        own_bounds: Bounds,
        own_radii: Corners,
        owns_cutout: bool,
    ) -> MaskChainSample {
        content_mask_chain_sample_with_window(
            chain,
            position,
            single_mask,
            single_mask_position,
            own_bounds,
            own_radii,
            owns_cutout,
            true,
        )
    }

    /// Like [`content_mask_chain_sample`], but can skip the window's global rounded mask.
    /// Used by the element's own drop shadow when it is explicitly exempt from the window clip.
    pub fn content_mask_chain_sample_with_window(
        chain: u32,
        position: Vec2f,
        single_mask: ContentMask,
        single_mask_position: Vec2f,
        own_bounds: Bounds,
        own_radii: Corners,
        owns_cutout: bool,
        apply_window_mask: bool,
    ) -> MaskChainSample {
        let count = chain >> 24u32;
        let offset = chain & 16777215u32;
        let window_active = get!(GLOBALS).window_mask_params.x >= 0.5;

        let mut fade = 1.0;
        // `-1e30` is far inside every mask; if every mask is skipped as the primitive's own
        // cutout (or no mask is bound) the coverage stays exactly 1.
        let mut distance = -1.0e30;
        if count == 0u32 {
            // No chain: either no clip scope is active at all, or this is content built outside
            // the normal painter (tests / backends that do not upload the arena). Fall back to
            // the single pre-intersected mask, which is the legacy behaviour.
            fade = content_mask_fade(single_mask.bounds, single_mask.fade_out, single_mask_position);
            if !(owns_cutout && mask_is_own_cutout(single_mask, own_bounds, own_radii)) {
                distance = rounded_rectangle_signed_distance(
                    single_mask_position,
                    single_mask.bounds,
                    single_mask.corner_radii,
                );
            }
        } else {
            let mut index = 0u32;
            while index < count {
                let mask = get!(CONTENT_MASKS)[(offset + index) as usize];
                fade = fade * content_mask_fade(mask.bounds, mask.fade_out, position);
                if !(owns_cutout && mask_is_own_cutout(mask, own_bounds, own_radii)) {
                    distance = max(
                        distance,
                        rounded_rectangle_signed_distance(position, mask.bounds, mask.corner_radii),
                    );
                }
                index = index + 1u32;
            }
        }

        if apply_window_mask && window_active {
            let mask = get!(GLOBALS).window_mask;
            fade = fade * content_mask_fade(mask.bounds, mask.fade_out, position);
            if !(owns_cutout && mask_is_own_cutout(mask, own_bounds, own_radii)) {
                distance = max(
                    distance,
                    rounded_rectangle_signed_distance(position, mask.bounds, mask.corner_radii),
                );
            }
        }

        MaskChainSample { distance, fade }
    }

    pub fn content_mask_chain_alpha(
        chain: u32,
        position: Vec2f,
        single_mask: ContentMask,
        single_mask_position: Vec2f,
        own_bounds: Bounds,
        own_radii: Corners,
        owns_cutout: bool,
    ) -> f32 {
        let sample = content_mask_chain_sample(
            chain,
            position,
            single_mask,
            single_mask_position,
            own_bounds,
            own_radii,
            owns_cutout,
        );
        antialiased_coverage(sample.distance) * sample.fade
    }

    pub fn content_mask_coverage(
        mask_point: Vec2f,
        mask_bounds: Bounds,
        mask_radii: Corners,
        mask_fade_out: Edges,
        own_origin: Vec2f,
        own_size: Vec2f,
        own_radii: Corners,
        owns_cutout: bool,
    ) -> f32 {
        let fade = content_mask_fade(mask_bounds, mask_fade_out, mask_point);
        if owns_cutout
            && same_origin(mask_bounds, Bounds { origin: own_origin, size: own_size })
            && same_size(mask_bounds, Bounds { origin: own_origin, size: own_size })
            && mask_radii.top_left == own_radii.top_left
            && mask_radii.top_right == own_radii.top_right
            && mask_radii.bottom_left == own_radii.bottom_left
            && mask_radii.bottom_right == own_radii.bottom_right
        {
            return fade;
        }
        antialiased_coverage(rounded_rectangle_signed_distance(
            mask_point,
            mask_bounds,
            mask_radii,
        )) * fade
    }

    pub fn distance_from_clip_rect_impl(position: Vec2f, clip_bounds: Bounds) -> Vec4f {
        let top_left = position - clip_bounds.origin;
        let bottom_right = clip_bounds.origin + clip_bounds.size - position;
        vec4f(top_left.x, bottom_right.x, top_left.y, bottom_right.y)
    }

    pub fn to_device_position_impl(position: Vec2f) -> Vec4f {
        viewport_to_clip_position(position)
    }

    pub fn to_device_position(unit_vertex: Vec2f, bounds: Bounds) -> Vec4f {
        to_device_position_impl(Bounds::position(bounds, unit_vertex))
    }

    pub fn distance_from_clip_rect(
        unit_vertex: Vec2f,
        bounds: Bounds,
        clip_bounds: Bounds,
    ) -> Vec4f {
        distance_from_clip_rect_impl(Bounds::position(bounds, unit_vertex), clip_bounds)
    }

    /// Coverage of a distance field, one device pixel wide. Kept for the local blur
    /// shader, whose edge math predates upstream's `antialiased_coverage`.
    pub fn aa_coverage(signed_distance: f32, per_pixel: f32) -> f32 {
        saturate(0.5 - signed_distance / max(per_pixel, 1e-5))
    }

    pub fn quad_sdf_impl(corner_center_to_point: Vec2f, corner_radius: f32) -> f32 {
        rounded_rectangle_signed_distance_from_corner(corner_center_to_point, corner_radius)
    }

    pub fn quad_sdf(point: Vec2f, bounds: Bounds, corner_radii: Corners) -> f32 {
        rounded_rectangle_signed_distance(point, bounds, corner_radii)
    }

    /// Reflects `p` back into `rect` along both axes, used by the local blur
    /// downsample pass so taps outside the element repeat its edge content.
    pub fn mirror_into_rect(p: Vec2f, rect: Bounds) -> Vec2f {
        if rect.size.x <= 0.0 || rect.size.y <= 0.0 {
            return p;
        }
        let t = (p - rect.origin) / rect.size;
        let m = t - 2.0 * floor(t * 0.5);
        rect.origin + (vec2f(1.0, 1.0) - abs(m - vec2f(1.0, 1.0))) * rect.size
    }

    pub fn corner_dash_velocity(first: f32, second: f32) -> f32 {
        if first == 0.0 {
            second
        } else if second == 0.0 {
            first
        } else {
            min(first, second)
        }
    }
    pub fn signed_modulo(value: f32, modulus: f32) -> f32 {
        value - modulus * trunc(value / modulus)
    }
    pub fn dash_coverage(position: f32, period: f32, dash_length: f32, dash_velocity: f32) -> f32 {
        let half_period = period / 2.0;
        let half_dash_length = dash_length / 2.0;
        let centered =
            signed_modulo(position + half_period - half_dash_length, period) - half_period;
        let signed_distance = abs(centered) - half_dash_length;
        antialiased_coverage(signed_distance / dash_velocity)
    }
    pub fn quarter_ellipse_signed_distance(point: Vec2f, radii: Vec2f) -> f32 {
        (length(point / radii) - 1.0) * (radii.x + radii.y) * -0.5
    }
}

pub use source::*;
