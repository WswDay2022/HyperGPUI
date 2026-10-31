//! Cross-frame cache for drop shadows.
//!
//! A drop shadow is a blurred rounded rectangle. Re-evaluating the Gaussian integral for every
//! pixel of a large shadow on every frame is the most expensive part of the shadow pipeline, yet
//! static controls repaint the exact same shadow over and over. This module bakes an eligible
//! shadow into a BGRA tile that lives in the window's sprite atlas, so subsequent frames only
//! sample a texture.
//!
//! Only solid-colour, non-inset shadows are eligible. Gradient colours, inset shadows, corner
//! smoothing, and tiles above the area budget keep using the analytic shader; that path remains
//! the correctness reference.

use collections::{FxHashMap, FxHashSet};
use image::{Frame, ImageBuffer, Rgba as ImageRgba};
use smallvec::SmallVec;
use std::sync::Arc;

use crate::{Bounds, Corners, RenderImage, SceneHsla, ScaledPixels};

/// The same Gaussian cutoff the fragment shader uses for the shadow's quad geometry.
pub(crate) const SHADOW_GAUSSIAN_CUTOFF: f32 = 3.0;
/// Number of integration samples per shadow pixel; mirrors `SHADOW_INTEGRATION_SAMPLE_COUNT`.
const SHADOW_INTEGRATION_SAMPLE_COUNT: usize = 4;
/// Rough error-function polynomial constants from the shader.
const ERF_LINEAR: f32 = 0.278393;
const ERF_QUADRATIC: f32 = 0.230389;
const ERF_CUBIC: f32 = 0.000972;
const ERF_QUARTIC: f32 = 0.078108;

/// Baking a tile is a one-off CPU cost on the paint thread; keep it bounded. Larger shadows
/// (the window root, big dialogs) take the analytic path, which is already cheap in the interior
/// after the coverage early-out.
const MAX_TILE_PIXELS: u64 = 512 * 512;
/// Bound the atlas memory held by stale shadow variants.
const MAX_CACHED_TILES: usize = 64;
/// Bound the CPU work spent baking tiles in one frame. If a frame surfaces many new shadow
/// variants (startup, theme switch), spread their tiles over several frames instead of
/// stalling one frame for all of them. The analytic path covers the gap.
const MAX_BAKE_PIXELS_PER_FRAME: u64 = 256 * 256;

/// The identity of a baked shadow tile.
///
/// The tile is translation invariant, so the absolute origin is not part of the key. The
/// fractional origin phase *is*: it determines where the pixel centres fall relative to the
/// shape, and therefore the sampled coverage. Sizes are stored bit-exactly so a subpixel resize
/// can never reuse a tile baked for a different shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ShadowTileKey {
    width: u32,
    height: u32,
    size_x: u32,
    size_y: u32,
    phase_x: u32,
    phase_y: u32,
    radii: [u32; 4],
    blur: u32,
    color: [u32; 4],
}

impl ShadowTileKey {
    pub(crate) fn new(
        shape_bounds: Bounds<ScaledPixels>,
        corner_radii: Corners<ScaledPixels>,
        blur: ScaledPixels,
        color: SceneHsla,
    ) -> Option<Self> {
        let radii = [
            corner_radii.top_left.0,
            corner_radii.top_right.0,
            corner_radii.bottom_right.0,
            corner_radii.bottom_left.0,
        ];
        if !blur.0.is_finite()
            || !(blur.0 > 0.0)
            || !shape_bounds.origin.x.0.is_finite()
            || !shape_bounds.origin.y.0.is_finite()
            || !shape_bounds.size.width.0.is_finite()
            || !shape_bounds.size.height.0.is_finite()
            || !(shape_bounds.size.width.0 > 0.0)
            || !(shape_bounds.size.height.0 > 0.0)
            || radii.iter().any(|radius| !radius.is_finite())
            || !color.h.is_finite()
            || !color.s.is_finite()
            || !color.l.is_finite()
            || !color.a.is_finite()
        {
            return None;
        }
        let margin = blur.0 * SHADOW_GAUSSIAN_CUTOFF;
        let width = (shape_bounds.size.width.0 + 2.0 * margin).ceil();
        let height = (shape_bounds.size.height.0 + 2.0 * margin).ceil();
        if width < 1.0 || height < 1.0 {
            return None;
        }
        let (width, height) = (width as u32, height as u32);
        if u64::from(width) * u64::from(height) > MAX_TILE_PIXELS {
            return None;
        }
        if !(color.a > 0.0) {
            return None;
        }
        Some(Self {
            width,
            height,
            size_x: shape_bounds.size.width.0.to_bits(),
            size_y: shape_bounds.size.height.0.to_bits(),
            phase_x: phase_bits(shape_bounds.origin.x.0),
            phase_y: phase_bits(shape_bounds.origin.y.0),
            radii: [
                radii[0].to_bits(),
                radii[1].to_bits(),
                radii[2].to_bits(),
                radii[3].to_bits(),
            ],
            blur: blur.0.to_bits(),
            color: [
                color.h.to_bits(),
                color.s.to_bits(),
                color.l.to_bits(),
                color.a.to_bits(),
            ],
        })
    }

    pub(crate) fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Quantizes a subpixel origin phase to 16 fractional bits. Float subtraction makes
/// `10.2.fract()` and `30.2.fract()` differ in the last bits even though both move the pixel
/// grid by the same amount; 1/65536 device pixels is far below visible precision.
fn phase_bits(value: f32) -> u32 {
    ((value.rem_euclid(1.0) * 65536.0).round() as u32) & 0xFFFF
}

/// Bakes `shape_bounds` (a cover-bounds rounded rectangle in device pixels) into a straight-alpha
/// BGRA image, using the same coverage model as the shadow fragment shader.
pub(crate) fn generate_shadow_tile(
    key: &ShadowTileKey,
    shape_bounds: Bounds<ScaledPixels>,
    corner_radii: Corners<ScaledPixels>,
    blur: ScaledPixels,
    color: SceneHsla,
) -> Arc<RenderImage> {
    let margin = blur.0 * SHADOW_GAUSSIAN_CUTOFF;
    let tile_origin_x = shape_bounds.origin.x.0 - margin;
    let tile_origin_y = shape_bounds.origin.y.0 - margin;
    let shape_center_x = shape_bounds.origin.x.0 + shape_bounds.size.width.0 * 0.5;
    let shape_center_y = shape_bounds.origin.y.0 + shape_bounds.size.height.0 * 0.5;
    let half_width = shape_bounds.size.width.0 * 0.5;
    let half_height = shape_bounds.size.height.0 * 0.5;
    let radii = [
        corner_radii.top_left.0,
        corner_radii.top_right.0,
        corner_radii.bottom_right.0,
        corner_radii.bottom_left.0,
    ];
    let [red, green, blue, color_alpha] = scene_hsla_to_rgba(color);

    let mut bytes = vec![0u8; key.width as usize * key.height as usize * 4];
    let mut offset = 0;
    for y in 0..key.height {
        let position_y = tile_origin_y + y as f32 + 0.5;
        for x in 0..key.width {
            let position_x = tile_origin_x + x as f32 + 0.5;
            let coverage = blurred_shadow_coverage(
                position_x - shape_center_x,
                position_y - shape_center_y,
                half_width,
                half_height,
                radii,
                blur.0,
            );
            let alpha = (color_alpha * coverage).clamp(0.0, 1.0);
            // Straight (non-premultiplied) BGRA: the sprite shader premultiplies at blend time.
            bytes[offset] = color_to_u8(blue);
            bytes[offset + 1] = color_to_u8(green);
            bytes[offset + 2] = color_to_u8(red);
            bytes[offset + 3] = (alpha * 255.0).round() as u8;
            offset += 4;
        }
    }

    let buffer = ImageBuffer::<ImageRgba<u8>, _>::from_raw(key.width, key.height, bytes)
        .expect("tile dimensions match the byte buffer");
    Arc::new(RenderImage::new(SmallVec::from_const([Frame::new(buffer)])))
}

fn color_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The exact HSL-to-sRGB formula the shadow fragment shader uses. Keeping it in sync matters:
/// the baked tile and the analytic fallback must produce the same colour for gradient-free
/// shadows.
fn scene_hsla_to_rgba(color: SceneHsla) -> [f32; 4] {
    let chroma = color.s * (1.0 - (2.0 * color.l - 1.0).abs());
    let channel = |offset: f32| {
        let phase = (color.h + offset).fract();
        let unit = ((phase * 6.0 - 3.0).abs() - 1.0).clamp(0.0, 1.0);
        (unit - 0.5) * chroma + color.l
    };
    [
        channel(0.0),
        channel(2.0 / 3.0),
        channel(1.0 / 3.0),
        color.a,
    ]
}

fn blurred_shadow_coverage(
    center_to_point_x: f32,
    center_to_point_y: f32,
    half_width: f32,
    half_height: f32,
    radii: [f32; 4],
    standard_deviation: f32,
) -> f32 {
    let corner_radius = pick_corner_radius(center_to_point_x, center_to_point_y, radii);
    let vertical_minimum = center_to_point_y - half_height;
    let vertical_maximum = center_to_point_y + half_height;
    let integration_radius = SHADOW_GAUSSIAN_CUTOFF * standard_deviation;
    let integration_start = (-integration_radius).clamp(vertical_minimum, vertical_maximum);
    let integration_end = integration_radius.clamp(vertical_minimum, vertical_maximum);
    let sample_step =
        (integration_end - integration_start) / SHADOW_INTEGRATION_SAMPLE_COUNT as f32;

    let mut coverage = 0.0;
    for sample_index in 0..SHADOW_INTEGRATION_SAMPLE_COUNT {
        let sample_position = integration_start
            + sample_step * (sample_index as f32 + 0.5);
        coverage += integrated_rounded_rectangle_coverage(
            center_to_point_x,
            center_to_point_y - sample_position,
            standard_deviation,
            corner_radius,
            half_width,
            half_height,
        ) * gaussian(sample_position, standard_deviation)
            * sample_step;
    }
    coverage
}

fn integrated_rounded_rectangle_coverage(
    horizontal_position: f32,
    vertical_position: f32,
    standard_deviation: f32,
    corner_radius: f32,
    half_width: f32,
    half_height: f32,
) -> f32 {
    let distance_beyond_corner =
        (half_height - corner_radius - vertical_position.abs()).min(0.0);
    let curved_extent = half_width - corner_radius
        + (corner_radius * corner_radius - distance_beyond_corner * distance_beyond_corner)
            .max(0.0)
            .sqrt();
    let scale = std::f32::consts::FRAC_1_SQRT_2 / standard_deviation;
    let lower = erf_approx((horizontal_position - curved_extent) * scale);
    let upper = erf_approx((horizontal_position + curved_extent) * scale);
    let integral_lower = 0.5 + 0.5 * lower;
    let integral_upper = 0.5 + 0.5 * upper;
    integral_upper - integral_lower
}

fn gaussian(position: f32, standard_deviation: f32) -> f32 {
    let variance = standard_deviation * standard_deviation;
    (-(position * position) / (2.0 * variance)).exp()
        / ((2.0 * std::f32::consts::PI).sqrt() * standard_deviation)
}

/// The same piecewise corner selection as the fragment shader: `<= 0` on an axis selects the
/// bottom/right radius, because the shader's `select(false, true, cond)` picks the true arm only
/// for strictly negative coordinates.
fn pick_corner_radius(x: f32, y: f32, radii: [f32; 4]) -> f32 {
    let left = if y < 0.0 {
        radii[0] // top-left
    } else {
        radii[3] // bottom-left
    };
    let right = if y < 0.0 {
        radii[1] // top-right
    } else {
        radii[2] // bottom-right
    };
    if x < 0.0 { left } else { right }
}

/// Abramowitz & Stegun 7.1.26, the same approximation the shader uses.
fn erf_approx(value: f32) -> f32 {
    if value == 0.0 {
        return 0.0;
    }
    let sign = value.signum();
    let absolute = value.abs();
    let polynomial = 1.0
        + absolute
            * (ERF_LINEAR
                + absolute
                    * (ERF_QUADRATIC
                        + absolute * (ERF_CUBIC + absolute * ERF_QUARTIC)));
    let squared = polynomial * polynomial;
    sign - sign / (squared * squared)
}

pub(crate) struct ShadowTile {
    pub(crate) image: Arc<RenderImage>,
    last_used_frame: u64,
}

pub(crate) enum ShadowTileLookup {
    Hit(Arc<RenderImage>),
    /// The key was first seen this frame. Use the analytic shadow for now; bake it if the same
    /// shape is still present on the next frame (which avoids regenerating during a resize drag).
    Deferred,
    /// The key was already pending last frame; bake it now.
    Generate,
    /// The key is stable, but this frame's baking budget is exhausted. The key stays pending
    /// for the next frame.
    BudgetDeferred,
}

/// Per-window cache of baked shadow tiles with a one-frame stability gate.
#[derive(Default)]
pub(crate) struct ShadowTileCache {
    entries: FxHashMap<ShadowTileKey, ShadowTile>,
    previous_pending: FxHashSet<ShadowTileKey>,
    current_pending: FxHashSet<ShadowTileKey>,
    frame: u64,
    baked_pixels_this_frame: u64,
}

impl ShadowTileCache {
    pub(crate) fn begin_frame(&mut self) {
        self.frame += 1;
        self.baked_pixels_this_frame = 0;
        std::mem::swap(&mut self.previous_pending, &mut self.current_pending);
        self.current_pending.clear();
    }

    pub(crate) fn lookup(&mut self, key: ShadowTileKey) -> ShadowTileLookup {
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used_frame = self.frame;
            return ShadowTileLookup::Hit(Arc::clone(&entry.image));
        }
        if self.previous_pending.contains(&key) {
            let pixels = u64::from(key.width) * u64::from(key.height);
            // A single tile may exceed the per-frame budget (large shadows are capped by
            // `MAX_TILE_PIXELS`); always let it through when nothing else was baked this frame,
            // otherwise it would starve forever.
            let fits = self.baked_pixels_this_frame == 0
                || self.baked_pixels_this_frame + pixels <= MAX_BAKE_PIXELS_PER_FRAME;
            if fits {
                self.baked_pixels_this_frame += pixels;
                return ShadowTileLookup::Generate;
            }
            // Keep the stable key pending; the analytic path covers this frame.
            self.current_pending.insert(key);
            return ShadowTileLookup::BudgetDeferred;
        }
        self.current_pending.insert(key);
        ShadowTileLookup::Deferred
    }

    /// Inserts a freshly baked tile, returning the least-recently-used entry evicted to stay
    /// within the cap. Entries used by the current frame are never evicted.
    pub(crate) fn insert(
        &mut self,
        key: ShadowTileKey,
        image: Arc<RenderImage>,
    ) -> Option<Arc<RenderImage>> {
        self.current_pending.remove(&key);
        self.entries.insert(
            key,
            ShadowTile {
                image,
                last_used_frame: self.frame,
            },
        );
        if self.entries.len() <= MAX_CACHED_TILES {
            return None;
        }
        // Keep one frame of slack before dropping an atlas tile: the previous frame's GPU
        // submission may still be reading it when the new frame starts baking.
        let oldest = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.last_used_frame + 1 < self.frame)
            .min_by_key(|(_, entry)| entry.last_used_frame)
            .map(|(key, _)| *key);
        oldest.and_then(|key| self.entries.remove(&key).map(|entry| entry.image))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{point, size, hsla};

    fn shape() -> Bounds<ScaledPixels> {
        Bounds {
            origin: point(ScaledPixels(10.0), ScaledPixels(20.0)),
            size: size(ScaledPixels(40.0), ScaledPixels(24.0)),
        }
    }

    fn radii(radius: f32) -> Corners<ScaledPixels> {
        Corners {
            top_left: ScaledPixels(radius),
            top_right: ScaledPixels(radius),
            bottom_right: ScaledPixels(radius),
            bottom_left: ScaledPixels(radius),
        }
    }

    #[test]
    fn bake_budget_spreads_large_tiles_over_frames() {
        let color = hsla(0.0, 0.0, 0.0, 0.5).into();
        let at = |x: f32, width: f32| Bounds {
            origin: point(ScaledPixels(x), ScaledPixels(20.0)),
            size: size(ScaledPixels(width), ScaledPixels(200.0)),
        };
        let first = ShadowTileKey::new(
            at(10.0, 200.0),
            radii(6.0),
            ScaledPixels(4.0),
            color,
        )
        .unwrap();
        let second = ShadowTileKey::new(
            at(300.0, 201.0),
            radii(6.0),
            ScaledPixels(4.0),
            color,
        )
        .unwrap();
        assert_ne!(first, second, "the budget fixture needs two distinct tiles");
        assert!(
            u64::from(first.width) * u64::from(first.height) * 2
                > MAX_BAKE_PIXELS_PER_FRAME,
            "the fixture must exceed one frame's budget when both tiles are baked"
        );

        let mut cache = ShadowTileCache::default();
        cache.begin_frame();
        let _ = cache.lookup(first);
        let _ = cache.lookup(second);
        cache.begin_frame();
        assert!(matches!(cache.lookup(first), ShadowTileLookup::Generate));
        assert!(matches!(
            cache.lookup(second),
            ShadowTileLookup::BudgetDeferred
        ));
        cache.begin_frame();
        assert!(matches!(cache.lookup(second), ShadowTileLookup::Generate));
    }

    #[test]
    fn keys_separate_subpixel_sizes_and_reuse_integer_translations() {
        let color = hsla(0.0, 0.0, 0.0, 0.5).into();
        let at = |x: f32, y: f32, width: f32, height: f32| {
            Bounds {
                origin: point(ScaledPixels(x), ScaledPixels(y)),
                size: size(ScaledPixels(width), ScaledPixels(height)),
            }
        };
        let first = ShadowTileKey::new(
            at(10.2, 20.2, 40.1, 24.1),
            radii(6.0),
            ScaledPixels(4.0),
            color,
        )
        .unwrap();
        let translated = ShadowTileKey::new(
            at(30.2, 40.2, 40.1, 24.1),
            radii(6.0),
            ScaledPixels(4.0),
            color,
        )
        .unwrap();
        assert_eq!(first, translated, "integer translations share a tile");

        let resized = ShadowTileKey::new(
            at(10.2, 20.2, 40.9, 24.9),
            radii(6.0),
            ScaledPixels(4.0),
            color,
        )
        .unwrap();
        assert_ne!(first, resized, "subpixel resizes must not reuse a tile");
    }

    #[test]
    fn hsla_conversion_matches_the_shader_formula() {
        let cyan: SceneHsla = hsla(0.5, 1.0, 0.5, 0.75).into();
        let [red, green, blue, alpha] = scene_hsla_to_rgba(cyan);
        assert!(red.abs() < 1e-5, "hue 0.5 should have no red, got {red}");
        assert!((green - 1.0).abs() < 1e-5, "hue 0.5 should be fully green, got {green}");
        assert!((blue - 1.0).abs() < 1e-5, "hue 0.5 should be fully blue, got {blue}");
        assert!((alpha - 0.75).abs() < 1e-5);
    }

    #[test]
    fn coverage_matches_the_shadow_model_at_known_points() {
        let radii = [6.0; 4];
        let center = blurred_shadow_coverage(0.0, 0.0, 20.0, 12.0, radii, 4.0);
        assert!(
            (center - 1.0).abs() < 0.01,
            "a point well inside the shape contracts to full coverage, got {center}"
        );
        let edge = blurred_shadow_coverage(-20.0, 0.0, 20.0, 12.0, radii, 4.0);
        assert!(
            (edge - 0.5).abs() < 0.03,
            "the straight edge should be half covered, got {edge}"
        );
        let outside = blurred_shadow_coverage(-40.0, 0.0, 20.0, 12.0, radii, 4.0);
        assert!(outside < 0.01, "content beyond the blur radius should be clear, got {outside}");
        let mirrored = blurred_shadow_coverage(20.0, 0.0, 20.0, 12.0, radii, 4.0);
        assert!((edge - mirrored).abs() < 1e-6, "the model must be symmetric");
        let top_corner = blurred_shadow_coverage(-20.0, -12.0, 20.0, 12.0, radii, 4.0);
        let bottom_corner = blurred_shadow_coverage(-20.0, 12.0, 20.0, 12.0, radii, 4.0);
        assert!((top_corner - bottom_corner).abs() < 1e-6);
        assert!(
            top_corner < 0.25,
            "the rounded corner should be mostly clear, got {top_corner}"
        );
    }

    #[test]
    fn generated_tile_is_solid_at_center_and_transparent_at_corners() {
        let bounds = shape();
        let color = hsla(0.0, 0.0, 0.0, 0.5).into();
        let key = ShadowTileKey::new(bounds, radii(6.0), ScaledPixels(4.0), color)
            .expect("small shadow is cacheable");
        let image = generate_shadow_tile(&key, bounds, radii(6.0), ScaledPixels(4.0), color);
        let bytes = image.as_bytes(0).unwrap();
        let stride = key.width as usize * 4;
        let center_x = key.width as usize / 2;
        let center_y = key.height as usize / 2;
        let center = &bytes[center_y * stride + center_x * 4..][..4];
        assert!(center[3] > 120, "centre alpha should be ~half of 255, got {center:?}");
        assert_eq!(&center[..3], &[0, 0, 0], "tile keeps the shadow's straight colour");
        let corner = &bytes[..4];
        assert!(corner[3] < 8, "the blurred corner should be transparent, got {corner:?}");
    }

    #[test]
    fn cache_defers_then_uses_the_stable_key() {
        let bounds = shape();
        let color = hsla(0.0, 0.0, 0.0, 0.5).into();
        let key = ShadowTileKey::new(bounds, radii(6.0), ScaledPixels(4.0), color).unwrap();
        let mut cache = ShadowTileCache::default();

        cache.begin_frame();
        assert!(matches!(cache.lookup(key), ShadowTileLookup::Deferred));
        cache.begin_frame();
        assert!(matches!(cache.lookup(key), ShadowTileLookup::Generate));

        let image = generate_shadow_tile(&key, bounds, radii(6.0), ScaledPixels(4.0), color);
        let id = image.id;
        assert!(cache.insert(key, image).is_none());
        cache.begin_frame();
        match cache.lookup(key) {
            ShadowTileLookup::Hit(image) => assert_eq!(image.id, id),
            _ => panic!("a stable key must hit the cache"),
        }
    }

    #[test]
    fn oversized_shadows_are_not_cacheable() {
        let bounds = Bounds {
            origin: point(ScaledPixels(0.0), ScaledPixels(0.0)),
            size: size(ScaledPixels(2000.0), ScaledPixels(2000.0)),
        };
        assert!(ShadowTileKey::new(
            bounds,
            radii(0.0),
            ScaledPixels(8.0),
            hsla(0.0, 0.0, 0.0, 0.5).into()
        )
        .is_none());
    }
}
