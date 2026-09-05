//! CSS-style transforms for the `Styled` API.
//!
//! [`CssTransform`] models the CSS `transform` property: an ordered list of transform
//! functions, applied left-to-right (the leftmost function is the outermost, exactly like
//! CSS). All 2D functions (`translate`, `scale`, `rotate`, `skew`, `matrix`) are fully
//! implemented and compose into a [`TransformationMatrix`] that the renderers apply at GPU
//! drawing time. The 3D functions exist for API parity with CSS but are not implemented yet:
//! they contribute the identity matrix and are marked as such.
//!
//! Like CSS, the transform only affects painting — layout and hit-testing keep using the
//! element's untransformed bounds.

use crate::{size, Point, Pixels, Radians, ScaledPixels, TransformationMatrix};
use serde::{Deserialize, Serialize};

/// A single CSS transform function.
///
/// The units follow CSS where a length is involved: pixels for translations and `matrix`'s
/// `e`/`f` entries. Percentage translations (e.g. `translate(50%, 50%)`) are not implemented
/// yet. Angles are radians (use [`crate::degrees`] to convert from degrees).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CssTransformFunction {
    /// `translate(tx, ty)`
    Translate {
        /// Horizontal translation in pixels.
        x: Pixels,
        /// Vertical translation in pixels.
        y: Pixels,
    },
    /// `translateX(tx)`
    TranslateX(Pixels),
    /// `translateY(ty)`
    TranslateY(Pixels),
    /// `scale(sx, sy)`
    Scale {
        /// Horizontal scale factor.
        x: f32,
        /// Vertical scale factor.
        y: f32,
    },
    /// `scaleX(sx)`
    ScaleX(f32),
    /// `scaleY(sy)`
    ScaleY(f32),
    /// `rotate(a)`, clockwise in radians.
    Rotate(Radians),
    /// `skewX(a)`, in radians.
    SkewX(Radians),
    /// `skewY(a)`, in radians.
    SkewY(Radians),
    /// `matrix(a, b, c, d, e, f)` — the CSS 2D affine matrix, where `e`/`f` are in pixels.
    Matrix {
        /// `a`: x contribution of the x coordinate.
        a: f32,
        /// `b`: y contribution of the x coordinate.
        b: f32,
        /// `c`: x contribution of the y coordinate.
        c: f32,
        /// `d`: y contribution of the y coordinate.
        d: f32,
        /// `e`: x translation in pixels.
        e: Pixels,
        /// `f`: y translation in pixels.
        f: Pixels,
    },

    // --- 3D transforms (declared for CSS parity; NOT implemented yet) ---
    /// `translate3d(x, y, z)` — unimplemented (identity).
    Translate3d {
        /// x translation in pixels.
        x: Pixels,
        /// y translation in pixels.
        y: Pixels,
        /// z translation in pixels.
        z: Pixels,
    },
    /// `translateZ(z)` — unimplemented (identity).
    TranslateZ(Pixels),
    /// `scale3d(x, y, z)` — unimplemented (identity).
    Scale3d {
        /// x scale factor.
        x: f32,
        /// y scale factor.
        y: f32,
        /// z scale factor.
        z: f32,
    },
    /// `scaleZ(z)` — unimplemented (identity).
    ScaleZ(f32),
    /// `rotateX(a)` — unimplemented (identity).
    RotateX(Radians),
    /// `rotateY(a)` — unimplemented (identity).
    RotateY(Radians),
    /// `rotateZ(a)` — equivalent to `rotate`; implemented.
    RotateZ(Radians),
    /// `perspective(d)` — unimplemented (identity).
    Perspective(Pixels),
    /// `matrix3d(m00 … m33)` — unimplemented (identity).
    Matrix3d([f32; 16]),
}

impl CssTransformFunction {
    /// The 2D affine matrix for this function, in the element's local coordinate space.
    /// Translations (and `matrix`'s `e`/`f` entries) are multiplied by `scale_factor` to
    /// convert from logical pixels to device pixels. 3D functions return the identity
    /// matrix (not implemented yet).
    fn to_matrix(self, scale_factor: f32) -> TransformationMatrix {
        match self {
            CssTransformFunction::Translate { x, y } => TransformationMatrix::unit()
                .translate(Point::new(
                    ScaledPixels(x.0 * scale_factor),
                    ScaledPixels(y.0 * scale_factor),
                )),
            CssTransformFunction::TranslateX(x) => TransformationMatrix::unit()
                .translate(Point::new(ScaledPixels(x.0 * scale_factor), ScaledPixels(0.0))),
            CssTransformFunction::TranslateY(y) => TransformationMatrix::unit()
                .translate(Point::new(ScaledPixels(0.0), ScaledPixels(y.0 * scale_factor))),
            CssTransformFunction::Scale { x, y } => {
                TransformationMatrix::unit().scale(size(x, y))
            }
            CssTransformFunction::ScaleX(x) => {
                TransformationMatrix::unit().scale(size(x, 1.0))
            }
            CssTransformFunction::ScaleY(y) => {
                TransformationMatrix::unit().scale(size(1.0, y))
            }
            CssTransformFunction::Rotate(angle) | CssTransformFunction::RotateZ(angle) => {
                TransformationMatrix::unit().rotate(angle)
            }
            CssTransformFunction::SkewX(angle) => TransformationMatrix::unit().compose(
                TransformationMatrix {
                    rotation_scale: [[1.0, angle.0.tan()], [0.0, 1.0]],
                    translation: [0.0, 0.0],
                },
            ),
            CssTransformFunction::SkewY(angle) => TransformationMatrix::unit().compose(
                TransformationMatrix {
                    rotation_scale: [[1.0, 0.0], [angle.0.tan(), 1.0]],
                    translation: [0.0, 0.0],
                },
            ),
            CssTransformFunction::Matrix { a, b, c, d, e, f } => TransformationMatrix {
                rotation_scale: [[a, c], [b, d]],
                translation: [e.0 * scale_factor, f.0 * scale_factor],
            },
            // 3D transforms are declared for CSS parity but not implemented yet.
            CssTransformFunction::Translate3d { .. }
            | CssTransformFunction::TranslateZ(_)
            | CssTransformFunction::Scale3d { .. }
            | CssTransformFunction::ScaleZ(_)
            | CssTransformFunction::RotateX(_)
            | CssTransformFunction::RotateY(_)
            | CssTransformFunction::Perspective(_)
            | CssTransformFunction::Matrix3d(_) => TransformationMatrix::unit(),
        }
    }
}

/// A CSS `transform` property value: an ordered list of transform functions.
///
/// Functions are composed left-to-right, exactly like CSS: `translateX(10px) scale(2)`
/// scales the element around its transform origin first, then translates it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CssTransform {
    functions: Vec<CssTransformFunction>,
}

// The chained setters intentionally mirror the CSS function names (`translateX`, `scaleY`, ...).
#[allow(non_snake_case)]
impl CssTransform {
    /// The identity transform (no effect).
    pub fn identity() -> Self {
        Self::default()
    }

    /// `translate(tx, ty)`
    pub fn translate(mut self, x: impl Into<Pixels>, y: impl Into<Pixels>) -> Self {
        self.functions.push(CssTransformFunction::Translate {
            x: x.into(),
            y: y.into(),
        });
        self
    }

    /// `translateX(tx)`
    pub fn translate_x(mut self, x: impl Into<Pixels>) -> Self {
        self.functions
            .push(CssTransformFunction::TranslateX(x.into()));
        self
    }

    /// `translateY(ty)`
    pub fn translate_y(mut self, y: impl Into<Pixels>) -> Self {
        self.functions
            .push(CssTransformFunction::TranslateY(y.into()));
        self
    }

    /// `scale(sx, sy)`
    pub fn scale(mut self, x: f32, y: f32) -> Self {
        self.functions.push(CssTransformFunction::Scale { x, y });
        self
    }

    /// `scaleX(sx)`
    pub fn scale_x(mut self, x: f32) -> Self {
        self.functions.push(CssTransformFunction::ScaleX(x));
        self
    }

    /// `scaleY(sy)`
    pub fn scale_y(mut self, y: f32) -> Self {
        self.functions.push(CssTransformFunction::ScaleY(y));
        self
    }

    /// `rotate(a)`, clockwise in radians.
    pub fn rotate(mut self, angle: impl Into<Radians>) -> Self {
        self.functions
            .push(CssTransformFunction::Rotate(angle.into()));
        self
    }

    /// `skewX(a)`, in radians.
    pub fn skew_x(mut self, angle: impl Into<Radians>) -> Self {
        self.functions
            .push(CssTransformFunction::SkewX(angle.into()));
        self
    }

    /// `skewY(a)`, in radians.
    pub fn skew_y(mut self, angle: impl Into<Radians>) -> Self {
        self.functions
            .push(CssTransformFunction::SkewY(angle.into()));
        self
    }

    /// `matrix(a, b, c, d, e, f)` — the CSS 2D affine matrix (`e`/`f` in pixels).
    #[allow(clippy::too_many_arguments)]
    pub fn matrix(
        mut self,
        a: f32,
        b: f32,
        c: f32,
        d: f32,
        e: impl Into<Pixels>,
        f: impl Into<Pixels>,
    ) -> Self {
        self.functions.push(CssTransformFunction::Matrix {
            a,
            b,
            c,
            d,
            e: e.into(),
            f: f.into(),
        });
        self
    }

    /// `translate3d(x, y, z)` — declared for CSS parity; not implemented yet (identity).
    pub fn translate3d(
        mut self,
        x: impl Into<Pixels>,
        y: impl Into<Pixels>,
        z: impl Into<Pixels>,
    ) -> Self {
        self.functions.push(CssTransformFunction::Translate3d {
            x: x.into(),
            y: y.into(),
            z: z.into(),
        });
        self
    }

    /// `translateZ(z)` — declared for CSS parity; not implemented yet (identity).
    pub fn translate_z(mut self, z: impl Into<Pixels>) -> Self {
        self.functions
            .push(CssTransformFunction::TranslateZ(z.into()));
        self
    }

    /// `scale3d(x, y, z)` — declared for CSS parity; not implemented yet (identity).
    pub fn scale3d(mut self, x: f32, y: f32, z: f32) -> Self {
        self.functions.push(CssTransformFunction::Scale3d { x, y, z });
        self
    }

    /// `scaleZ(z)` — declared for CSS parity; not implemented yet (identity).
    pub fn scale_z(mut self, z: f32) -> Self {
        self.functions.push(CssTransformFunction::ScaleZ(z));
        self
    }

    /// `rotateX(a)` — declared for CSS parity; not implemented yet (identity).
    pub fn rotate_x(mut self, angle: impl Into<Radians>) -> Self {
        self.functions
            .push(CssTransformFunction::RotateX(angle.into()));
        self
    }

    /// `rotateY(a)` — declared for CSS parity; not implemented yet (identity).
    pub fn rotate_y(mut self, angle: impl Into<Radians>) -> Self {
        self.functions
            .push(CssTransformFunction::RotateY(angle.into()));
        self
    }

    /// `rotateZ(a)` — clockwise rotation in radians (same as `rotate`).
    pub fn rotate_z(mut self, angle: impl Into<Radians>) -> Self {
        self.functions
            .push(CssTransformFunction::RotateZ(angle.into()));
        self
    }

    /// `perspective(d)` — declared for CSS parity; not implemented yet (identity).
    pub fn perspective(mut self, d: impl Into<Pixels>) -> Self {
        self.functions
            .push(CssTransformFunction::Perspective(d.into()));
        self
    }

    /// `matrix3d(m00 … m33)` — declared for CSS parity; not implemented yet (identity).
    pub fn matrix3d(mut self, values: [f32; 16]) -> Self {
        self.functions.push(CssTransformFunction::Matrix3d(values));
        self
    }

    /// Whether this transform has any effect (i.e. it is not the identity).
    pub fn is_identity(&self) -> bool {
        self.functions.is_empty()
    }

    /// Compose this transform into a [`TransformationMatrix`] around the given center point
    /// (the CSS transform origin, defaulting to the element's center), in device pixels.
    ///
    /// The returned matrix maps element-local pixel coordinates to the parent coordinate
    /// system: `translate(center) · functions · translate(-center)`.
    pub fn to_matrix(&self, center: Point<Pixels>, scale_factor: f32) -> TransformationMatrix {
        let mut matrix = TransformationMatrix::unit();
        for function in &self.functions {
            matrix = matrix.compose(function.to_matrix(scale_factor));
        }
        let center = center.scale(scale_factor);
        TransformationMatrix::unit()
            .translate(center)
            .compose(matrix)
            .translate(center * -1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{point, px, radians};
    use std::f32::consts::FRAC_PI_2;
    use std::f32::consts::FRAC_PI_4;

    fn assert_point(m: &TransformationMatrix, input: (f32, f32), expected: (f32, f32)) {
        let out = m.apply(point(px(input.0), px(input.1)));
        assert!(
            (out.x.0 - expected.0).abs() < 1e-3 && (out.y.0 - expected.1).abs() < 1e-3,
            "expected {:?} got {:?}",
            expected,
            (out.x.0, out.y.0)
        );
    }

    #[test]
    fn identity_transform_leaves_points_unchanged() {
        let m = CssTransform::identity().to_matrix(point(px(10.0), px(20.0)), 1.0);
        assert_point(&m, (0.0, 0.0), (0.0, 0.0));
        assert_point(&m, (100.0, 50.0), (100.0, 50.0));
    }

    #[test]
    fn translate_shifts_by_pixels() {
        // translate(10, 20) moves every point by (10, 20), independent of the center.
        let m = CssTransform::identity()
            .translate(px(10.0), px(20.0))
            .to_matrix(point(px(50.0), px(50.0)), 1.0);
        assert_point(&m, (0.0, 0.0), (10.0, 20.0));
        assert_point(&m, (100.0, 50.0), (110.0, 70.0));
    }

    #[test]
    fn translate_x_and_y_are_axis_aligned() {
        let mx = CssTransform::identity()
            .translate_x(px(15.0))
            .to_matrix(point(px(0.0), px(0.0)), 1.0);
        assert_point(&mx, (5.0, 7.0), (20.0, 7.0));

        let my = CssTransform::identity()
            .translate_y(px(-6.0))
            .to_matrix(point(px(0.0), px(0.0)), 1.0);
        assert_point(&my, (5.0, 7.0), (5.0, 1.0));
    }

    #[test]
    fn scale_about_center_scales_from_the_center() {
        // scale(2) around center (50, 50): (50, 50) stays put, (0, 0) -> (-50, -50).
        let m = CssTransform::identity()
            .scale(2.0, 2.0)
            .to_matrix(point(px(50.0), px(50.0)), 1.0);
        assert_point(&m, (50.0, 50.0), (50.0, 50.0));
        assert_point(&m, (0.0, 0.0), (-50.0, -50.0));
        assert_point(&m, (100.0, 100.0), (150.0, 150.0));
    }

    #[test]
    fn rotate_about_center_rotates_90_degrees() {
        // rotate(90°) around (0, 0): (1, 0) -> (0, 1).
        let m = CssTransform::identity()
            .rotate(radians(FRAC_PI_2))
            .to_matrix(point(px(0.0), px(0.0)), 1.0);
        assert_point(&m, (10.0, 0.0), (0.0, 10.0));
        assert_point(&m, (0.0, 10.0), (-10.0, 0.0));
    }

    #[test]
    fn skew_x_shifts_x_by_tangent_times_y() {
        // skewX(45°): x' = x + y.
        let m = CssTransform::identity()
            .skew_x(radians(FRAC_PI_4))
            .to_matrix(point(px(0.0), px(0.0)), 1.0);
        assert_point(&m, (10.0, 5.0), (15.0, 5.0));
    }

    #[test]
    fn skew_y_shifts_y_by_tangent_times_x() {
        let m = CssTransform::identity()
            .skew_y(radians(FRAC_PI_4))
            .to_matrix(point(px(0.0), px(0.0)), 1.0);
        assert_point(&m, (10.0, 5.0), (10.0, 15.0));
    }

    #[test]
    fn matrix_uses_css_conventions() {
        // matrix(2, 0, 0, 1, 10, 0): x' = 2x + 10, y' = y.
        let m = CssTransform::identity()
            .matrix(2.0, 0.0, 0.0, 1.0, px(10.0), px(0.0))
            .to_matrix(point(px(0.0), px(0.0)), 1.0);
        assert_point(&m, (5.0, 5.0), (20.0, 5.0));
    }

    #[test]
    fn functions_compose_left_to_right_like_css() {
        // transform: translateX(10px) scale(2) — scale first (around the origin), then
        // translate: (0,0) -> scale -> (0,0) -> translate -> (10, 0).
        let m = CssTransform::identity()
            .translate_x(px(10.0))
            .scale(2.0, 2.0)
            .to_matrix(point(px(0.0), px(0.0)), 1.0);
        assert_point(&m, (0.0, 0.0), (10.0, 0.0));
        assert_point(&m, (5.0, 0.0), (20.0, 0.0));
    }

    #[test]
    fn scale_factor_scales_translations() {
        let m = CssTransform::identity()
            .translate_x(px(10.0))
            .to_matrix(point(px(0.0), px(0.0)), 2.0);
        assert_point(&m, (0.0, 0.0), (20.0, 0.0));
    }

    #[test]
    fn three_d_transforms_are_identity_for_now() {
        let m = CssTransform::identity()
            .translate3d(px(10.0), px(10.0), px(10.0))
            .rotate_x(radians(FRAC_PI_4))
            .perspective(px(100.0))
            .to_matrix(point(px(0.0), px(0.0)), 1.0);
        assert_point(&m, (5.0, 5.0), (5.0, 5.0));
    }

    #[test]
    fn translate_accepts_floats() {
        let m = CssTransform::identity()
            .translate_x(15.0)
            .translate_y(3.0)
            .to_matrix(point(px(0.0), px(0.0)), 1.0);
        assert_point(&m, (0.0, 0.0), (15.0, 3.0));
    }
}
