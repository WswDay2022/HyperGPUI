//! CPU-side folding of a scene filter chain into one color matrix.
//!
//! Every non-spatial CSS filter function (`grayscale`, `sepia`, `saturate`, `brightness`,
//! `contrast`, `invert`, `hue-rotate`, `opacity`) is an affine transform of straight-alpha
//! sRGB color. A chain of them therefore collapses into a single 4×4 matrix plus a 4-vector
//! offset, which the composite shader applies once — no matter how many color filters the
//! element lists.
//!
//! Blur is linear and commutes with these transforms (the gaussian weights are normalized and
//! the transform is affine), so the matrix is applied after the blur passes. The order of the
//! color filters relative to each other and to the blur is preserved.

use hgpui::ScaledFilter;

/// An affine color transform in straight-alpha space:
/// `out[i] = dot(rows[i], input) + offset[i]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorFilterMatrix {
    /// Four row vectors, row-major. The fourth row controls alpha.
    pub rows: [[f32; 4]; 4],
    /// Constant term added after the matrix multiply.
    pub offset: [f32; 4],
}

impl Default for ColorFilterMatrix {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl ColorFilterMatrix {
    /// The transform that leaves the color unchanged.
    pub const IDENTITY: Self = Self {
        rows: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        offset: [0.0; 4],
    };

    /// Compose `self` followed by `next` (`next(self(color))`).
    fn compose(self, next: Self) -> Self {
        let mut rows = [[0.0; 4]; 4];
        for (row_index, row) in rows.iter_mut().enumerate() {
            for (column_index, value) in row.iter_mut().enumerate() {
                *value = (0..4)
                    .map(|k| next.rows[row_index][k] * self.rows[k][column_index])
                    .sum();
            }
        }

        let mut offset = [0.0; 4];
        for (index, value) in offset.iter_mut().enumerate() {
            *value = next.offset[index]
                + (0..4)
                    .map(|k| next.rows[index][k] * self.offset[k])
                    .sum::<f32>();
        }

        Self { rows, offset }
    }

    /// Apply the transform to a straight-alpha RGBA color. Used by tests and host-side previews;
    /// the renderers apply it on the GPU.
    pub fn apply(&self, color: [f32; 4]) -> [f32; 4] {
        let mut output = [0.0; 4];
        for (index, value) in output.iter_mut().enumerate() {
            *value =
                self.offset[index] + (0..4).map(|k| self.rows[index][k] * color[k]).sum::<f32>();
        }
        output
    }
}

/// Fold every color filter in `filters` into one matrix, preserving their authored order.
///
/// Returns `None` when the chain has no color filters (blur-only or empty), letting the
/// renderers skip the color-matrix path and keep their existing blur output bit-for-bit.
pub fn color_filter_matrix(filters: &[ScaledFilter]) -> Option<ColorFilterMatrix> {
    let mut matrix = ColorFilterMatrix::IDENTITY;
    let mut has_color_filter = false;

    for filter in filters {
        let next = match filter {
            ScaledFilter::Blur(_) => continue,
            ScaledFilter::Grayscale(amount) => grayscale(*amount),
            ScaledFilter::Sepia(amount) => sepia(*amount),
            ScaledFilter::Saturate(amount) => saturate(*amount),
            ScaledFilter::Brightness(amount) => brightness(*amount),
            ScaledFilter::Contrast(amount) => contrast(*amount),
            ScaledFilter::Invert(amount) => invert(*amount),
            ScaledFilter::HueRotate(degrees) => hue_rotate(*degrees),
            ScaledFilter::Opacity(amount) => opacity(*amount),
        };
        matrix = matrix.compose(next);
        has_color_filter = true;
    }

    has_color_filter.then_some(matrix)
}

/// The effective blur radius of a filter chain, in scaled pixels.
///
/// Successive gaussian blurs compose by adding their variances, so two `blur(3)`/`blur(4)`
/// filters behave like one `blur(5)` — not `blur(4)` as a max would give. Non-positive and
/// non-finite radii contribute nothing.
pub fn combined_blur_radius(filters: &[ScaledFilter]) -> f32 {
    filters
        .iter()
        .fold(0.0, |sum_of_squares, filter| match filter {
            ScaledFilter::Blur(radius) => {
                if radius.0.is_finite() && radius.0 > 0.0 {
                    sum_of_squares + radius.0 * radius.0
                } else {
                    sum_of_squares
                }
            }
            ScaledFilter::Grayscale(_)
            | ScaledFilter::Sepia(_)
            | ScaledFilter::Saturate(_)
            | ScaledFilter::Brightness(_)
            | ScaledFilter::Contrast(_)
            | ScaledFilter::Invert(_)
            | ScaledFilter::HueRotate(_)
            | ScaledFilter::Opacity(_) => sum_of_squares,
        })
        .sqrt()
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn clamp01(value: f32) -> f32 {
    finite_or(value, 0.0).clamp(0.0, 1.0)
}

fn non_negative(value: f32) -> f32 {
    finite_or(value, 1.0).max(0.0)
}

fn linear(matrix: [[f32; 4]; 4]) -> ColorFilterMatrix {
    ColorFilterMatrix {
        rows: matrix,
        offset: [0.0; 4],
    }
}

fn grayscale(amount: f32) -> ColorFilterMatrix {
    let inv = 1.0 - clamp01(amount);
    linear([
        [
            0.2126 + 0.7874 * inv,
            0.7152 - 0.7152 * inv,
            0.0722 - 0.0722 * inv,
            0.0,
        ],
        [
            0.2126 - 0.2126 * inv,
            0.7152 + 0.2848 * inv,
            0.0722 - 0.0722 * inv,
            0.0,
        ],
        [
            0.2126 - 0.2126 * inv,
            0.7152 - 0.7152 * inv,
            0.0722 + 0.9278 * inv,
            0.0,
        ],
        [0.0, 0.0, 0.0, 1.0],
    ])
}

fn sepia(amount: f32) -> ColorFilterMatrix {
    let inv = 1.0 - clamp01(amount);
    linear([
        [
            0.393 + 0.607 * inv,
            0.769 - 0.769 * inv,
            0.189 - 0.189 * inv,
            0.0,
        ],
        [
            0.349 - 0.349 * inv,
            0.686 + 0.314 * inv,
            0.168 - 0.168 * inv,
            0.0,
        ],
        [
            0.272 - 0.272 * inv,
            0.534 - 0.534 * inv,
            0.131 + 0.869 * inv,
            0.0,
        ],
        [0.0, 0.0, 0.0, 1.0],
    ])
}

fn saturate(amount: f32) -> ColorFilterMatrix {
    let s = non_negative(amount);
    linear([
        [0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s, 0.0],
        [0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s, 0.0],
        [0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ])
}

fn brightness(amount: f32) -> ColorFilterMatrix {
    let b = non_negative(amount);
    linear([
        [b, 0.0, 0.0, 0.0],
        [0.0, b, 0.0, 0.0],
        [0.0, 0.0, b, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ])
}

fn contrast(amount: f32) -> ColorFilterMatrix {
    let c = non_negative(amount);
    let offset = 0.5 * (1.0 - c);
    ColorFilterMatrix {
        rows: [
            [c, 0.0, 0.0, 0.0],
            [0.0, c, 0.0, 0.0],
            [0.0, 0.0, c, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        offset: [offset, offset, offset, 0.0],
    }
}

fn invert(amount: f32) -> ColorFilterMatrix {
    let a = clamp01(amount);
    let scale = 1.0 - 2.0 * a;
    ColorFilterMatrix {
        rows: [
            [scale, 0.0, 0.0, 0.0],
            [0.0, scale, 0.0, 0.0],
            [0.0, 0.0, scale, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        offset: [a, a, a, 0.0],
    }
}

fn hue_rotate(degrees: f32) -> ColorFilterMatrix {
    let radians = finite_or(degrees, 0.0).to_radians();
    let (sin, cos) = radians.sin_cos();
    linear([
        [
            0.213 + cos * 0.787 - sin * 0.213,
            0.715 - cos * 0.715 - sin * 0.715,
            0.072 - cos * 0.072 + sin * 0.928,
            0.0,
        ],
        [
            0.213 - cos * 0.213 + sin * 0.143,
            0.715 + cos * 0.285 + sin * 0.140,
            0.072 - cos * 0.072 - sin * 0.283,
            0.0,
        ],
        [
            0.213 - cos * 0.213 - sin * 0.787,
            0.715 - cos * 0.715 + sin * 0.715,
            0.072 + cos * 0.928 + sin * 0.072,
            0.0,
        ],
        [0.0, 0.0, 0.0, 1.0],
    ])
}

fn opacity(amount: f32) -> ColorFilterMatrix {
    let a = clamp01(amount);
    linear([
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, a],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use hgpui::{ScaledFilter, ScaledPixels};

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-5,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn grayscale_one_uses_luma_weights() {
        let matrix = color_filter_matrix(&[ScaledFilter::Grayscale(1.0)]).unwrap();
        assert_close(matrix.rows[0][0], 0.2126);
        assert_close(matrix.rows[0][1], 0.7152);
        assert_close(matrix.rows[0][2], 0.0722);
        assert_close(matrix.rows[1][0], 0.2126);
        assert_close(matrix.rows[2][2], 0.0722);
        let filtered = matrix.apply([1.0, 0.0, 0.0, 1.0]);
        assert_close(filtered[0], 0.2126);
        assert_close(filtered[1], 0.2126);
        assert_close(filtered[2], 0.2126);
    }

    #[test]
    fn zero_contrast_pushes_everything_to_mid_gray() {
        let matrix = color_filter_matrix(&[ScaledFilter::Contrast(0.0)]).unwrap();
        let filtered = matrix.apply([0.9, 0.1, 0.4, 1.0]);
        assert_close(filtered[0], 0.5);
        assert_close(filtered[1], 0.5);
        assert_close(filtered[2], 0.5);
    }

    #[test]
    fn filters_compose_in_authored_order() {
        // brightness(2) then contrast(0.5): 0.5 * (2 * x) + 0.25.
        let matrix =
            color_filter_matrix(&[ScaledFilter::Brightness(2.0), ScaledFilter::Contrast(0.5)])
                .unwrap();
        let filtered = matrix.apply([0.3, 0.3, 0.3, 1.0]);
        assert_close(filtered[0], 0.55);

        // The reverse order differs: contrast around 0.5 first, then brightness.
        let reversed =
            color_filter_matrix(&[ScaledFilter::Contrast(0.5), ScaledFilter::Brightness(2.0)])
                .unwrap();
        let filtered = reversed.apply([0.3, 0.3, 0.3, 1.0]);
        assert_close(filtered[0], 0.8);
    }

    #[test]
    fn blur_only_chains_have_no_color_matrix() {
        assert_eq!(
            color_filter_matrix(&[ScaledFilter::Blur(ScaledPixels(8.0))]),
            None
        );
        assert_eq!(color_filter_matrix(&[]), None);
    }

    #[test]
    fn mixed_chain_keeps_the_color_filters() {
        let matrix = color_filter_matrix(&[
            ScaledFilter::Blur(ScaledPixels(8.0)),
            ScaledFilter::Brightness(2.0),
        ])
        .unwrap();
        let filtered = matrix.apply([0.25, 0.25, 0.25, 1.0]);
        assert_close(filtered[0], 0.5);
    }

    #[test]
    fn neutral_amounts_fold_to_identity() {
        let matrix = color_filter_matrix(&[
            ScaledFilter::Grayscale(0.0),
            ScaledFilter::Sepia(0.0),
            ScaledFilter::Saturate(1.0),
            ScaledFilter::Brightness(1.0),
            ScaledFilter::Contrast(1.0),
            ScaledFilter::Invert(0.0),
            ScaledFilter::HueRotate(0.0),
            ScaledFilter::Opacity(1.0),
        ])
        .unwrap();
        assert_eq!(matrix, ColorFilterMatrix::IDENTITY);
    }

    #[test]
    fn blurs_compose_by_adding_variance() {
        assert_eq!(
            combined_blur_radius(&[
                ScaledFilter::Blur(ScaledPixels(3.0)),
                ScaledFilter::Blur(ScaledPixels(4.0)),
            ]),
            5.0
        );
        assert_eq!(
            combined_blur_radius(&[
                ScaledFilter::Blur(ScaledPixels(5.0)),
                ScaledFilter::Grayscale(1.0),
            ]),
            5.0
        );
        assert_eq!(combined_blur_radius(&[]), 0.0);
    }

    #[test]
    fn opacity_scales_alpha_only() {
        let matrix = color_filter_matrix(&[ScaledFilter::Opacity(0.5)]).unwrap();
        let filtered = matrix.apply([0.4, 0.6, 0.8, 1.0]);
        assert_close(filtered[0], 0.4);
        assert_close(filtered[3], 0.5);
    }
}
