use palette::{IntoColor, OklabHue, Oklcha, RgbHue};
use schemars::{JsonSchema, json_schema};
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};

// Re-exported for api maintenance
pub use palette::{Hsla, rgb::Rgba};

/// Convert an RGB hex color code number to a color type
pub const fn rgb(hex: u32) -> Rgba {
    let [_, r, g, b] = hex.to_be_bytes();
    Rgba {
        color: palette::rgb::Rgb::new(
            (r as f32) / 255.0,
            (g as f32) / 255.0,
            (b as f32) / 255.0
        ),
        alpha: 1.0,
    }
}

/// Convert an RGBA hex color code number to [`Rgba`]
pub const fn rgba(hex: u32) -> Rgba {
    let [r, g, b, a] = hex.to_be_bytes();
    Rgba::new(
        (r as f32) / 255.0,
        (g as f32) / 255.0,
        (b as f32) / 255.0,
        (a as f32) / 255.0
    )
}

/// Construct an [`Rgba`] from 8-bit sRGB channels and an 8-bit alpha.
///
/// Unlike [`rgba`] (which takes a packed `0xRRGGBBAA` number), each channel is
/// given separately in the usual `0..=255` range: `rgba8(255, 0, 0, 255)` is
/// opaque red.
pub const fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Rgba {
    Rgba::new(
        (r as f32) / 255.0,
        (g as f32) / 255.0,
        (b as f32) / 255.0,
        (a as f32) / 255.0,
    )
}

/// Construct an opaque [`Rgba`] from 8-bit sRGB channels.
///
/// The companion of [`rgb`] (packed hex) with explicit `0..=255` channels:
/// `rgb8(255, 0, 0)` is opaque red.
pub const fn rgb8(r: u8, g: u8, b: u8) -> Rgba {
    rgba8(r, g, b, 255)
}

/// Convert an sRGB color to GPUI's HSL-with-alpha representation.
///
/// This is the explicit conversion boundary for APIs that store [`Hsla`]. It
/// avoids exposing Palette's conversion traits to applications and examples.
pub fn rgb_to_hsla(color: Rgba) -> Hsla {
    color.into_color()
}

/// Convert GPUI's HSL-with-alpha representation to sRGB.
pub fn hsla_to_rgba(color: Hsla) -> Rgba {
    color.into_color()
}

/// Swap from RGBA with premultiplied alpha to BGRA
pub fn swap_rgba_pa_to_bgra(color: &mut [u8]) {
    color.swap(0, 2);
    if color[3] > 0 {
        let a = color[3] as f32 / 255.;
        color[0] = (color[0] as f32 / a) as u8;
        color[1] = (color[1] as f32 / a) as u8;
        color[2] = (color[2] as f32 / a) as u8;
    }
}

/// Construct an [`Hsla`] object from plain values.
///
/// Units: `h` is a fraction of the color wheel (`0.0..=1.0`, where `0.25` is
/// 90°), `s`, `l` and `a` are `0.0..=1.0`. All values are clamped.
pub const fn hsla(h: f32, s: f32, l: f32, a: f32) -> Hsla {
    Hsla {
        color: palette::Hsl::new_const(
            // `RgbHue` stores degrees, so the 0..1 fraction of the circle needs
            // scaling to 0..360 before it's wrapped.
            RgbHue::new(h.clamp(0., 1.) * 360.),
            s.clamp(0., 1.),
            l.clamp(0., 1.),
        ),
        alpha: a.clamp(0., 1.),
    }
}

/// Constructs a ['Oklcha'](palette::Oklcha) object from plain values.
pub fn oklcha<T>(lightness: T, chroma: T, hue: impl Into<OklabHue<T>>, alpha: T) -> Oklcha<T> {
    Oklcha::new(lightness, chroma, hue, alpha)
}

/// Pure black in [`Hsla`]
pub const fn black() -> Hsla {
    Hsla::new_const(RgbHue::new(0.), 0., 0., 1.)
}

/// Transparent black in [`Hsla`]
pub const fn transparent_black() -> Hsla {
    Hsla::new_const(RgbHue::new(0.), 0., 0., 0.)
}

/// Transparent white in [`Hsla`]
pub const fn transparent_white() -> Hsla {
    Hsla::new_const(RgbHue::new(0.), 0., 1., 0.)
}

/// Opaque grey in [`Hsla`], values must be provided in the range [0, 1]
pub const fn opaque_grey(lightness: f32, opacity: f32) -> Hsla {
    Hsla::new_const(RgbHue::new(0.), 0., lightness, opacity)
}

/// Pure white in [`Hsla`]
pub const fn white() -> Hsla {
    Hsla::new_const(RgbHue::new(0.), 0., 1., 1.)
}

/// The color red in [`Hsla`]
pub const fn red() -> Hsla {
    Hsla::new_const(RgbHue::new(0.), 1., 0.5, 1.)
}

/// The color blue in [`Hsla`]
pub const fn blue() -> Hsla {
    Hsla::new_const(RgbHue::new(240.), 1., 0.5, 1.)
}

/// The color green in [`Hsla`]
pub const fn green() -> Hsla {
    Hsla::new_const(RgbHue::new(120.), 1., 0.25, 1.)
}

/// The color yellow in [`Hsla`]
pub const fn yellow() -> Hsla {
    Hsla::new_const(RgbHue::new(60.), 1., 0.5, 1.)
}

/// Generates the JsonSchema for palette::Hsla
pub fn hsla_schemar(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    // https://github.com/Ogeon/palette/blob/9aa1ac21a7da60db398e8c044a43dbf3fdaf4855/palette/src/hsl.rs#L633-L636
    // https://github.com/Ogeon/palette/blob/9aa1ac21a7da60db398e8c044a43dbf3fdaf4855/palette/src/alpha/alpha.rs#L1197
    json_schema!({
        "type": "object",
        "properties": {
          "hue": {
              "type": "number",
              "format": "float"
          },
          "saturation": {
              "type": "number",
              "format": "float"
          },
          "lightness": {
              "type": "number",
              "format": "float"
          },
          "alpha": {
              "type": "number",
              "format": "float"
          },
        }
    })
}

/// Generates the JsonSchema for palette::Rgba
pub fn rgba_schemar(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    // https://github.com/Ogeon/palette/blob/9aa1ac21a7da60db398e8c044a43dbf3fdaf4855/palette/src/rgb/rgb.rs#L1690
    // https://github.com/Ogeon/palette/blob/9aa1ac21a7da60db398e8c044a43dbf3fdaf4855/palette/src/alpha/alpha.rs#L1197
    json_schema!({
        "type": "object",
        "properties": {
          "red": {
              "type": "number",
              "format": "float"
          },
          "green": {
              "type": "number",
              "format": "float"
          },
          "blue": {
              "type": "number",
              "format": "float"
          },
          "alpha": {
              "type": "number",
              "format": "float"
          },
        }
    })
}

/// Convenience methods shared by [`Rgba`] and [`Hsla`].
///
/// `Rgba` and `Hsla` come from the `palette` crate; this trait is where hgpui
/// keeps its ergonomics for them. Import it (or `hgpui::prelude::*`) to get
/// chainable color manipulation.
///
/// # Units
///
/// - `with_alpha` and `alpha_u8` use the 8-bit range `0..=255`, where `255` is
///   fully opaque.
/// - `with_alphaf` and the color types' `alpha` field use `0.0..=1.0`.
/// - `darken` / `lighten` take a lightness delta in `0.0..=1.0`.
pub trait ColorExt {
    /// Performs a SrcAlpha x (1 - SrcAlpha) blend
    fn blend(&self, other: &Self) -> Self
    where
        Self: Sized;

    /// Fade out the color by a given factor. This factor should be between 0.0 and 1.0.
    /// Where 0.0 will leave the color unchanged, and 1.0 will completely fade out the color.
    fn fade_out(&mut self, factor: f32);

    /// Multiplies the alpha value of the color by a given factor and returns a new color.
    /// If the color was previously opaque, then this is equivalent to
    /// [`with_alphaf(1.0 - factor)`](ColorExt::with_alphaf).
    ///
    /// Useful for transforming colors with dynamic opacity,
    /// like a color from an external source.
    ///
    /// Example:
    /// ```
    /// use hgpui::ColorExt;
    /// let color = hgpui::red();
    /// let faded_color = color.opacity(0.5);
    /// assert_eq!(faded_color.alpha, 0.5);
    /// ```
    ///
    /// This will return a red color with half the opacity.
    ///
    /// Example:
    /// ```
    /// use hgpui::{hsla, ColorExt};
    /// let color = hsla(0.7, 1.0, 0.5, 0.7); // A saturated blue
    /// let faded_color = color.opacity(0.16);
    /// assert!((faded_color.alpha - 0.112).abs() < 1e-6);
    /// ```
    ///
    /// This will return a blue color with around ~10% opacity,
    /// suitable for an element's hover or selected state.
    ///
    fn opacity(&self, factor: f32) -> Self
    where
        Self: Sized;

    /// Sets the alpha channel from an 8-bit value (`0` = fully transparent,
    /// `255` = fully opaque) and returns the color.
    ///
    /// This is the `u8` flavor; see [`with_alphaf`](ColorExt::with_alphaf) for
    /// the `0.0..=1.0` float flavor.
    ///
    /// ```
    /// use hgpui::ColorExt;
    /// let color = hgpui::red().with_alpha(128); // ~50% opacity
    /// assert_eq!(color.alpha_u8(), 128);
    /// ```
    fn with_alpha(self, alpha: u8) -> Self
    where
        Self: Sized;

    /// Sets the alpha channel from a `0.0..=1.0` float and returns the color.
    ///
    /// Out-of-range values are clamped (unlike `palette`'s unclamped
    /// `WithAlpha::with_alpha`):
    ///
    /// ```
    /// use hgpui::ColorExt;
    /// assert_eq!(hgpui::blue().with_alphaf(2.0).alpha_u8(), 255);
    /// assert_eq!(hgpui::blue().with_alphaf(-1.0).alpha_u8(), 0);
    /// ```
    fn with_alphaf(self, alpha: f32) -> Self
    where
        Self: Sized;

    /// Returns the alpha channel as an 8-bit value (`0..=255`).
    fn alpha_u8(self) -> u8
    where
        Self: Sized;

    /// Converts the color to sRGB ([`Rgba`]). Identity for `Rgba`.
    fn to_rgba(self) -> Rgba
    where
        Self: Sized;

    /// Converts the color to HSL-with-alpha ([`Hsla`]). Identity for `Hsla`.
    fn to_hsla(self) -> Hsla
    where
        Self: Sized;

    /// Returns a copy with its HSL lightness decreased by `amount`, a
    /// `0.0..=1.0` fraction clamped at black.
    fn darken(self, amount: f32) -> Self
    where
        Self: Sized;

    /// Returns a copy with its HSL lightness increased by `amount`, a
    /// `0.0..=1.0` fraction clamped at white.
    fn lighten(self, amount: f32) -> Self
    where
        Self: Sized;

    /// The YIQ perceived brightness of the color, `0.0` (black) to `1.0`
    /// (white). The alpha channel is ignored.
    fn perceived_brightness(self) -> f32
    where
        Self: Sized;

    /// Whether the color reads as dark (perceived brightness below `0.5`).
    ///
    /// Handy for picking a contrasting foreground, `tinycolor`-style:
    ///
    /// ```
    /// use hgpui::ColorExt;
    /// assert!(hgpui::black().is_dark());
    /// assert!(hgpui::white().is_light());
    /// ```
    fn is_dark(self) -> bool
    where
        Self: Sized;

    /// Whether the color reads as light. The inverse of [`is_dark`](ColorExt::is_dark).
    fn is_light(self) -> bool
    where
        Self: Sized,
    {
        !self.is_dark()
    }
}

/// YIQ perceived brightness (the formula behind `tinycolor`'s `isDark()`).
fn rgba_brightness(color: Rgba) -> f32 {
    0.299 * color.color.red + 0.587 * color.color.green + 0.114 * color.color.blue
}

impl ColorExt for Rgba {
    fn blend(&self, other: &Self) -> Self {
        use palette::blend::{BlendWith, Equations, Parameter};
        let blend_mode =
            Equations::from_parameters(Parameter::OneMinusSourceAlpha, Parameter::SourceAlpha);
        self.blend_with(*other, blend_mode)
    }

    fn fade_out(&mut self, factor: f32) {
        self.alpha *= 1.0 - factor.clamp(0., 1.);
    }

    fn opacity(&self, factor: f32) -> Self {
        let mut color = *self;
        color.alpha *= factor.clamp(0., 1.);
        color
    }

    fn with_alpha(mut self, alpha: u8) -> Self {
        self.alpha = alpha as f32 / 255.0;
        self
    }

    fn with_alphaf(mut self, alpha: f32) -> Self {
        self.alpha = alpha.clamp(0.0, 1.0);
        self
    }

    fn alpha_u8(self) -> u8 {
        (self.alpha * 255.0).round().clamp(0.0, 255.0) as u8
    }

    fn to_rgba(self) -> Rgba {
        self
    }

    fn to_hsla(self) -> Hsla {
        rgb_to_hsla(self)
    }

    fn darken(self, amount: f32) -> Self {
        let mut hsla = rgb_to_hsla(self);
        hsla.color.lightness = (hsla.color.lightness - amount).clamp(0.0, 1.0);
        hsla_to_rgba(hsla)
    }

    fn lighten(self, amount: f32) -> Self {
        let mut hsla = rgb_to_hsla(self);
        hsla.color.lightness = (hsla.color.lightness + amount).clamp(0.0, 1.0);
        hsla_to_rgba(hsla)
    }

    fn perceived_brightness(self) -> f32 {
        rgba_brightness(self)
    }

    fn is_dark(self) -> bool {
        self.perceived_brightness() < 0.5
    }
}
impl ColorExt for Hsla {
    fn blend(&self, other: &Self) -> Self {
        let this: Rgba = (*self).into_color();
        let other: Rgba = (*other).into_color();
        this.blend(&other).into_color()
    }

    fn fade_out(&mut self, factor: f32) {
        self.alpha *= 1.0 - factor.clamp(0., 1.);
    }

    fn opacity(&self, factor: f32) -> Self {
        let mut color = *self;
        color.alpha *= factor.clamp(0., 1.);
        color
    }

    fn with_alpha(mut self, alpha: u8) -> Self {
        self.alpha = alpha as f32 / 255.0;
        self
    }

    fn with_alphaf(mut self, alpha: f32) -> Self {
        self.alpha = alpha.clamp(0.0, 1.0);
        self
    }

    fn alpha_u8(self) -> u8 {
        (self.alpha * 255.0).round().clamp(0.0, 255.0) as u8
    }

    fn to_rgba(self) -> Rgba {
        hsla_to_rgba(self)
    }

    fn to_hsla(self) -> Hsla {
        self
    }

    fn darken(mut self, amount: f32) -> Self {
        self.color.lightness = (self.color.lightness - amount).clamp(0.0, 1.0);
        self
    }

    fn lighten(mut self, amount: f32) -> Self {
        self.color.lightness = (self.color.lightness + amount).clamp(0.0, 1.0);
        self
    }

    fn perceived_brightness(self) -> f32 {
        rgba_brightness(hsla_to_rgba(self))
    }

    fn is_dark(self) -> bool {
        self.perceived_brightness() < 0.5
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub(crate) enum BackgroundTag {
    Solid = 0,
    LinearGradient = 1,
    PatternSlash = 2,
    Checkerboard = 3,
    /// A radial gradient with an elliptical ending shape (the CSS default).
    RadialGradient = 4,
    /// A radial gradient with a circular ending shape.
    RadialGradientCircle = 5,
}

/// A color space for color interpolation.
///
/// References:
/// - <https://developer.mozilla.org/en-US/docs/Web/CSS/color-interpolation-method>
/// - <https://www.w3.org/TR/css-color-4/#typedef-color-space>
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub enum ColorSpace {
    #[default]
    /// The sRGB color space.
    Srgb = 0,
    /// The Oklab color space.
    Oklab = 1,
}

impl Display for ColorSpace {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ColorSpace::Srgb => write!(f, "sRGB"),
            ColorSpace::Oklab => write!(f, "Oklab"),
        }
    }
}

/// The ending shape of a [`radial_gradient`], set by [`Background::radial_shape`].
///
/// <https://developer.mozilla.org/en-US/docs/Web/CSS/gradient/radial-gradient#ending-shape>
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub enum RadialShape {
    #[default]
    /// An ellipse reaching the element's farthest corner along each axis.
    Ellipse,
    /// A circle reaching the element's farthest corner.
    Circle,
}

/// The ending-shape size of a [`radial_gradient`], set by [`Background::radial_size`].
///
/// The four keywords of CSS's `<ending-shape-size>`; explicit lengths are expressed by ending the
/// gradient's color stops closer to the center instead.
///
/// <https://developer.mozilla.org/en-US/docs/Web/CSS/gradient/radial-gradient#ending-shape-size>
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub enum RadialSize {
    #[default]
    /// Sized to meet the element's farthest corner (the CSS default, the largest of the four).
    FarthestCorner = 0,
    /// Sized to meet the element's nearest side.
    ClosestSide = 1,
    /// Sized to meet the element's farthest side.
    FarthestSide = 2,
    /// Sized to meet the element's nearest corner.
    ClosestCorner = 3,
}

/// A background color, which can be either a solid color or a linear gradient.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub struct Background {
    pub(crate) tag: BackgroundTag,
    pub(crate) color_space: ColorSpace,
    pub(crate) solid: crate::SceneHsla,
    /// The first tag-specific scalar: the angle in degrees for a linear gradient, the packed
    /// stripe width and interval for a slash pattern, the square size for a checkerboard, and the
    /// horizontal fraction of the element's size for a radial gradient's center.
    pub(crate) gradient_angle_or_pattern_height: f32,
    pub(crate) colors: [LinearColorStop; 2],
    /// The second tag-specific scalar: the vertical fraction of the element's size for a radial
    /// gradient's center, padding for `repr(C)` alignment otherwise. Serialized under its old
    /// `pad` name so previously written backgrounds still deserialize.
    #[serde(default, alias = "pad")]
    pub(crate) radial_center_y: f32,
    /// The ending-shape size keyword of a radial gradient, unused by other backgrounds.
    #[serde(default)]
    pub(crate) radial_size: RadialSize,
    /// Padding that keeps `Background` a multiple of 8 bytes. WGSL aligns the `mat2x2` in the
    /// `Quad` struct's `transformation` to 8 while Rust's `repr(C)` layout (and FXC's structured
    /// buffer packing) align it to 4, so the matrix must not land on a 4-byte-aligned offset.
    ///
    /// Skipped by serde: it is never anything but zero, and the name would otherwise collide with
    /// `radial_center_y`'s `pad` alias.
    #[serde(skip)]
    pad: u32,
}

impl std::fmt::Debug for Background {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.tag {
            BackgroundTag::Solid => write!(f, "Solid({:?})", self.solid),
            BackgroundTag::LinearGradient => write!(
                f,
                "LinearGradient({}, {:?}, {:?})",
                self.gradient_angle_or_pattern_height, self.colors[0], self.colors[1]
            ),
            BackgroundTag::PatternSlash => write!(
                f,
                "PatternSlash({:?}, {})",
                self.solid, self.gradient_angle_or_pattern_height
            ),
            BackgroundTag::Checkerboard => write!(
                f,
                "Checkerboard({:?}, {})",
                self.solid, self.gradient_angle_or_pattern_height
            ),
            BackgroundTag::RadialGradient | BackgroundTag::RadialGradientCircle => write!(
                f,
                "RadialGradient({:?}, {:?}, {:?}, {:?}, {:?})",
                self.radial_shape_from_tag(),
                self.radial_size,
                self.radial_center(),
                self.colors[0],
                self.colors[1]
            ),
        }
    }
}

impl Eq for Background {}
impl Default for Background {
    fn default() -> Self {
        Self {
            tag: BackgroundTag::Solid,
            solid: Hsla::default().into(),
            color_space: ColorSpace::default(),
            gradient_angle_or_pattern_height: 0.0,
            colors: [LinearColorStop::default(), LinearColorStop::default()],
            radial_center_y: 0.0,
            radial_size: RadialSize::default(),
            pad: 0,
        }
    }
}

/// Creates a hash pattern background
pub fn pattern_slash(color: impl IntoColor<Hsla>, width: f32, interval: f32) -> Background {
    let width_scaled = (width * 255.0) as u32;
    let interval_scaled = (interval * 255.0) as u32;
    let height = ((width_scaled * 0xFFFF) + interval_scaled) as f32;

    Background {
        tag: BackgroundTag::PatternSlash,
        solid: color.into_color().into(),
        gradient_angle_or_pattern_height: height,
        ..Default::default()
    }
}

/// Creates a checkerboard pattern background
pub fn checkerboard(color: impl IntoColor<Hsla>, size: f32) -> Background {
    Background {
        tag: BackgroundTag::Checkerboard,
        solid: color.into_color().into(),
        gradient_angle_or_pattern_height: size,
        ..Default::default()
    }
}

/// Creates a solid background color.
pub fn solid_background(color: impl IntoColor<Hsla>) -> Background {
    Background {
        solid: color.into_color().into(),
        ..Default::default()
    }
}

/// Creates a LinearGradient background color.
///
/// The gradient line's angle of direction. A value of `0.` is equivalent to top; increasing values rotate clockwise from there.
///
/// The `angle` is in degrees value in the range 0.0 to 360.0.
///
/// <https://developer.mozilla.org/en-US/docs/Web/CSS/gradient/linear-gradient>
pub fn linear_gradient(
    angle: f32,
    from: impl Into<LinearColorStop>,
    to: impl Into<LinearColorStop>,
) -> Background {
    Background {
        tag: BackgroundTag::LinearGradient,
        gradient_angle_or_pattern_height: angle,
        colors: [from.into(), to.into()],
        color_space: ColorSpace::Oklab,
        ..Default::default()
    }
}

/// Creates a RadialGradient background color, like CSS
/// `radial-gradient(<shape> farthest-corner at <center>, from, to)`.
///
/// `center` is a fraction of the element's size: `point(0.5, 0.5)` is the center, `point(0.0,
/// 0.0)` the top-left corner; values outside `0..=1` place the center outside the element. The
/// gradient reaches the element's farthest corner at the last color stop, so the stops'
/// percentages control how far it spreads — a glow that fades out halfway is
/// `radial_gradient(point(0.5, 0.5), linear_color_stop(color, 0.0),
/// linear_color_stop(transparent_black(), 0.5))`.
///
/// The ending shape is an ellipse, as in CSS; [`Background::radial_shape`] makes it a circle.
///
/// <https://developer.mozilla.org/en-US/docs/Web/CSS/gradient/radial-gradient>
pub fn radial_gradient(
    center: crate::Point<f32>,
    from: impl Into<LinearColorStop>,
    to: impl Into<LinearColorStop>,
) -> Background {
    Background {
        tag: BackgroundTag::RadialGradient,
        gradient_angle_or_pattern_height: center.x,
        radial_center_y: center.y,
        colors: [from.into(), to.into()],
        color_space: ColorSpace::Oklab,
        ..Default::default()
    }
}

/// A color stop in a linear gradient.
///
/// <https://developer.mozilla.org/en-US/docs/Web/CSS/gradient/linear-gradient#linear-color-stop>
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub struct LinearColorStop {
    /// The color of the color stop.
    pub color: crate::SceneHsla,
    /// The percentage of the gradient, in the range 0.0 to 1.0.
    pub percentage: f32,
}

/// Creates a new linear color stop.
///
/// The percentage of the gradient, in the range 0.0 to 1.0.
pub fn linear_color_stop(color: impl IntoColor<Hsla>, percentage: f32) -> LinearColorStop {
    LinearColorStop {
        color: color.into_color().into(),
        percentage,
    }
}

impl LinearColorStop {
    /// Returns a new color stop with the same color, but with a modified alpha value.
    pub fn opacity(&self, factor: f32) -> Self {
        let color: Hsla = self.color.into();
        Self {
            percentage: self.percentage,
            color: color.opacity(factor).into(),
        }
    }
}

/// What a [`Background`] paints, decoded from its packed representation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BackgroundKind {
    /// A flat color.
    Solid(Hsla),
    /// A linear gradient between two color stops.
    LinearGradient {
        /// The gradient line's angle in degrees, `0.0` pointing up, increasing clockwise.
        angle: f32,
        /// The two ends of the gradient.
        stops: [LinearColorStop; 2],
    },
    /// A radial gradient spreading from a point out to its ending shape.
    RadialGradient {
        /// The gradient's center, as fractions of the element's size.
        center: crate::Point<f32>,
        /// The gradient's ending shape.
        shape: RadialShape,
        /// The gradient's ending-shape size.
        size: RadialSize,
        /// The two ends of the gradient.
        stops: [LinearColorStop; 2],
    },
    /// A diagonal stripe pattern.
    PatternSlash {
        /// The stripe color.
        color: Hsla,
        /// The stripe width, in logical pixels.
        width: f32,
        /// The gap between stripes, in logical pixels.
        interval: f32,
    },
    /// Alternating squares of one color and full transparency.
    Checkerboard {
        /// The color of one set of squares. The other set is fully transparent.
        color: Hsla,
        /// The width and height of each square, in logical pixels.
        size: f32,
    },
}

impl Background {
    /// Returns the solid color if this is a solid background, None otherwise.
    pub fn as_solid(&self) -> Option<Hsla> {
        if self.tag == BackgroundTag::Solid {
            Some(self.solid.into())
        } else {
            None
        }
    }

    /// Returns the decoded form of this background.
    pub fn kind(&self) -> BackgroundKind {
        match self.tag {
            BackgroundTag::Solid => BackgroundKind::Solid(self.solid.into()),
            BackgroundTag::LinearGradient => BackgroundKind::LinearGradient {
                angle: self.gradient_angle_or_pattern_height,
                stops: self.colors,
            },
            BackgroundTag::RadialGradient | BackgroundTag::RadialGradientCircle => {
                BackgroundKind::RadialGradient {
                    center: self.radial_center(),
                    shape: self.radial_shape_from_tag(),
                    size: self.radial_size,
                    stops: self.colors,
                }
            }
            BackgroundTag::PatternSlash => {
                // `pattern_slash` packs both values into one f32 as `(width * 255) * 0xFFFF + (interval * 255)`.
                // floor + rem_euclid to invert it since that's the pairing that stays correct for negative inputs.
                // truncation and `%` give the wrong entry.
                let packed = self.gradient_angle_or_pattern_height;
                BackgroundKind::PatternSlash {
                    color: self.solid.into(),
                    width: (packed / 0xFFFF as f32).floor() / 255.0,
                    interval: (packed.rem_euclid(0xFFFF as f32)) / 255.0,
                }
            }
            BackgroundTag::Checkerboard => BackgroundKind::Checkerboard {
                color: self.solid.into(),
                size: self.gradient_angle_or_pattern_height,
            },
        }
    }

    /// Use specified color space for color interpolation.
    ///
    /// <https://developer.mozilla.org/en-US/docs/Web/CSS/color-interpolation-method>
    pub fn color_space(mut self, color_space: ColorSpace) -> Self {
        self.color_space = color_space;
        self
    }

    /// Sets the ending shape of a radial gradient, set by [`radial_gradient`].
    ///
    /// Has no effect on other backgrounds. Defaults to [`RadialShape::Ellipse`] (the CSS default).
    pub fn radial_shape(mut self, shape: RadialShape) -> Self {
        if matches!(
            self.tag,
            BackgroundTag::RadialGradient | BackgroundTag::RadialGradientCircle
        ) {
            self.tag = match shape {
                RadialShape::Ellipse => BackgroundTag::RadialGradient,
                RadialShape::Circle => BackgroundTag::RadialGradientCircle,
            };
        }
        self
    }

    /// Sets the ending-shape size of a radial gradient, set by [`radial_gradient`].
    ///
    /// Has no effect on other backgrounds. Defaults to [`RadialSize::FarthestCorner`], the CSS
    /// default; [`Background::radial_shape`] chooses between the elliptical and circular sizes.
    pub fn radial_size(mut self, size: RadialSize) -> Self {
        self.radial_size = size;
        self
    }

    /// The center of a radial gradient, as fractions of the element's size.
    fn radial_center(&self) -> crate::Point<f32> {
        crate::Point {
            x: self.gradient_angle_or_pattern_height,
            y: self.radial_center_y,
        }
    }

    /// The ending shape encoded in the tag (only meaningful for radial gradients).
    fn radial_shape_from_tag(&self) -> RadialShape {
        if self.tag == BackgroundTag::RadialGradientCircle {
            RadialShape::Circle
        } else {
            RadialShape::Ellipse
        }
    }

    /// The color space used to interpolate this background, set by [`Background::color_space`].
    pub fn interpolation_space(&self) -> ColorSpace {
        self.color_space
    }

    /// Returns a new background color with the same hue, saturation, and lightness, but with a modified alpha value.
    pub fn opacity(&self, factor: f32) -> Self {
        let mut background = *self;
        let solid: Hsla = background.solid.into();
        background.solid = solid.opacity(factor).into();
        background.colors = [
            self.colors[0].opacity(factor),
            self.colors[1].opacity(factor),
        ];
        background
    }

    /// Returns whether the background color is transparent.
    pub fn is_transparent(&self) -> bool {
        match self.tag {
            BackgroundTag::Solid => self.solid.a == 0.,
            BackgroundTag::LinearGradient
            | BackgroundTag::RadialGradient
            | BackgroundTag::RadialGradientCircle => self.colors.iter().all(|c| c.color.a == 0.),
            BackgroundTag::PatternSlash => self.solid.a == 0.,
            BackgroundTag::Checkerboard => self.solid.a == 0.,
        }
    }
}

impl<T: IntoColor<Hsla>> From<T> for Background {
    fn from(value: T) -> Self {
        Self {
            tag: BackgroundTag::Solid,
            solid: value.into_color().into(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_background_solid() {
        let color: Hsla = rgba(0xff0099ff).into_color();
        let mut background = Background::from(color);
        assert_eq!(background.tag, BackgroundTag::Solid);
        assert_eq!(background.solid, color.into());

        assert_eq!(background.opacity(0.5).solid, color.opacity(0.5).into());
        assert!(!background.is_transparent());
        background.solid = hsla(0.0, 0.0, 0.0, 0.0).into();
        assert!(background.is_transparent());
    }

    #[test]
    fn test_background_linear_gradient() {
        let from = linear_color_stop(rgba(0xff0099ff), 0.0);
        let to = linear_color_stop(rgba(0x00ff99ff), 1.0);
        let background = linear_gradient(90.0, from, to);
        assert_eq!(background.tag, BackgroundTag::LinearGradient);
        assert_eq!(background.colors[0], from);
        assert_eq!(background.colors[1], to);

        assert_eq!(background.opacity(0.5).colors[0], from.opacity(0.5));
        assert_eq!(background.opacity(0.5).colors[1], to.opacity(0.5));
        assert!(!background.is_transparent());
        assert!(background.opacity(0.0).is_transparent());
    }

    #[test]
    fn test_background_radial_gradient() {
        let from = linear_color_stop(rgba(0xff0099ff), 0.0);
        let to = linear_color_stop(rgba(0x00ff99ff), 1.0);
        let background = radial_gradient(crate::point(0.25, 0.75), from, to);
        assert_eq!(background.tag, BackgroundTag::RadialGradient);
        assert_eq!(background.colors[0], from);
        assert_eq!(background.colors[1], to);
        assert_eq!(
            background.kind(),
            BackgroundKind::RadialGradient {
                center: crate::point(0.25, 0.75),
                shape: RadialShape::Ellipse,
                size: RadialSize::FarthestCorner,
                stops: [from, to],
            }
        );

        // The size keyword defaults to CSS's farthest-corner and is switched independently of the
        // ending shape.
        assert_eq!(
            background
                .radial_size(RadialSize::ClosestSide)
                .radial_shape(RadialShape::Circle)
                .kind(),
            BackgroundKind::RadialGradient {
                center: crate::point(0.25, 0.75),
                shape: RadialShape::Circle,
                size: RadialSize::ClosestSide,
                stops: [from, to],
            }
        );

        // The shape builder switches the tag, and leaves other backgrounds alone.
        let circle = background.radial_shape(RadialShape::Circle);
        assert_eq!(circle.tag, BackgroundTag::RadialGradientCircle);
        assert_eq!(circle.radial_shape_from_tag(), RadialShape::Circle);
        assert_eq!(circle.radial_center(), crate::point(0.25, 0.75));
        let solid = solid_background(rgba(0xff0099ff)).radial_shape(RadialShape::Circle);
        assert_eq!(solid.tag, BackgroundTag::Solid);

        // Opacity and transparency follow the color stops, as for the linear gradient.
        assert_eq!(background.opacity(0.5).colors[0], from.opacity(0.5));
        assert_eq!(background.opacity(0.5).colors[1], to.opacity(0.5));
        assert!(!background.is_transparent());
        assert!(background.opacity(0.0).is_transparent());
    }

    #[test]
    fn test_background_kind() {
        let color: Hsla = rgba(0xff0099ff).into_color();
        assert_eq!(Background::from(color).kind(), BackgroundKind::Solid(color));

        let from = linear_color_stop(rgba(0xff0099ff), 0.0);
        let to = linear_color_stop(rgba(0x00ff99ff), 1.0);
        assert_eq!(
            linear_gradient(90.0, from, to).kind(),
            BackgroundKind::LinearGradient {
                angle: 90.0,
                stops: [from, to],
            }
        );

        assert_eq!(
            checkerboard(color, 12.0).kind(),
            BackgroundKind::Checkerboard { color, size: 12.0 }
        );
    }

    #[test]
    fn test_background_kind_unpacks_pattern_slash() {
        let color: Hsla = rgba(0xff0099ff).into_color();
        // Both values survive to the 1/255 the constructor quantizes them to.
        for (width, interval) in [(1.0, 3.0), (0.5, 0.25), (2.0, 10.0)] {
            let BackgroundKind::PatternSlash {
                width: got_width,
                interval: got_interval,
                ..
            } = pattern_slash(color, width, interval).kind()
            else {
                panic!("pattern_slash did not produce a PatternSlash");
            };
            assert!((got_width - width).abs() <= 1.0 / 255.0);
            assert!((got_interval - interval).abs() <= 1.0 / 255.0);
        }
    }

    #[test]
    fn test_u8_constructors() {
        let from_hex = rgba(0xff0000ff);
        let from_u8 = rgba8(255, 0, 0, 255);
        assert_eq!(from_u8.color.red, from_hex.color.red);
        assert_eq!(from_u8.color.green, from_hex.color.green);
        assert_eq!(from_u8.color.blue, from_hex.color.blue);
        assert_eq!(from_u8.alpha, from_hex.alpha);

        let opaque = rgb8(0, 128, 255);
        assert_eq!(opaque.color.red, 0.0);
        assert!((opaque.color.green - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(opaque.color.blue, 1.0);
        assert_eq!(opaque.alpha, 1.0);
    }

    #[test]
    fn test_with_alpha_flavors() {
        let color = rgba8(255, 0, 0, 128);
        assert_eq!(color.with_alpha(64).alpha_u8(), 64);
        assert_eq!(color.with_alphaf(0.25).alpha, 0.25);
        // Unlike palette's `WithAlpha`, out-of-range floats are clamped.
        assert_eq!(color.with_alphaf(1.5).alpha, 1.0);
        assert_eq!(color.with_alphaf(-0.5).alpha, 0.0);
        // The same API is available on `Hsla`.
        assert_eq!(red().with_alpha(128).alpha_u8(), 128);
        assert_eq!(red().with_alphaf(0.25).alpha, 0.25);
    }

    #[test]
    fn test_alpha_u8_roundtrip() {
        for alpha in [0u8, 1, 64, 128, 254, 255] {
            assert_eq!(red().with_alpha(alpha).alpha_u8(), alpha);
        }
    }

    #[test]
    fn test_darken_lighten() {
        assert_eq!(black().lighten(0.5).color.lightness, 0.5);
        assert_eq!(white().darken(0.25).color.lightness, 0.75);
        // Clamped at the extremes.
        assert_eq!(black().darken(0.5).color.lightness, 0.0);
        assert_eq!(white().lighten(0.5).color.lightness, 1.0);

        // `Rgba` goes through HSL internally.
        let lightened = rgb8(128, 128, 128).lighten(0.2).to_hsla();
        assert!((lightened.color.lightness - (128.0 / 255.0 + 0.2)).abs() < 0.01);
        assert!(
            rgb8(128, 128, 128).darken(0.2).perceived_brightness()
                < rgb8(128, 128, 128).perceived_brightness()
        );
    }

    #[test]
    fn test_is_dark() {
        assert!(black().is_dark());
        assert!(!black().is_light());
        assert!(white().is_light());
        assert!(!white().is_dark());
        assert!(rgb8(120, 120, 120).is_dark());
        assert!(rgb8(140, 140, 140).is_light());
        assert!(rgb8(255, 255, 0).is_light()); // yellow reads bright
        assert!(rgb8(0, 0, 255).is_dark()); // blue reads dark
    }

    #[test]
    fn test_color_space_conversions_roundtrip() {
        let rgba = rgba8(10, 200, 30, 255);
        let roundtrip = rgba.to_hsla().to_rgba();
        assert!((roundtrip.color.red - rgba.color.red).abs() < 1e-4);
        assert!((roundtrip.color.green - rgba.color.green).abs() < 1e-4);
        assert!((roundtrip.color.blue - rgba.color.blue).abs() < 1e-4);
        assert_eq!(roundtrip.alpha, rgba.alpha);
    }

    #[test]
    fn test_opacity_vs_with_alphaf() {
        let color = rgba8(255, 0, 0, 128);
        // `opacity` multiplies the existing alpha...
        assert_eq!(color.opacity(0.5).alpha_u8(), 64);
        // ...while `with_alphaf` sets it outright.
        assert_eq!(color.with_alphaf(0.5).alpha_u8(), 128);
    }
}
