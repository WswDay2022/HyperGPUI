//! Shared sampling geometry for the separable Gaussian blur passes (DX11, WGPU and Metal).

use hgpui::{Bounds, ScaledPixels};

/// Gaussian kernel and scissors for the three half-resolution passes. Scissors
/// are `[x, y, width, height]` in the unmodified half-resolution viewport.
#[derive(Clone, Copy, Debug)]
pub struct BlurPassPlan {
    /// Geometry sampled by the final full-resolution composite.
    pub composite_bounds: Bounds<ScaledPixels>,
    /// Standard deviation in half-resolution texels.
    pub sigma: f32,
    /// Number of taps on either side of the center, excluding the center.
    pub tap_count: f32,
    /// Spacing between taps in half-resolution texels.
    pub tap_step: f32,
    /// Source-to-ping pass: both axes need a kernel halo.
    pub downsample: [u32; 4],
    /// Ping-to-pong pass: only the vertical pass needs a halo from this output.
    pub horizontal: [u32; 4],
    /// Pong-to-ping pass: only texels read by the composite need to be written.
    pub vertical: [u32; 4],
}

/// Plans a blur backwards from the texels read by the visible composite. The shader
/// still uses the full-size viewports and the same taps, UVs and clear operations.
/// Returns `None` if there is no Gaussian blur or the composite is fully offscreen.
pub fn blur_pass_plan(
    bounds: Bounds<ScaledPixels>,
    blur_radius: f32,
    clip_rounded: bool,
    viewport_width: u32,
    viewport_height: u32,
) -> Option<BlurPassPlan> {
    let sigma = blur_radius * 0.5;
    if !sigma.is_finite() || sigma <= 0.0 || viewport_width == 0 || viewport_height == 0 {
        return None;
    }
    let ideal_taps = (3.0 * sigma).ceil();
    let tap_count = ideal_taps.clamp(1.0, 32.0);
    let tap_step = (ideal_taps / tap_count).max(1.0);
    let composite_bounds = if clip_rounded {
        bounds
    } else {
        bounds.dilate(ScaledPixels(3.0 * blur_radius))
    };
    let half_w = (viewport_width / 2).max(1);
    let half_h = (viewport_height / 2).max(1);
    let footprint = |lo: f32, hi: f32, full: u32, half: u32| {
        let lo = lo.max(0.0);
        let hi = hi.min(full as f32);
        if lo >= hi {
            return None;
        }
        // The composite samples at position / viewport, so an odd full dimension
        // cannot be mapped with a simple divide by two. Linear sampling needs the
        // texel on each side of the sample (u * size - 0.5, before clamping).
        let to_texel = |v: f32| f64::from(v) * f64::from(half) / f64::from(full) - 0.5;
        let start = (to_texel(lo).floor() as u32).min(half - 1);
        let end = ((to_texel(hi).floor() + 2.0) as u32).clamp(start + 1, half);
        Some((start, end))
    };
    let (x0, x1) = footprint(
        composite_bounds.left().0,
        composite_bounds.right().0,
        viewport_width,
        half_w,
    )?;
    let (y0, y1) = footprint(
        composite_bounds.top().0,
        composite_bounds.bottom().0,
        viewport_height,
        half_h,
    )?;
    // Use the actual furthest tap, not the unrounded 3σ radius. Reserve one
    // additional texel for linear filtering and GPU coordinate roundoff.
    let halo = ((tap_count * tap_step).ceil() as u32).saturating_add(1);
    let hx0 = x0.saturating_sub(halo);
    let hx1 = x1.saturating_add(halo).min(half_w);
    let hy0 = y0.saturating_sub(halo);
    let hy1 = y1.saturating_add(halo).min(half_h);
    Some(BlurPassPlan {
        composite_bounds,
        sigma,
        tap_count,
        tap_step,
        downsample: [hx0, hy0, hx1 - hx0, hy1 - hy0],
        horizontal: [x0, hy0, x1 - x0, hy1 - hy0],
        vertical: [x0, y0, x1 - x0, y1 - y0],
    })
}

/// The legacy shared half-resolution scissor region `[x, y, width, height]`.
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
    fn titlebar_vertical_pass_only_writes_texels_read_by_composite() {
        let titlebar = bounds(
            point(ScaledPixels(0.0), ScaledPixels(0.0)),
            size(ScaledPixels(1920.0), ScaledPixels(50.0)),
        );
        let legacy = blur_scissor_region(titlebar, 40.0, true, 1920, 1080).unwrap();
        let plan = blur_pass_plan(titlebar, 40.0, true, 1920, 1080).unwrap();
        assert_eq!(legacy, [0, 0, 960, 85]);
        assert_eq!(plan.vertical, [0, 0, 960, 26]);
        assert_eq!(plan.horizontal, [0, 0, 960, 87]);
        assert_eq!(plan.downsample, [0, 0, 960, 87]);
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
