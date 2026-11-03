//! Shared scissor region for the separable gaussian blur passes.
//!
//! Both native GPU backends compute the same region: at half resolution a separable gaussian
//! reads one kernel reach (3σ) beyond every pixel it writes, so the scissored passes carry an
//! extra reach of valid data around the composite region. Without it, taps near the scissor
//! edge read stale texels from a previous frame and paint a contaminated band along the blur's
//! border.

use hgpui::{Bounds, ScaledPixels};

/// The half-resolution scissor region `[x, y, width, height]` the blur passes may write.
///
/// `clip_rounded` selects the backdrop path (composite samples only the element's box) versus
/// the content-filter path (composite samples the box dilated by the kernel reach). Returns
/// `None` when the region is empty, in which case the blur can be skipped entirely.
pub fn blur_scissor_region(
    bounds: Bounds<ScaledPixels>,
    blur_radius: f32,
    clip_rounded: bool,
    viewport_width: u32,
    viewport_height: u32,
) -> Option<[u32; 4]> {
    let half_w = (viewport_width / 2).max(1);
    let half_h = (viewport_height / 2).max(1);
    let kernel_reach = 3.0 * blur_radius; // full-resolution pixels
    let composite_dilation = if clip_rounded { 0.0 } else { kernel_reach };
    let scissor_dilation = composite_dilation + kernel_reach;
    let x0 = (((bounds.origin.x.0 - scissor_dilation) * 0.5)
        .floor()
        .max(0.0) as u32)
        .min(half_w);
    let y0 = (((bounds.origin.y.0 - scissor_dilation) * 0.5)
        .floor()
        .max(0.0) as u32)
        .min(half_h);
    let x1 = ((((bounds.origin.x.0 + bounds.size.width.0 + scissor_dilation) * 0.5)
        .ceil()
        .max(0.0) as u32)
        .min(half_w))
    .max(x0);
    let y1 = ((((bounds.origin.y.0 + bounds.size.height.0 + scissor_dilation) * 0.5)
        .ceil()
        .max(0.0) as u32)
        .min(half_h))
    .max(y0);
    let region = [x0, y0, x1 - x0, y1 - y0];
    (region[2] > 0 && region[3] > 0).then_some(region)
}

/// The integer region `[x, y, width, height]` a color-only filter composite covers, clamped to
/// the viewport.
///
/// Backdrop color filters copy this region into a scratch texture before compositing, because
/// the source and destination are the same render target. Copying only the element's box keeps
/// that extra pass proportional to the filtered area instead of the whole window.
pub fn filter_copy_region(
    bounds: Bounds<ScaledPixels>,
    viewport_width: u32,
    viewport_height: u32,
) -> Option<[u32; 4]> {
    let x0 = (bounds.origin.x.0.floor().max(0.0) as u32).min(viewport_width);
    let y0 = (bounds.origin.y.0.floor().max(0.0) as u32).min(viewport_height);
    let x1 = (((bounds.origin.x.0 + bounds.size.width.0).ceil().max(0.0)) as u32)
        .min(viewport_width)
        .max(x0);
    let y1 = (((bounds.origin.y.0 + bounds.size.height.0).ceil().max(0.0)) as u32)
        .min(viewport_height)
        .max(y0);
    let region = [x0, y0, x1 - x0, y1 - y0];
    (region[2] > 0 && region[3] > 0).then_some(region)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hgpui::{bounds, point, size};

    #[test]
    fn scissor_covers_the_element_band_not_the_viewport() {
        let titlebar = bounds(
            point(ScaledPixels(0.0), ScaledPixels(0.0)),
            size(ScaledPixels(1911.0), ScaledPixels(50.0)),
        );
        let region = blur_scissor_region(titlebar, 40.0, true, 1911, 1090)
            .expect("the titlebar band is non-empty");
        assert_eq!(region, [0, 0, 955, 85]);
        let half_viewport_area = (1911u64 / 2) * (1090u64 / 2);
        assert!(
            u64::from(region[2]) * u64::from(region[3]) * 4 < half_viewport_area,
            "the blur scissor must cover far less than the viewport"
        );

        // Content blur samples a region dilated by its kernel, so its scissor is taller.
        let content_region = blur_scissor_region(titlebar, 40.0, false, 1911, 1090)
            .expect("the dilated band is non-empty");
        assert_eq!(content_region[2], region[2]);
        assert!(content_region[3] > region[3]);
    }

    #[test]
    fn offscreen_blur_has_no_scissor_region() {
        let outside = bounds(
            point(ScaledPixels(-1000.0), ScaledPixels(-1000.0)),
            size(ScaledPixels(10.0), ScaledPixels(10.0)),
        );
        assert!(blur_scissor_region(outside, 4.0, true, 100, 100).is_none());
    }

    #[test]
    fn filter_copy_region_clamps_to_the_viewport() {
        let panel = bounds(
            point(ScaledPixels(10.5), ScaledPixels(-4.0)),
            size(ScaledPixels(100.2), ScaledPixels(40.0)),
        );
        assert_eq!(filter_copy_region(panel, 80, 100), Some([10, 0, 70, 36]));

        let outside = bounds(
            point(ScaledPixels(200.0), ScaledPixels(20.0)),
            size(ScaledPixels(10.0), ScaledPixels(10.0)),
        );
        assert_eq!(filter_copy_region(outside, 100, 100), None);
    }
}
