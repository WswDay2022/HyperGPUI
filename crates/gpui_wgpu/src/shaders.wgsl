/* Functions useful for debugging:

// A heat map color for debugging (blue -> cyan -> green -> yellow -> red).
fn heat_map_color(value: f32, minValue: f32, maxValue: f32, position: vec2<f32>) -> vec4<f32> {
    // Normalize value to 0-1 range
    let t = clamp((value - minValue) / (maxValue - minValue), 0.0, 1.0);

    // Heat map color calculation
    let r = t * t;
    let g = 4.0 * t * (1.0 - t);
    let b = (1.0 - t) * (1.0 - t);
    let heat_color = vec3<f32>(r, g, b);

    // Create a checkerboard pattern (black and white)
    let sum = floor(position.x / 3) + floor(position.y / 3);
    let is_odd = fract(sum * 0.5); // 0.0 for even, 0.5 for odd
    let checker_value = is_odd * 2.0; // 0.0 for even, 1.0 for odd
    let checker_color = vec3<f32>(checker_value);

    // Determine if value is in range (1.0 if in range, 0.0 if out of range)
    let in_range = step(minValue, value) * step(value, maxValue);

    // Mix checkerboard and heat map based on whether value is in range
    let final_color = mix(checker_color, heat_color, in_range);

    return vec4<f32>(final_color, 1.0);
}

*/

// Contrast and gamma correction adapted from https://github.com/microsoft/terminal/blob/1283c0f5b99a2961673249fa77c6b986efb5086c/src/renderer/atlas/dwrite.hlsl
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.
fn color_brightness(color: vec3<f32>) -> f32 {
    // REC. 601 luminance coefficients for perceived brightness
    return dot(color, vec3<f32>(0.30, 0.59, 0.11));
}

fn light_on_dark_contrast(enhancedContrast: f32, color: vec3<f32>) -> f32 {
    let brightness = color_brightness(color);
    let multiplier = saturate(4.0 * (0.75 - brightness));
    return enhancedContrast * multiplier;
}

fn enhance_contrast(alpha: f32, k: f32) -> f32 {
    return alpha * (k + 1.0) / (alpha * k + 1.0);
}

fn enhance_contrast3(alpha: vec3<f32>, k: f32) -> vec3<f32> {
    return alpha * (k + 1.0) / (alpha * k + 1.0);
}

fn apply_alpha_correction(a: f32, b: f32, g: vec4<f32>) -> f32 {
    let brightness_adjustment = g.x * b + g.y;
    let correction = brightness_adjustment * a + (g.z * b + g.w);
    return a + a * (1.0 - a) * correction;
}

fn apply_alpha_correction3(a: vec3<f32>, b: vec3<f32>, g: vec4<f32>) -> vec3<f32> {
    let brightness_adjustment = g.x * b + g.y;
    let correction = brightness_adjustment * a + (g.z * b + g.w);
    return a + a * (1.0 - a) * correction;
}

fn apply_contrast_and_gamma_correction(sample: f32, color: vec3<f32>, enhanced_contrast_factor: f32, gamma_ratios: vec4<f32>) -> f32 {
    let enhanced_contrast = light_on_dark_contrast(enhanced_contrast_factor, color);
    let brightness = color_brightness(color);

    let contrasted = enhance_contrast(sample, enhanced_contrast);
    return apply_alpha_correction(contrasted, brightness, gamma_ratios);
}

fn apply_contrast_and_gamma_correction3(sample: vec3<f32>, color: vec3<f32>, enhanced_contrast_factor: f32, gamma_ratios: vec4<f32>) -> vec3<f32> {
    let enhanced_contrast = light_on_dark_contrast(enhanced_contrast_factor, color);

    let contrasted = enhance_contrast3(sample, enhanced_contrast);
    return apply_alpha_correction3(contrasted, color, gamma_ratios);
}

struct GlobalParams {
    viewport_size: vec2<f32>,
    premultiplied_alpha: u32,
    pad: u32,
}

struct GammaParams {
    gamma_ratios: vec4<f32>,
    grayscale_enhanced_contrast: f32,
    subpixel_enhanced_contrast: f32,
    is_bgr: u32,
    pad: u32,
}

@group(0) @binding(0) var<uniform> globals: GlobalParams;
@group(0) @binding(1) var<uniform> gamma_params: GammaParams;
@group(1) @binding(1) var t_sprite: texture_2d<f32>;
@group(1) @binding(2) var s_sprite: sampler;

const M_PI_F: f32 = 3.1415926;
const GRAYSCALE_FACTORS: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

struct Bounds {
    origin: vec2<f32>,
    size: vec2<f32>,
}

struct Corners {
    top_left: f32,
    top_right: f32,
    bottom_right: f32,
    bottom_left: f32,
}

struct Edges {
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
}

struct Hsla {
    h: f32,
    s: f32,
    l: f32,
    a: f32,
}

struct LinearColorStop {
    color: Hsla,
    percentage: f32,
}

struct Background {
    // 0u is Solid
    // 1u is LinearGradient
    // 2u is PatternSlash
    // 3u is Checkerboard
    // 4u is RadialGradient
    // 5u is RadialGradientCircle
    tag: u32,
    // 0u is sRGB linear color
    // 1u is Oklab color
    color_space: u32,
    solid: Hsla,
    gradient_angle_or_pattern_height: f32,
    colors: array<LinearColorStop, 2>,
    // Radial gradients store the center's vertical fraction of the element's size here; padding
    // for alignment otherwise.
    radial_center_y: f32,
    // The ending-shape size keyword of a radial gradient (0 = farthest-corner, 1 = closest-side,
    // 2 = farthest-side, 3 = closest-corner); unused by other backgrounds.
    radial_size: u32,
    // Keeps the struct 8-byte sized: `transformation` in `Quad`/`PathVertex` is a `mat2x2`, which
    // WGSL aligns to 8, so it must land on the same offset as in the Rust layout.
    pad: u32,
}

struct AtlasTextureId {
    index: u32,
    kind: u32,
}

struct AtlasBounds {
    origin: vec2<i32>,
    size: vec2<i32>,
}

struct AtlasTile {
    texture_id: AtlasTextureId,
    tile_id: u32,
    padding: u32,
    bounds: AtlasBounds,
}

struct TransformationMatrix {
    rotation_scale: mat2x2<f32>,
    translation: vec2<f32>,
}

fn to_device_position_impl(position: vec2<f32>) -> vec4<f32> {
    let device_position = position / globals.viewport_size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    return vec4<f32>(device_position, 0.0, 1.0);
}

fn to_device_position(unit_vertex: vec2<f32>, bounds: Bounds) -> vec4<f32> {
    let position = unit_vertex * vec2<f32>(bounds.size) + bounds.origin;
    return to_device_position_impl(position);
}

fn to_tile_position(unit_vertex: vec2<f32>, tile: AtlasTile) -> vec2<f32> {
  let atlas_size = vec2<f32>(textureDimensions(t_sprite, 0));
  return (vec2<f32>(tile.bounds.origin) + unit_vertex * vec2<f32>(tile.bounds.size)) / atlas_size;
}

// The exact clip: the fragment is discarded at the mask's edge. This is what a primitive that
// has no cut of its own must use — the path rasterizer draws through this and nothing else, so
// padding it here would let every path and SVG leak a pixel out of every clip.
fn distance_from_clip_rect_impl(position: vec2<f32>, clip_bounds: Bounds) -> vec4<f32> {
    let tl = position - clip_bounds.origin;
    let br = clip_bounds.origin + clip_bounds.size - position;
    return vec4<f32>(tl.x, br.x, tl.y, br.y);
}

// The same, a margin looser. `content_mask_coverage` cuts the mask's *whole* outline again in
// the fragment — with a one-pixel ramp, so the edge follows the arc instead of the pixel grid —
// and this clip is then only the coarse cull that keeps fragments far outside from being shaded
// at all. Only the primitives that apply that cut may use this, or the margin becomes a hole.
fn distance_from_clip_rect_aa_impl(position: vec2<f32>, clip_bounds: Bounds) -> vec4<f32> {
    return distance_from_clip_rect_impl(position, clip_bounds) + AA_MARGIN;
}

fn distance_from_clip_rect(unit_vertex: vec2<f32>, bounds: Bounds, clip_bounds: Bounds) -> vec4<f32> {
    let position = unit_vertex * vec2<f32>(bounds.size) + bounds.origin;
    return distance_from_clip_rect_aa_impl(position, clip_bounds);
}


// How far outside its own outline a primitive is still rasterized, in device pixels. The
// rasterizer only emits fragments for pixels whose *centre* is inside the primitive, so a
// shader that shapes its own edge from a signed distance field — as every shader here does —
// can otherwise only ever *shrink* coverage: an edge landing in the far half of a pixel has no
// fragment to taper, and its partial pixel is lost outright, always in the same direction,
// which also shifts the edge's apparent position onto the pixel grid. Expanding the geometry
// by this much gives the distance ramp a fragment in every pixel it can reach; the ramp then
// drives the coverage to zero on its own well inside the margin, so nothing is painted
// outside the shape.
const AA_MARGIN: f32 = 1.0;

// Per-axis expansion, in a primitive's local units, that widens its local rect by `AA_MARGIN`
// device pixels on every side. `m` maps local units to device pixels with its columns the
// images of the local axes, so the local rect has to grow by `AA_MARGIN * |axis y| / |det|`
// along x for the edges that run along y to move `AA_MARGIN` pixels outward (and the mirror
// expression along y).
fn aa_margin_local(m: mat2x2<f32>) -> vec2<f32> {
    let axes = transpose(m);
    let lengths = vec2<f32>(length(axes[0]), length(axes[1]));
    let det = abs(axes[0].x * axes[1].y - axes[0].y * axes[1].x);
    return AA_MARGIN * lengths.yx / max(det, 1e-3);
}

// Coverage of a distance field, one device pixel wide, from the field's value at the pixel
// centre and its change across that pixel along each screen axis.
//
// Dividing the distance by `|grad|` turns it into pixels measured from the pixel centre, and
// integrating a straight edge across a pixel gives exactly `saturate(0.5 - pixels)`. The
// gradient has to be the field's *screen-space* gradient for that to hold under a transform:
// the caller measures it by sampling the same field one pixel apart, using the interpolated
// local position's own derivatives as the step.
//
// Those samples straddle the pixel centre, which is what keeps this accurate where the field
// curves: a one-sided difference over a whole pixel reads the slope tens of percent low at a
// corner of a pixel or two radius, and the widened ramp it produces then swallows the arc.
fn sdf_coverage(centre: f32, slope_x: f32, slope_y: f32) -> f32 {
    let per_pixel = max(length(vec2<f32>(slope_x, slope_y)), 1e-6);
    return saturate(0.5 - centre / per_pixel);
}

// --- exact area coverage for a rounded corner --- //
//
// The one-pixel ramp above is exact for a straight edge but only first order where the boundary
// curves, and a corner's arc is exactly that: measured against a 32x32 supersampled reference it
// is off by up to 0.06 of a pixel (15 levels of 255) — a corner a shade too full, which is the
// "not as smooth as CSS" complaint. Skia's CPU rasterizer resolves curves by area rather than by
// a ramp (`SkScan_AAAPath.cpp`: "we analytically compute the coverage of this horizontal strip
// ... ground-truth coverages"), and the one curve a rounded rectangle has is in closed form.
//
// A pixel's coverage is the area of the pixel inside the shape, and the shape near a corner is
// the rectangle minus the notch the arc cuts out of the corner's `radius x radius` square. The
// rectangle's coverage is exact per axis (a half-plane over a pixel is a ramp). The notch is that
// square clipped to the pixel, minus the area of the same box inside the disc — the circle's
// antiderivative, with the box split at the disc's axes because the disc's area is even in both.

// `integral from 0 to x of sqrt(r^2 - t^2) dt`: the area under the circle's upper half out to x.
fn circle_area_to(x: f32, r: f32) -> f32 {
    let reach = clamp(x, 0.0, r);
    let half_width = sqrt(max(r * r - reach * reach, 0.0));
    // `max(r, ...)`: a zero radius must not turn the division into a NaN, and its term is zero
    // anyway.
    let angle = asin(clamp(reach / max(r, 1e-6), -1.0, 1.0));
    return 0.5 * (reach * half_width + r * r * angle);
}

// The area of [0, x] x [0, y] inside the disc of radius r centred at the origin: the circle's
// half-width integrated along x, clipped to the box's height.
fn disc_area_first_quadrant(x: f32, y: f32, r: f32) -> f32 {
    let reach = clamp(x, 0.0, r);
    let height = clamp(y, 0.0, r);
    // Past this point the circle's half-width is below the box's top, so the integrand is the
    // box's height rather than the circle.
    let pinch = sqrt(max(r * r - height * height, 0.0));
    let flat = min(pinch, reach);
    return height * flat + (circle_area_to(reach, r) - circle_area_to(flat, r));
}

// The area of the axis-aligned box [lo, hi] inside the disc of radius r centred at the origin,
// for a box anywhere in the plane. `disc_area_first_quadrant` only measures boxes that start at
// the axes, so the box is split there first — keeping each piece's distance from the axis, which
// is what decides whether it is anywhere near the disc — and each piece is then the four-corner
// sum of that cumulative area. A piece that does not reach its side of the axis has both its ends
// at zero, and its four-corner sum cancels to nothing.
fn disc_area_box(lo: vec2<f32>, hi: vec2<f32>, r: f32) -> f32 {
    let x_neg = vec2<f32>(max(-min(hi.x, 0.0), 0.0), max(-lo.x, 0.0));
    let x_pos = vec2<f32>(max(lo.x, 0.0), max(hi.x, 0.0));
    let y_neg = vec2<f32>(max(-min(hi.y, 0.0), 0.0), max(-lo.y, 0.0));
    let y_pos = vec2<f32>(max(lo.y, 0.0), max(hi.y, 0.0));
    return
        // x below the axis, y below the axis
        disc_area_first_quadrant(x_neg.y, y_neg.y, r)
        - disc_area_first_quadrant(x_neg.x, y_neg.y, r)
        - disc_area_first_quadrant(x_neg.y, y_neg.x, r)
        + disc_area_first_quadrant(x_neg.x, y_neg.x, r)
        // x below the axis, y above it
        + disc_area_first_quadrant(x_neg.y, y_pos.y, r)
        - disc_area_first_quadrant(x_neg.x, y_pos.y, r)
        - disc_area_first_quadrant(x_neg.y, y_pos.x, r)
        + disc_area_first_quadrant(x_neg.x, y_pos.x, r)
        // x above the axis, y below it
        + disc_area_first_quadrant(x_pos.y, y_neg.y, r)
        - disc_area_first_quadrant(x_pos.x, y_neg.y, r)
        - disc_area_first_quadrant(x_pos.y, y_neg.x, r)
        + disc_area_first_quadrant(x_pos.x, y_neg.x, r)
        // x above the axis, y above it
        + disc_area_first_quadrant(x_pos.y, y_pos.y, r)
        - disc_area_first_quadrant(x_pos.x, y_pos.y, r)
        - disc_area_first_quadrant(x_pos.y, y_pos.x, r)
        + disc_area_first_quadrant(x_pos.x, y_pos.x, r);
}

// The exact coverage of a pixel at `corner_to_point` (the point relative to the corner, both
// components <= 0 inside the quad, as the quad's SDF mirrors it) by a quad whose corner has the
// given radius. `step` is one screen pixel in the quad's local units, per axis — valid only when
// the transform maps the pixel to an axis-aligned box, which is why callers check that first.
fn rounded_corner_coverage(corner_to_point: vec2<f32>, step: vec2<f32>, radius: f32) -> f32 {
    // The rectangle, whose two straight edges are exact per axis.
    let rect = clamp(0.5 - corner_to_point.x / step.x, 0.0, 1.0)
        * clamp(0.5 - corner_to_point.y / step.y, 0.0, 1.0);
    // The pixel's box in this mirrored space, clipped to the square the arc is inscribed in —
    // only there does the rounding cut anything.
    let half = 0.5 * step;
    var lo = max(corner_to_point - half, vec2<f32>(-radius));
    let hi = min(corner_to_point + half, vec2<f32>(0.0));
    lo = min(lo, hi);
    let box_area = (hi.x - lo.x) * (hi.y - lo.y);
    // The arc's centre is the corner of that square; shift it to the disc's origin.
    let disc_area = disc_area_box(lo + vec2<f32>(radius), hi + vec2<f32>(radius), radius);
    // Both areas are in local units squared and the pixel is `step` of them per side.
    return clamp(rect - (box_area - disc_area) / (step.x * step.y), 0.0, 1.0);
}

// https://gamedev.stackexchange.com/questions/92015/optimized-linear-to-srgb-glsl
fn srgb_to_linear(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    let lower = srgb / vec3<f32>(12.92);
    return select(higher, lower, cutoff);
}

fn srgb_to_linear_component(a: f32) -> f32 {
    let cutoff = a < 0.04045;
    let higher = pow((a + 0.055) / 1.055, 2.4);
    let lower = a / 12.92;
    return select(higher, lower, cutoff);
}

fn linear_to_srgb(linear: vec3<f32>) -> vec3<f32> {
    let cutoff = linear < vec3<f32>(0.0031308);
    let higher = vec3<f32>(1.055) * pow(linear, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    let lower = linear * vec3<f32>(12.92);
    return select(higher, lower, cutoff);
}

/// Convert a linear color to sRGBA space.
fn linear_to_srgba(color: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(linear_to_srgb(color.rgb), color.a);
}

/// Convert a sRGBA color to linear space.
fn srgba_to_linear(color: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(srgb_to_linear(color.rgb), color.a);
}

/// Hsla to linear RGBA conversion.
fn hsla_to_rgba(hsla: Hsla) -> vec4<f32> {
    let h = hsla.h * 6.0; // Now, it's an angle but scaled in [0, 6) range
    let s = hsla.s;
    let l = hsla.l;
    let a = hsla.a;

    let c = (1.0 - abs(2.0 * l - 1.0)) * s;
    let x = c * (1.0 - abs(h % 2.0 - 1.0));
    let m = l - c / 2.0;
    var color = vec3<f32>(m);

    if (h >= 0.0 && h < 1.0) {
        color.r += c;
        color.g += x;
    } else if (h >= 1.0 && h < 2.0) {
        color.r += x;
        color.g += c;
    } else if (h >= 2.0 && h < 3.0) {
        color.g += c;
        color.b += x;
    } else if (h >= 3.0 && h < 4.0) {
        color.g += x;
        color.b += c;
    } else if (h >= 4.0 && h < 5.0) {
        color.r += x;
        color.b += c;
    } else {
        color.r += c;
        color.b += x;
    }

    return vec4<f32>(color, a);
}

/// Convert a linear sRGB to Oklab space.
/// Reference: https://bottosson.github.io/posts/oklab/#converting-from-linear-srgb-to-oklab
fn linear_srgb_to_oklab(color: vec4<f32>) -> vec4<f32> {
	let l = 0.4122214708 * color.r + 0.5363325363 * color.g + 0.0514459929 * color.b;
	let m = 0.2119034982 * color.r + 0.6806995451 * color.g + 0.1073969566 * color.b;
	let s = 0.0883024619 * color.r + 0.2817188376 * color.g + 0.6299787005 * color.b;

	let l_ = pow(l, 1.0 / 3.0);
	let m_ = pow(m, 1.0 / 3.0);
	let s_ = pow(s, 1.0 / 3.0);

	return vec4<f32>(
		0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
		1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
		0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
		color.a
	);
}

/// Convert an Oklab color to linear sRGB space.
fn oklab_to_linear_srgb(color: vec4<f32>) -> vec4<f32> {
	let l_ = color.r + 0.3963377774 * color.g + 0.2158037573 * color.b;
	let m_ = color.r - 0.1055613458 * color.g - 0.0638541728 * color.b;
	let s_ = color.r - 0.0894841775 * color.g - 1.2914855480 * color.b;

	let l = l_ * l_ * l_;
	let m = m_ * m_ * m_;
	let s = s_ * s_ * s_;

	return vec4<f32>(
		4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
		-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
		-0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
		color.a
	);
}

fn over(below: vec4<f32>, above: vec4<f32>) -> vec4<f32> {
    let alpha = above.a + below.a * (1.0 - above.a);
    let color = (above.rgb * above.a + below.rgb * below.a * (1.0 - above.a)) / alpha;
    return vec4<f32>(color, alpha);
}

// A standard gaussian function, used for weighting samples
fn gaussian(x: f32, sigma: f32) -> f32{
    return exp(-(x * x) / (2.0 * sigma * sigma)) / (sqrt(2.0 * M_PI_F) * sigma);
}

// This approximates the error function, needed for the gaussian integral
fn erf(v: vec2<f32>) -> vec2<f32> {
    let s = sign(v);
    let a = abs(v);
    let r1 = 1.0 + (0.278393 + (0.230389 + (0.000972 + 0.078108 * a) * a) * a) * a;
    let r2 = r1 * r1;
    return s - s / (r2 * r2);
}

fn blur_along_x(x: f32, y: f32, sigma: f32, corner: f32, half_size: vec2<f32>) -> f32 {
  let delta = min(half_size.y - corner - abs(y), 0.0);
  let curved = half_size.x - corner + sqrt(max(0.0, corner * corner - delta * delta));
  let integral = 0.5 + 0.5 * erf((x + vec2<f32>(-curved, curved)) * (sqrt(0.5) / sigma));
  return integral.y - integral.x;
}

// Selects corner radius based on quadrant.
fn pick_corner_radius(center_to_point: vec2<f32>, radii: Corners) -> f32 {
    if (center_to_point.x < 0.0) {
        if (center_to_point.y < 0.0) {
            return radii.top_left;
        } else {
            return radii.bottom_left;
        }
    } else {
        if (center_to_point.y < 0.0) {
            return radii.top_right;
        } else {
            return radii.bottom_right;
        }
    }
}

// Signed distance of the point to the quad's border - positive outside the
// border, and negative inside.
//
// See comments on similar code using `quad_sdf_impl` in `fs_quad` for
// explanation.
fn quad_sdf(point: vec2<f32>, bounds: Bounds, corner_radii: Corners) -> f32 {
    let half_size = bounds.size / 2.0;
    let center = bounds.origin + half_size;
    let center_to_point = point - center;
    let corner_radius = pick_corner_radius(center_to_point, corner_radii);
    let corner_to_point = abs(center_to_point) - half_size;
    let corner_center_to_point = corner_to_point + corner_radius;
    return quad_sdf_impl(corner_center_to_point, corner_radius);
}

fn quad_sdf_impl(corner_center_to_point: vec2<f32>, corner_radius: f32) -> f32 {
    if (corner_radius == 0.0) {
        // Fast path for unrounded corners.
        return max(corner_center_to_point.x, corner_center_to_point.y);
    } else {
        // Signed distance of the point from a quad that is inset by corner_radius.
        // It is negative inside this quad, and positive outside.
        let signed_distance_to_inset_quad =
            // 0 inside the inset quad, and positive outside.
            length(max(vec2<f32>(0.0), corner_center_to_point)) +
            // 0 outside the inset quad, and negative inside.
            min(0.0, max(corner_center_to_point.x, corner_center_to_point.y));

        return signed_distance_to_inset_quad - corner_radius;
    }
}

// The quad's own outline as a distance field of a point in its local space. The corner radius
// is supplied rather than picked here so that the samples `fs_quad` takes around the pixel
// centre all describe one corner's field - `pick_corner_radius` switches quadrant at the
// centre lines, where the radii of neighbouring corners may differ.
fn quad_own_sdf(point: vec2<f32>, half_size: vec2<f32>, corner_radius: f32) -> f32 {
    let corner_to_point = abs(point - half_size) - half_size;
    return quad_sdf_impl(corner_to_point + corner_radius, corner_radius);
}

// The border's inner edge as a distance field of a point in the quad's local space, mirroring
// the branch `fs_quad` picks at the pixel centre (0 straight, 1 provably outside, 2 circular,
// 3 ellipse) so the samples around the centre stay on one continuous field.
fn quad_inner_sdf(point: vec2<f32>, half_size: vec2<f32>, corner_radius: f32,
                  reduced_border: vec2<f32>, outer_sdf: f32, branch: u32) -> f32 {
    let corner_to_point = abs(point - half_size) - half_size;
    if (branch == 0u) {
        let straight = corner_to_point + reduced_border;
        return -max(straight.x, straight.y);
    } else if (branch == 1u) {
        return -1.0;
    } else if (branch == 2u) {
        return -(outer_sdf + reduced_border.x);
    } else {
        let ellipse_radii = max(vec2<f32>(0.0), corner_radius - reduced_border);
        return quarter_ellipse_sdf(corner_to_point + corner_radius, ellipse_radii);
    }
}

// Abstract away the final color transformation based on the
// target alpha compositing mode.
// Content-mask coverage, as an alpha multiplier.
//
// The whole mask — straight edges as well as rounded corners — is shaped here from its
// distance field, so an edge that does not land on the pixel grid tapers over exactly one
// pixel instead of being cut off hard. `clip_distances` still culls the primitive, but it is
// padded by `AA_MARGIN` (see `distance_from_clip_rect_impl`) and never decides the edge.
//
// The mask-equals-own case (`owns_cutout` with a mask identical to the element's own rounded
// cutout) is skipped because the element's own SDF would otherwise cut the same edge twice.
fn content_mask_coverage(mask_point: vec2<f32>, mask_bounds: Bounds, mask_radii: Corners,
                         own_origin: vec2<f32>, own_size: vec2<f32>, own_radii: Corners,
                         owns_cutout: bool) -> f32 {
    if (owns_cutout &&
            all(mask_bounds.origin == own_origin) &&
            all(mask_bounds.size == own_size) &&
            mask_radii.top_left == own_radii.top_left &&
            mask_radii.top_right == own_radii.top_right &&
            mask_radii.bottom_left == own_radii.bottom_left &&
            mask_radii.bottom_right == own_radii.bottom_right) {
        return 1.0;
    }
    // Inside a rounded corner's quadrant the cut is the pixel's exact area, the same as the
    // quad's own corner — a clip that lags the shape it clips by four levels of 255 is visible
    // as the shape overflowing its own container at the corners. The mask lives in scene space,
    // which *is* device pixels, so a pixel there is always a unit axis-aligned box and this is
    // always available. (Do not instead average `0.5 - d` over quarter-pixel samples to "smooth"
    // it: that ramp is the fraction of a one-pixel box *centred on the sample*, so a shifted
    // sample undershoots — a pixel the mask fully covers would come out at 0.875 — and the mask
    // would eat into everything it clips.)
    let mask_half_size = mask_bounds.size / 2.0;
    let mask_center_to_point = mask_point - (mask_bounds.origin + mask_half_size);
    let mask_corner_radius = pick_corner_radius(mask_center_to_point, mask_radii);
    let mask_corner_to_point = abs(mask_center_to_point) - mask_half_size;
    if (mask_corner_to_point.x + mask_corner_radius >= 0.0 &&
            mask_corner_to_point.y + mask_corner_radius >= 0.0) {
        return rounded_corner_coverage(mask_corner_to_point, vec2<f32>(1.0), mask_corner_radius);
    }
    return saturate(0.5 - quad_sdf(mask_point, mask_bounds, mask_radii));
}

fn blend_color(color: vec4<f32>, alpha_factor: f32) -> vec4<f32> {
    let alpha = color.a * alpha_factor;
    let multiplier = select(1.0, alpha, globals.premultiplied_alpha != 0u);
    return vec4<f32>(color.rgb * multiplier, alpha);
}


struct GradientColor {
    solid: vec4<f32>,
    color0: vec4<f32>,
    color1: vec4<f32>,
}

fn prepare_gradient_color(tag: u32, color_space: u32,
    solid: Hsla, colors: array<LinearColorStop, 2>) -> GradientColor {
    var result = GradientColor();

    if (tag == 0u || tag == 2u || tag == 3u) {
        result.solid = hsla_to_rgba(solid);
    } else if (tag == 1u || tag == 4u || tag == 5u) {
        // `hsla_to_rgba` returns the color in the render target's encoding (non-linear sRGB),
        // which is also the space CSS `in srgb` interpolates in, so the `ColorSpace::Srgb` case
        // needs no conversion at all — the fragment shader mixes these values directly.
        result.color0 = hsla_to_rgba(colors[0].color);
        result.color1 = hsla_to_rgba(colors[1].color);

        // Convert to the Oklab color space in the vertex shader to keep the per-pixel work in
        // the fragment shader to the mix and the conversion back.
        if (color_space == 1u) {
            // Oklab
            result.color0 = linear_srgb_to_oklab(srgba_to_linear(result.color0));
            result.color1 = linear_srgb_to_oklab(srgba_to_linear(result.color1));
        }
    }

    return result;
}

/// Resolves a gradient's ramp parameter into a color: applies the color stops, mixes in the
/// requested color space, and dithers to hide 8-bit banding. Shared by the linear and radial
/// gradients so both ramps behave identically.
fn gradient_ramp_color(background: Background, t: f32, position: vec2<f32>,
    color0: vec4<f32>, color1: vec4<f32>) -> vec4<f32> {
    // Adjust t based on the stop percentages.
    let stop_t = clamp((t - background.colors[0].percentage)
        / (background.colors[1].percentage - background.colors[0].percentage), 0.0, 1.0);

    var color: vec4<f32>;
    switch (background.color_space) {
        // sRGB: the stops are already in the interpolating space (the target's encoding).
        default: {
            color = mix(color0, color1, stop_t);
        }
        case 1u: {
            let oklab_color = mix(color0, color1, stop_t);
            color = linear_to_srgba(oklab_to_linear_srgb(oklab_color));
        }
    }

    // Dither to reduce banding in gradients (especially dark/alpha). Triangular-distributed noise
    // breaks up 8-bit quantization steps: ±2/255 for RGB (enough for dark-on-dark compositing),
    // ±3/255 for alpha (needs more because alpha × dark color = tiny steps).
    let seed = position * 0.6180339887; // golden ratio spread
    let r1 = fract(sin(dot(seed, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    let r2 = fract(sin(dot(seed, vec2<f32>(39.3460, 11.135))) * 24634.6345);
    let tri = r1 + r2 - 1.0; // triangular PDF, range [-1, +1]
    // WGSL has no assignments to swizzles, so add the noise as a whole vector.
    color += tri * vec4<f32>(2.0, 2.0, 2.0, 3.0) / 255.0;
    return color;
}

// Stripe pattern (background tag 2): the signed distance from `pt` to the nearest stripe edge,
// measured across the stripes. `period` is the stripe period and `half_stripe` half the painted
// stripe's width, both in the pattern's own (local) units.
fn stripe_sdf(pt: vec2<f32>, origin: vec2<f32>, rotation: mat2x2<f32>, period: f32,
              half_stripe: f32) -> f32 {
    let rotated = rotation * (pt - origin);
    let wrapped = rotated.x % period;
    return min(wrapped, period - wrapped) - half_stripe;
}

// Checkerboard (background tag 3): the signed distance from `pt` to the nearest cell edge, in
// local units. The coloured cells are half the board, so the field is symmetric.
fn checker_sdf(pt: vec2<f32>, origin: vec2<f32>, cell: f32) -> f32 {
    let cell_position = fract((pt - origin) / cell);
    let edge = min(cell_position, vec2<f32>(1.0) - cell_position);
    return min(edge.x, edge.y) * cell;
}

// Coverage of a pattern at `pt`, one screen pixel wide. `pixel_x`/`pixel_y` are one screen pixel
// expressed in the pattern's own space, taken by the caller from the interpolated local
// position's derivatives, so a scaled or skewed pattern keeps a one-pixel ramp like everything
// else. The samples straddle the pixel centre for the reason in `sdf_coverage`: a stripe is only
// a few pixels wide, so a one-sided slope reads far too low.
fn stripe_coverage(pt: vec2<f32>, origin: vec2<f32>, rotation: mat2x2<f32>, period: f32,
                   half_stripe: f32, pixel_x: vec2<f32>, pixel_y: vec2<f32>) -> f32 {
    let centre = stripe_sdf(pt, origin, rotation, period, half_stripe);
    // Not `sdf_coverage`: a pattern's field is symmetric about the very edge the coverage is
    // about, so its derivative reads zero there and the ramp would collapse to a hard step.
    // How far a screen pixel moves in the pattern's space never vanishes.
    let per_pixel = max(0.5 * (length(pixel_x) + length(pixel_y)), 1e-5);
    return saturate(0.5 - centre / per_pixel);
}

fn checker_coverage(pt: vec2<f32>, origin: vec2<f32>, cell: f32,
                    pixel_x: vec2<f32>, pixel_y: vec2<f32>) -> f32 {
    let centre = checker_sdf(pt, origin, cell);
    let per_pixel = max(0.5 * (length(pixel_x) + length(pixel_y)), 1e-5);
    return saturate(0.5 - centre / per_pixel);
}

fn gradient_color(background: Background, position: vec2<f32>, bounds: Bounds,
    solid_color: vec4<f32>, color0: vec4<f32>, color1: vec4<f32>) -> vec4<f32> {
    var background_color = vec4<f32>(0.0);

    switch (background.tag) {
        default: {
            return solid_color;
        }
        case 1u: {
            // Linear gradient background.
            // -90 degrees to match the CSS gradient angle.
            let angle = background.gradient_angle_or_pattern_height;
            let radians = (angle % 360.0 - 90.0) * M_PI_F / 180.0;
            var direction = vec2<f32>(cos(radians), sin(radians));

            // Expand the short side to be the same as the long side
            if (bounds.size.x > bounds.size.y) {
                direction.y *= bounds.size.y / bounds.size.x;
            } else {
                direction.x *= bounds.size.x / bounds.size.y;
            }

            // Get the t value for the linear gradient with the color stop percentages.
            let half_size = bounds.size / 2.0;
            let center = bounds.origin + half_size;
            let center_to_point = position - center;
            var t = dot(center_to_point, direction) / length(direction);
            // Check the direct to determine the use x or y
            if (abs(direction.x) > abs(direction.y)) {
                t = (t + half_size.x) / bounds.size.x;
            } else {
                t = (t + half_size.y) / bounds.size.y;
            }

            background_color = gradient_ramp_color(background, t, position, color0, color1);
        }
        case 2u: {
            // pattern slash
            let gradient_angle_or_pattern_height = background.gradient_angle_or_pattern_height;
            let pattern_width = (gradient_angle_or_pattern_height / 65535.0f) / 255.0f;
            let pattern_interval = (gradient_angle_or_pattern_height % 65535.0f) / 255.0f;
            let pattern_height = pattern_width + pattern_interval;
            let stripe_angle = M_PI_F / 4.0;
            let pattern_period = pattern_height * sin(stripe_angle);
            let rotation = mat2x2<f32>(
                cos(stripe_angle), -sin(stripe_angle),
                sin(stripe_angle), cos(stripe_angle)
            );
            let half_stripe = pattern_period * (pattern_width / pattern_height) * 0.5;
            background_color = solid_color;
            background_color.a *= stripe_coverage(position, bounds.origin, rotation,
                pattern_period, half_stripe, dpdx(position), dpdy(position));
        }
        case 3u: {
            // checkerboard
            let size = background.gradient_angle_or_pattern_height;
            let relative_position = position - bounds.origin;

            let x_index = floor(relative_position.x / size);
            let y_index = floor(relative_position.y / size);
            let should_be_colored = (x_index + y_index) % 2.0;

            // Antialias the cell edges, or the alternating squares show hard diagonal
            // staircases wherever they meet at an angle.
            let cell_coverage = checker_coverage(position, bounds.origin, size,
                dpdx(position), dpdy(position));

            background_color = solid_color;
            background_color.a *= saturate(should_be_colored) * cell_coverage;
        }
        case 4u, 5u: {
            // Radial gradient: CSS `radial-gradient(<shape> <size> at <center>, ...)`. The center
            // is stored as fractions of the element's size and may lie outside it.
            let center = bounds.origin
                + vec2<f32>(background.gradient_angle_or_pattern_height,
                    background.radial_center_y)
                    * bounds.size;
            let offset = position - center;
            // The ending-shape size: side sizes use the distance from the center to the
            // nearest/farthest side on each axis, corner sizes scale those by sqrt(2) so the
            // ending shape meets the chosen corner exactly at t = 1.
            let is_closest = background.radial_size == 1u || background.radial_size == 3u;
            let is_corner = background.radial_size == 0u || background.radial_size == 3u;
            let closest_side = min(center - bounds.origin, bounds.origin + bounds.size - center);
            let farthest_side = max(center - bounds.origin, bounds.origin + bounds.size - center);
            let side = select(farthest_side, closest_side, is_closest);
            // Ellipses take the per-axis distances; circles take a single radius — the nearest or
            // farthest side, or the corner distance.
            var radii = select(side, side * 1.41421356, is_corner);
            if (background.tag == 5u) {
                let side_radius = select(max(side.x, side.y), min(side.x, side.y), is_closest);
                radii = vec2<f32>(select(side_radius, length(side), is_corner));
            }
            // A zero-sized box would divide by zero; fall back to the last stop.
            var t = 1.0;
            if (radii.x > 0.0 && radii.y > 0.0) {
                t = length(offset / radii);
            }
            background_color = gradient_ramp_color(background, t, position, color0, color1);
        }
    }

    return background_color;
}

// --- quads --- //

struct Quad {
    order: u32,
    border_style: u32,
    // Non-zero when the quad paints only its fully covered interior — a later layer repaints
    // the same outline, so the antialiased band is left to it. See
    // `Scene::collapse_covered_outlines`.
    suppress_partial_coverage: u32,
    pad: u32,
    bounds: Bounds,
    content_mask: Bounds,
    content_mask_corner_radii: Corners,
    background: Background,
    border_color: Hsla,
    corner_radii: Corners,
    border_widths: Edges,
    // Must match the Rust `scene::Quad` layout (216 bytes, repr(C)):
    // `transformation` sits at byte offset 192 (mat2x2<f32> aligns to 8).
    transformation: TransformationMatrix,
}
@group(1) @binding(0) var<storage, read> b_quads: array<Quad>;

struct QuadVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) border_color: vec4<f32>,
    @location(1) @interpolate(flat) quad_id: u32,
    // TODO: use `clip_distance` once Naga supports it
    @location(2) clip_distances: vec4<f32>,
    @location(3) @interpolate(flat) background_solid: vec4<f32>,
    @location(4) @interpolate(flat) background_color0: vec4<f32>,
    @location(5) @interpolate(flat) background_color1: vec4<f32>,
    // Position in the quad's untransformed (local) coordinate space, used by the fragment
    // shader for the SDF/gradient so transforms don't distort corners or borders.
    @location(6) local_position: vec2<f32>,
}

@vertex
fn vs_quad(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> QuadVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let quad = b_quads[instance_id];

    var out = QuadVarying();
    // Grow the local rect so the rasterizer still emits fragments for the pixels the coverage
    // ramp reaches outside the outline (see `AA_MARGIN`).
    let margin = aa_margin_local(quad.transformation.rotation_scale);
    let local_position = unit_vertex * (vec2<f32>(quad.bounds.size) + 2.0 * margin)
        + (quad.bounds.origin - margin);
    // Apply the transform around the quad's center, matching `CssTransform::to_matrix` on the
    // Rust side. The Rust side stores the matrix row-major, so transpose to column-major first.
    let transformed_position =
        transpose(quad.transformation.rotation_scale) * local_position + quad.transformation.translation;
    out.position = to_device_position_impl(transformed_position);
    out.local_position = local_position;

    let gradient = prepare_gradient_color(
        quad.background.tag,
        quad.background.color_space,
        quad.background.solid,
        quad.background.colors
    );
    out.background_solid = gradient.solid;
    out.background_color0 = gradient.color0;
    out.background_color1 = gradient.color1;
    out.border_color = hsla_to_rgba(quad.border_color);
    out.quad_id = instance_id;
    // A mask that coincides with the quad's own bounds is the quad's own cutout (see `fs_quad`):
    // the straight-edge clip would slice the transformed quad at its untransformed rect, so it
    // is skipped — the quad's own local-space outline already bounds the painted area.
    let mask_is_own = all(quad.content_mask.origin == quad.bounds.origin) &&
        all(quad.content_mask.size == quad.bounds.size);
    out.clip_distances = select(
        distance_from_clip_rect_aa_impl(transformed_position, quad.content_mask),
        vec4<f32>(1.0),
        mask_is_own);
    return out;
}

@fragment
fn fs_quad(input: QuadVarying) -> @location(0) vec4<f32> {
    // Alpha clip first, since we don't have `clip_distance`.
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    let quad = b_quads[input.quad_id];

    // Rounded content-mask cutout: `vs_quad` clipped the mask's straight edges (via
    // `clip_distances`), so only the mask's rounded-corner arcs are cut here. A mask that
    // coincides with the quad's own *untransformed* bounds and radii is the quad's own rounded
    // cutout: the own SDF below already cuts that outline in local space (so it follows any
    // transform), and cutting it again in scene space would slice it at the untransformed rect
    // under rotation/skew/scale.
    let mask_coverage = content_mask_coverage(
        transpose(quad.transformation.rotation_scale) * input.local_position
            + quad.transformation.translation,
        quad.content_mask, quad.content_mask_corner_radii,
        quad.bounds.origin, quad.bounds.size,
        quad.corner_radii,
        true);

    let background_color = gradient_color(quad.background, input.local_position, quad.bounds,
        input.background_solid, input.background_color0, input.background_color1);

    let unrounded = quad.corner_radii.top_left == 0.0 &&
        quad.corner_radii.bottom_left == 0.0 &&
        quad.corner_radii.top_right == 0.0 &&
        quad.corner_radii.bottom_right == 0.0;
    let size = quad.bounds.size;
    let half_size = size / 2.0;
    let point = input.local_position - quad.bounds.origin;
    let center_to_point = point - half_size;

    // Signed distance field threshold for inclusion of pixels. 0.5 is the
    // minimum distance between the center of the pixel and the edge.
    let antialias_threshold = 0.5;

    // Vector from the corner of the quad bounds to the point, after mirroring
    // the point into the bottom right quadrant. Both components are <= 0.
    let corner_to_point = abs(center_to_point) - half_size;

    // Radius of the nearest corner, and the vector from the point to the centre of its circle,
    // also mirrored into the bottom right quadrant.
    let corner_radius = pick_corner_radius(center_to_point, quad.corner_radii);
    let corner_center_to_point = corner_to_point + corner_radius;

    // One screen pixel, expressed in the quad's local space. The interpolated local position is
    // linear in the pixel coordinates, so these derivatives are exact for the affine transform a
    // quad carries - including rotation, skew and non-uniform scale, where the two steps differ
    // in both length and direction. Taken here, before any branch: a derivative needs the whole
    // 2x2 quad's lanes still live.
    let pixel_x = dpdx(input.local_position);
    let pixel_y = dpdy(input.local_position);

    // Signed distance of the point to the outside edge of the quad's border, and the quad's own
    // coverage of it. The coverage samples the same field half a pixel either side of the
    // centre, which is what keeps the ramp one pixel wide where the field curves.
    //
    // Inside a rounded corner's quadrant the coverage is the pixel's exact area instead: that is
    // where the boundary curves, and the ramp is only first order there (see
    // `rounded_corner_coverage`). It needs the pixel to be an axis-aligned box in the quad's
    // local space, which a rotated or skewed transform does not give it.
    let outer_sdf = quad_own_sdf(point, half_size, corner_radius);
    let near_rounded_corner = corner_center_to_point.x >= 0.0 && corner_center_to_point.y >= 0.0;
    let axis_aligned = quad.transformation.rotation_scale[0][1] == 0.0 &&
        quad.transformation.rotation_scale[1][0] == 0.0;
    var own_coverage = sdf_coverage(outer_sdf,
        quad_own_sdf(point + 0.5 * pixel_x, half_size, corner_radius)
            - quad_own_sdf(point - 0.5 * pixel_x, half_size, corner_radius),
        quad_own_sdf(point + 0.5 * pixel_y, half_size, corner_radius)
            - quad_own_sdf(point - 0.5 * pixel_y, half_size, corner_radius));
    if (axis_aligned && near_rounded_corner) {
        own_coverage = rounded_corner_coverage(corner_to_point,
            vec2<f32>(length(pixel_x), length(pixel_y)), corner_radius);
    }

    var coverage = min(own_coverage, mask_coverage);

    // A layer that repaints an outline another, later layer covers contributes nothing but the
    // interior: its band would be a second application of the same coverage, and stacked
    // coverages compose as `1 - (1 - c)^n` - the arc's partial pixels merge into a hard step,
    // which is what flattens a small radius into a straight diagonal. See
    // `Scene::collapse_covered_outlines`.
    if (quad.suppress_partial_coverage != 0u) {
        coverage = select(0.0, 1.0, coverage >= 1.0);
    }

    // Fast path when the quad is not rounded and doesn't have any border: the outer distance
    // above is the whole story. (There is no transform-dependent variant any more - the SDF
    // coverage is exact for axis-aligned, scaled, skewed and rotated quads alike, so a quad that
    // is merely translated or scaled no longer needs a separate path, and the old one returned
    // the background with no coverage at all.)
    if (quad.border_widths.top == 0.0 &&
            quad.border_widths.left == 0.0 &&
            quad.border_widths.right == 0.0 &&
            quad.border_widths.bottom == 0.0 &&
            unrounded) {
        return blend_color(background_color, coverage);
    }

    // Width of the nearest borders
    let border = vec2<f32>(
        select(
            quad.border_widths.right,
            quad.border_widths.left,
            center_to_point.x < 0.0),
        select(
            quad.border_widths.bottom,
            quad.border_widths.top,
            center_to_point.y < 0.0));

    // 0-width borders are reduced so that `inner_sdf >= antialias_threshold`.
    // The purpose of this is to not draw antialiasing pixels in this case.
    let reduced_border =
        vec2<f32>(select(border.x, -antialias_threshold, border.x == 0.0),
                  select(border.y, -antialias_threshold, border.y == 0.0));

    // Whether the nearest point on the border is rounded
    let is_near_rounded_corner =
            corner_center_to_point.x >= 0 &&
            corner_center_to_point.y >= 0;

    // Vector from straight border inner corner to point.
    let straight_border_inner_corner_to_point = corner_to_point + reduced_border;

    // Whether the point is beyond the inner edge of the straight border.
    let is_beyond_inner_straight_border =
            straight_border_inner_corner_to_point.x > 0 ||
            straight_border_inner_corner_to_point.y > 0;

    // Whether the point is far enough inside the quad, such that the pixels are
    // not affected by the straight border.
    let is_within_inner_straight_border =
        straight_border_inner_corner_to_point.x < -antialias_threshold &&
        straight_border_inner_corner_to_point.y < -antialias_threshold;

    // Fast path for points that must be part of the background.
    //
    // This could be optimized further for large rounded corners by including
    // points in an inscribed rectangle, or some other quick linear check.
    // However, that might negatively impact performance in the case of
    // reasonable sizes for rounded corners.
    if (is_within_inner_straight_border && !is_near_rounded_corner) {
        return blend_color(background_color, coverage);
    }

    // Approximate signed distance of the point to the inside edge of the quad's
    // border. It is negative outside this edge (within the border), and
    // positive inside.
    //
    // This is not always an accurate signed distance:
    // * The rounded portions with varying border width use an approximation of
    //   nearest-point-on-ellipse.
    // * When it is quickly known to be outside the edge, -1.0 is used.
    var inner_branch = 3u;
    if (corner_center_to_point.x <= 0 || corner_center_to_point.y <= 0) {
        // Fast paths for straight borders.
        inner_branch = 0u;
    } else if (is_beyond_inner_straight_border) {
        // Fast path for points that must be outside the inner edge.
        inner_branch = 1u;
    } else if (reduced_border.x == reduced_border.y) {
        // Fast path for circular inner edge.
        inner_branch = 2u;
    }
    let inner_sdf = quad_inner_sdf(point, half_size, corner_radius, reduced_border, outer_sdf,
        inner_branch);

    // Negative when inside the border
    let border_sdf = max(inner_sdf, outer_sdf);

    var color = background_color;
    if (border_sdf < antialias_threshold) {
        var border_color = input.border_color;

        // Dashed border logic when border_style == 1
        if (quad.border_style == 1) {
            // Position along the perimeter in "dash space", where each dash
            // period has length 1
            var t = 0.0;

            // Total number of dash periods, so that the dash spacing can be
            // adjusted to evenly divide it
            var max_t = 0.0;

            // Border width is proportional to dash size. This is the behavior
            // used by browsers, but also avoids dashes from different segments
            // overlapping when dash size is smaller than the border width.
            //
            // Dash pattern: (2 * border width) dash, (1 * border width) gap
            let dash_length_per_width = 2.0;
            let dash_gap_per_width = 1.0;
            let dash_period_per_width = dash_length_per_width + dash_gap_per_width;

            // Since the dash size is determined by border width, the density of
            // dashes varies. Multiplying a pixel distance by this returns a
            // position in dash space - it has units (dash period / pixels). So
            // a dash velocity of (1 / 10) is 1 dash every 10 pixels.
            var dash_velocity = 0.0;

            // Dividing this by the border width gives the dash velocity
            let dv_numerator = 1.0 / dash_period_per_width;

            if (unrounded) {
                // When corners aren't rounded, the dashes are separately laid
                // out on each straight line, rather than around the whole
                // perimeter. This way each line starts and ends with a dash.
                let is_horizontal =
                        corner_center_to_point.x <
                        corner_center_to_point.y;

                // When applying dashed borders to just some, not all, the sides.
                // The way we chose border widths above sometimes comes with a 0 width value.
                // So we choose again to avoid division by zero.
                // TODO: A better solution exists taking a look at the whole file.
                // this does not fix single dashed borders at the corners
                let dashed_border = vec2<f32>(
                        max(
                            quad.border_widths.bottom,
                            quad.border_widths.top,
                        ),
                        max(
                            quad.border_widths.right,
                            quad.border_widths.left,
                        )
                   );

                let border_width = select(dashed_border.y, dashed_border.x, is_horizontal);
                dash_velocity = dv_numerator / border_width;
                t = select(point.y, point.x, is_horizontal) * dash_velocity;
                max_t = select(size.y, size.x, is_horizontal) * dash_velocity;
            } else {
                // When corners are rounded, the dashes are laid out clockwise
                // around the whole perimeter.

                let r_tr = quad.corner_radii.top_right;
                let r_br = quad.corner_radii.bottom_right;
                let r_bl = quad.corner_radii.bottom_left;
                let r_tl = quad.corner_radii.top_left;

                let w_t = quad.border_widths.top;
                let w_r = quad.border_widths.right;
                let w_b = quad.border_widths.bottom;
                let w_l = quad.border_widths.left;

                // Straight side dash velocities
                let dv_t = select(dv_numerator / w_t, 0.0, w_t <= 0.0);
                let dv_r = select(dv_numerator / w_r, 0.0, w_r <= 0.0);
                let dv_b = select(dv_numerator / w_b, 0.0, w_b <= 0.0);
                let dv_l = select(dv_numerator / w_l, 0.0, w_l <= 0.0);

                // Straight side lengths in dash space
                let s_t = (size.x - r_tl - r_tr) * dv_t;
                let s_r = (size.y - r_tr - r_br) * dv_r;
                let s_b = (size.x - r_br - r_bl) * dv_b;
                let s_l = (size.y - r_bl - r_tl) * dv_l;

                let corner_dash_velocity_tr = corner_dash_velocity(dv_t, dv_r);
                let corner_dash_velocity_br = corner_dash_velocity(dv_b, dv_r);
                let corner_dash_velocity_bl = corner_dash_velocity(dv_b, dv_l);
                let corner_dash_velocity_tl = corner_dash_velocity(dv_t, dv_l);

                // Corner lengths in dash space
                let c_tr = r_tr * (M_PI_F / 2.0) * corner_dash_velocity_tr;
                let c_br = r_br * (M_PI_F / 2.0) * corner_dash_velocity_br;
                let c_bl = r_bl * (M_PI_F / 2.0) * corner_dash_velocity_bl;
                let c_tl = r_tl * (M_PI_F / 2.0) * corner_dash_velocity_tl;

                // Cumulative dash space upto each segment
                let upto_tr = s_t;
                let upto_r = upto_tr + c_tr;
                let upto_br = upto_r + s_r;
                let upto_b = upto_br + c_br;
                let upto_bl = upto_b + s_b;
                let upto_l = upto_bl + c_bl;
                let upto_tl = upto_l + s_l;
                max_t = upto_tl + c_tl;

                if (is_near_rounded_corner) {
                    let radians = atan2(corner_center_to_point.y,
                                        corner_center_to_point.x);
                    let corner_t = radians * corner_radius;

                    if (center_to_point.x >= 0.0) {
                        if (center_to_point.y < 0.0) {
                            dash_velocity = corner_dash_velocity_tr;
                            // Subtracted because radians is pi/2 to 0 when
                            // going clockwise around the top right corner,
                            // since the y axis has been flipped
                            t = upto_r - corner_t * dash_velocity;
                        } else {
                            dash_velocity = corner_dash_velocity_br;
                            // Added because radians is 0 to pi/2 when going
                            // clockwise around the bottom-right corner
                            t = upto_br + corner_t * dash_velocity;
                        }
                    } else {
                        if (center_to_point.y >= 0.0) {
                            dash_velocity = corner_dash_velocity_bl;
                            // Subtracted because radians is pi/2 to 0 when
                            // going clockwise around the bottom-left corner,
                            // since the x axis has been flipped
                            t = upto_l - corner_t * dash_velocity;
                        } else {
                            dash_velocity = corner_dash_velocity_tl;
                            // Added because radians is 0 to pi/2 when going
                            // clockwise around the top-left corner, since both
                            // axis were flipped
                            t = upto_tl + corner_t * dash_velocity;
                        }
                    }
                } else {
                    // Straight borders
                    let is_horizontal =
                            corner_center_to_point.x <
                            corner_center_to_point.y;
                    if (is_horizontal) {
                        if (center_to_point.y < 0.0) {
                            dash_velocity = dv_t;
                            t = (point.x - r_tl) * dash_velocity;
                        } else {
                            dash_velocity = dv_b;
                            t = upto_bl - (point.x - r_bl) * dash_velocity;
                        }
                    } else {
                        if (center_to_point.x < 0.0) {
                            dash_velocity = dv_l;
                            t = upto_tl - (point.y - r_tl) * dash_velocity;
                        } else {
                            dash_velocity = dv_r;
                            t = upto_r + (point.y - r_tr) * dash_velocity;
                        }
                    }
                }
            }

            let dash_length = dash_length_per_width / dash_period_per_width;
            let desired_dash_gap = dash_gap_per_width / dash_period_per_width;

            // Straight borders should start and end with a dash, so max_t is
            // reduced to cause this.
            max_t -= select(0.0, dash_length, unrounded);
            if (max_t >= 1.0) {
                // Adjust dash gap to evenly divide max_t.
                let dash_count = floor(max_t);
                let dash_period = max_t / dash_count;
                border_color.a *= dash_alpha(
                    t,
                    dash_period,
                    dash_length,
                    dash_velocity,
                    antialias_threshold);
            } else if (unrounded) {
                // When there isn't enough space for the full gap between the
                // two start / end dashes of a straight border, reduce gap to
                // make them fit.
                let dash_gap = max_t - dash_length;
                if (dash_gap > 0.0) {
                    let dash_period = dash_length + dash_gap;
                    border_color.a *= dash_alpha(
                        t,
                        dash_period,
                        dash_length,
                        dash_velocity,
                        antialias_threshold);
                }
            }
        }

        // Composite the border over the background, clipped to the inner edge by its coverage,
        // instead of interpolating between the two in *straight* alpha. Interpolating drags the
        // result towards the background's colour as the coverage falls, and a border quad's
        // background is transparent black, so every antialiased inner edge came out dragged
        // towards black - a dark fringe just inside each border, and a dark inner arc at every
        // rounded corner. Scaling the border's alpha by the coverage keeps its colour and ramps
        // only how much of it there is, which is what an antialiased edge is.
        let blended_border = vec4<f32>(border_color.rgb,
            border_color.a * sdf_coverage(inner_sdf,
                quad_inner_sdf(point + 0.5 * pixel_x, half_size, corner_radius, reduced_border,
                    quad_own_sdf(point + 0.5 * pixel_x, half_size, corner_radius), inner_branch)
                    - quad_inner_sdf(point - 0.5 * pixel_x, half_size, corner_radius, reduced_border,
                        quad_own_sdf(point - 0.5 * pixel_x, half_size, corner_radius), inner_branch),
                quad_inner_sdf(point + 0.5 * pixel_y, half_size, corner_radius, reduced_border,
                    quad_own_sdf(point + 0.5 * pixel_y, half_size, corner_radius), inner_branch)
                    - quad_inner_sdf(point - 0.5 * pixel_y, half_size, corner_radius, reduced_border,
                        quad_own_sdf(point - 0.5 * pixel_y, half_size, corner_radius), inner_branch)));
        color = over(background_color, blended_border);
    }

    return blend_color(color, coverage);
}

// Returns the dash velocity of a corner given the dash velocity of the two
// sides, by returning the slower velocity (larger dashes).
//
// Since 0 is used for dash velocity when the border width is 0 (instead of
// +inf), this returns the other dash velocity in that case.
//
// An alternative to this might be to appropriately interpolate the dash
// velocity around the corner, but that seems overcomplicated.
fn corner_dash_velocity(dv1: f32, dv2: f32) -> f32 {
    if (dv1 == 0.0) {
        return dv2;
    } else if (dv2 == 0.0) {
        return dv1;
    } else {
        return min(dv1, dv2);
    }
}

// Returns alpha used to render antialiased dashes.
// `t` is within the dash when `fmod(t, period) < length`.
fn dash_alpha(t: f32, period: f32, length: f32, dash_velocity: f32, antialias_threshold: f32) -> f32 {
    let half_period = period / 2;
    let half_length = length / 2;
    // Value in [-half_period, half_period].
    // The dash is in [-half_length, half_length].
    let centered = fmod(t + half_period - half_length, period) - half_period;
    // Signed distance for the dash, negative values are inside the dash.
    let signed_distance = abs(centered) - half_length;
    // Antialiased alpha based on the signed distance.
    return saturate(antialias_threshold - signed_distance / dash_velocity);
}

// This approximates distance to the nearest point to a quarter ellipse in a way
// that is sufficient for anti-aliasing when the ellipse is not very eccentric.
// The components of `point` are expected to be positive.
//
// Negative on the outside and positive on the inside.
fn quarter_ellipse_sdf(point: vec2<f32>, radii: vec2<f32>) -> f32 {
    // Distance to the nearest point on the quarter ellipse, to first order: the implicit
    // value F = |p / r| - 1 divided by its gradient magnitude. Exact for circular corners
    // and far more accurate than an average-radius scaling for eccentric ones.
    let circle_vec = point / radii;
    let len = length(circle_vec);
    let gradient = circle_vec / max(radii, vec2<f32>(1e-5)) / max(len, 1e-5);
    return (1.0 - len) / max(length(gradient), 1e-5);
}

// Modulus that has the same sign as `a`.
fn fmod(a: f32, b: f32) -> f32 {
    return a - b * trunc(a / b);
}

// --- shadows --- //

struct Shadow {
    order: u32,
    blur_radius: f32,
    // The shadow rect for drop shadows; the "hole" rect for inset shadows.
    bounds: Bounds,
    corner_radii: Corners,
    content_mask: Bounds,
    content_mask_corner_radii: Corners,
    color: Hsla,
    // Only consulted when `inset == 1u`: the element's own bounds, used as a rounded-rect
    // clip so the shadow never escapes the element.
    element_bounds: Bounds,
    element_corner_radii: Corners,
    // 0 = drop shadow, 1 = inset shadow.
    inset: u32,
    pad: u32, // align to 8 bytes
}
@group(1) @binding(0) var<storage, read> b_shadows: array<Shadow>;

struct ShadowVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
    @location(1) @interpolate(flat) shadow_id: u32,
    //TODO: use `clip_distance` once Naga supports it
    @location(3) clip_distances: vec4<f32>,
}

@vertex
fn vs_shadow(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> ShadowVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    var shadow = b_shadows[instance_id];

    var geometry: Bounds;
    if (shadow.inset != 0u) {
        // The inset shadow is bounded by the element's outline, whose own coverage ramp
        // (`saturate(0.5 - element_distance)` below) needs fragments on both sides of it.
        geometry = shadow.element_bounds;
        geometry.origin -= vec2<f32>(AA_MARGIN);
        geometry.size += 2.0 * vec2<f32>(AA_MARGIN);
    } else {
        // Leave room for the gaussian tail outside the shadow rect, plus the coverage ramp.
        let margin = 3.0 * shadow.blur_radius + AA_MARGIN;
        geometry = shadow.bounds;
        geometry.origin -= vec2<f32>(margin);
        geometry.size += 2.0 * vec2<f32>(margin);
    }

    var out = ShadowVarying();
    out.position = to_device_position(unit_vertex, geometry);
    out.color = hsla_to_rgba(shadow.color);
    out.shadow_id = instance_id;
    out.clip_distances = distance_from_clip_rect(unit_vertex, geometry, shadow.content_mask);
    return out;
}

@fragment
fn fs_shadow(input: ShadowVarying) -> @location(0) vec4<f32> {
    // Alpha clip first, since we don't have `clip_distance`.
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    let shadow = b_shadows[input.shadow_id];
    let half_size = shadow.bounds.size / 2.0;
    let center = shadow.bounds.origin + half_size;
    let center_to_point = input.position.xy - center;

    let corner_radius = pick_corner_radius(center_to_point, shadow.corner_radii);
    let blur_radius = shadow.blur_radius;

    var alpha: f32;
    if (blur_radius == 0.0) {
        let distance = quad_sdf(input.position.xy, shadow.bounds, shadow.corner_radii);
        alpha = saturate(0.5 - distance);
    } else {
        // The signal is only non-zero in a limited range, so don't waste samples
        let low = center_to_point.y - half_size.y;
        let high = center_to_point.y + half_size.y;
        let start = clamp(-3.0 * blur_radius, low, high);
        let end = clamp(3.0 * blur_radius, low, high);

        // Accumulate samples (we can get away with surprisingly few samples)
        let step = (end - start) / 4.0;
        var y = start + step * 0.5;
        alpha = 0.0;
        for (var i = 0; i < 4; i += 1) {
            let blur = blur_along_x(center_to_point.x, center_to_point.y - y,
                blur_radius, corner_radius, half_size);
            alpha +=  blur * gaussian(y, blur_radius) * step;
            y += step;
        }
    }

    if (shadow.inset != 0u) {
        // The inset shadow is the complement of the (blurred) hole rect, clipped to the element.
        // `saturate(0.5 - d)` gives a 1-pixel antialiased edge: d <= -0.5 -> 1, d >= 0.5 -> 0.
        alpha = 1.0 - alpha;
        let element_distance = quad_sdf(input.position.xy, shadow.element_bounds,
                                        shadow.element_corner_radii);
        alpha *= saturate(0.5 - element_distance);
    }

    // Rounded content-mask cutout: `vs_shadow` clipped the mask's straight edges, so only its
    // rounded-corner arcs are cut here. Skipped when the mask coincides with the shadow's own
    // rounded cutout — the shadow SDF above already cut `bounds` (or `element_bounds` for
    // inset shadows).
    var own_origin = shadow.bounds.origin;
    var own_size = shadow.bounds.size;
    var own_radii = shadow.corner_radii;
    if (shadow.inset != 0u) {
        own_origin = shadow.element_bounds.origin;
        own_size = shadow.element_bounds.size;
        own_radii = shadow.element_corner_radii;
    }
    let mask_coverage = content_mask_coverage(input.position.xy, shadow.content_mask,
        shadow.content_mask_corner_radii, own_origin, own_size, own_radii, true);

    return blend_color(input.color, min(alpha, mask_coverage));
}

// --- path rasterization --- //

struct PathRasterizationVertex {
    xy_position: vec2<f32>,
    st_position: vec2<f32>,
    color: Background,
    bounds: Bounds,
}

@group(1) @binding(0) var<storage, read> b_path_vertices: array<PathRasterizationVertex>;

struct PathRasterizationVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) st_position: vec2<f32>,
    @location(1) @interpolate(flat) vertex_id: u32,
    //TODO: use `clip_distance` once Naga supports it
    @location(3) clip_distances: vec4<f32>,
    // The gradient ramp is prepared per vertex, as `vs_quad` does: running the same conversion in
    // the fragment shader cost a decode plus an Oklab conversion for every path pixel.
    @location(4) @interpolate(flat) background_solid: vec4<f32>,
    @location(5) @interpolate(flat) background_color0: vec4<f32>,
    @location(6) @interpolate(flat) background_color1: vec4<f32>,
}

@vertex
fn vs_path_rasterization(@builtin(vertex_index) vertex_id: u32) -> PathRasterizationVarying {
    let v = b_path_vertices[vertex_id];

    var out = PathRasterizationVarying();
    out.position = to_device_position_impl(v.xy_position);
    out.st_position = v.st_position;
    out.vertex_id = vertex_id;
    out.clip_distances = distance_from_clip_rect_impl(v.xy_position, v.bounds);

    let gradient = prepare_gradient_color(
        v.color.tag,
        v.color.color_space,
        v.color.solid,
        v.color.colors
    );
    out.background_solid = gradient.solid;
    out.background_color0 = gradient.color0;
    out.background_color1 = gradient.color1;
    return out;
}

@fragment
fn fs_path_rasterization(input: PathRasterizationVarying) -> @location(0) vec4<f32> {
    let dx = dpdx(input.st_position);
    let dy = dpdy(input.st_position);
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    let v = b_path_vertices[input.vertex_id];
    let background = v.color;
    let bounds = v.bounds;

    // The gradient direction's x components: used as a vector for the length test and again
    // per component below.
    let d_st_x = vec2<f32>(dx.x, dy.x);
    var alpha: f32;
    if (length(d_st_x) < 0.001) {
        // If the gradient is too small, return a solid color.
        alpha = 1.0;
    } else {
        let gradient = 2.0 * input.st_position.xx * d_st_x - vec2<f32>(dx.y, dy.y);
        let f = input.st_position.x * input.st_position.x - input.st_position.y;
        let distance = f / length(gradient);
        alpha = saturate(0.5 - distance);
    }
    let color = gradient_color(background, input.position.xy, bounds,
        input.background_solid, input.background_color0, input.background_color1);
    return vec4<f32>(color.rgb * color.a * alpha, color.a * alpha);
}

// --- paths --- //

struct PathSprite {
    bounds: Bounds,
}
@group(1) @binding(0) var<storage, read> b_path_sprites: array<PathSprite>;

struct PathVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) texture_coords: vec2<f32>,
}

@vertex
fn vs_path(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> PathVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let sprite = b_path_sprites[instance_id];
    // Don't apply content mask because it was already accounted for when rasterizing the path.
    let device_position = to_device_position(unit_vertex, sprite.bounds);
    // For screen-space intermediate texture, convert screen position to texture coordinates
    let screen_position = sprite.bounds.origin + unit_vertex * sprite.bounds.size;
    let texture_coords = screen_position / globals.viewport_size;

    var out = PathVarying();
    out.position = device_position;
    out.texture_coords = texture_coords;

    return out;
}

@fragment
fn fs_path(input: PathVarying) -> @location(0) vec4<f32> {
    let sample = textureSample(t_sprite, s_sprite, input.texture_coords);
    return sample;
}

// --- underlines --- //

struct Underline {
    order: u32,
    pad: u32,
    bounds: Bounds,
    content_mask: Bounds,
    content_mask_corner_radii: Corners,
    color: Hsla,
    thickness: f32,
    wavy: u32,
    transformation: TransformationMatrix,
}
@group(1) @binding(0) var<storage, read> b_underlines: array<Underline>;

struct UnderlineVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
    @location(1) @interpolate(flat) underline_id: u32,
    //TODO: use `clip_distance` once Naga supports it
    @location(3) clip_distances: vec4<f32>,
    // Position in the underline's untransformed (local) coordinate space, used by the
    // fragment shader for the wavy SDF so it isn't distorted by an ancestor transform.
    @location(2) local_position: vec2<f32>,
}

@vertex
fn vs_underline(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> UnderlineVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let underline = b_underlines[instance_id];

    var out = UnderlineVarying();
    // Grow the local rect so the coverage ramp below has fragments outside the outline (see
    // `AA_MARGIN`) - without them the rasterizer's pixel-centre rule would leave the underline's
    // ends hard regardless of sub-pixel position.
    let margin = aa_margin_local(underline.transformation.rotation_scale);
    let local_position = unit_vertex * (vec2<f32>(underline.bounds.size) + 2.0 * margin)
        + (underline.bounds.origin - margin);
    let transformed_position =
        transpose(underline.transformation.rotation_scale) * local_position + underline.transformation.translation;
    out.position = to_device_position_impl(transformed_position);
    out.color = hsla_to_rgba(underline.color);
    out.underline_id = instance_id;
    out.clip_distances = distance_from_clip_rect_aa_impl(transformed_position, underline.content_mask);
    out.local_position = local_position;
    return out;
}

// The wavy underline's stroke as a distance field of a point in the underline's local space: the
// distance across the stroke, signed, in local units.
fn wavy_underline_sdf(point: vec2<f32>, size: vec2<f32>, thickness: f32) -> f32 {
    const WAVE_FREQUENCY: f32 = 2.0;
    const WAVE_HEIGHT_RATIO: f32 = 0.8;
    let st = point / size.y - vec2<f32>(0.0, 0.5);
    let frequency = M_PI_F * WAVE_FREQUENCY * thickness / size.y;
    let amplitude = (thickness * WAVE_HEIGHT_RATIO) / size.y;
    let sine = sin(st.x * frequency) * amplitude;
    let dSine = cos(st.x * frequency) * amplitude * frequency;
    let distance_in_pixels = ((st.y - sine) / sqrt(1.0 + dSine * dSine)) * size.y;
    let half_thickness = thickness * 0.5;
    return max(-(distance_in_pixels + half_thickness), distance_in_pixels - half_thickness);
}

@fragment
fn fs_underline(input: UnderlineVarying) -> @location(0) vec4<f32> {
    const WAVE_FREQUENCY: f32 = 2.0;
    const WAVE_HEIGHT_RATIO: f32 = 0.8;

    // Alpha clip first, since we don't have `clip_distance`.
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    let underline = b_underlines[input.underline_id];

    // One screen pixel in the underline's local space, and the underline's own rectangle. The
    // outline is unrounded, so this is the plain box distance; it is what keeps the geometry
    // margin from painting past the underline's ends.
    let pixel_x = dpdx(input.local_position);
    let pixel_y = dpdy(input.local_position);
    let half_size = underline.bounds.size / 2.0;
    let point = input.local_position - underline.bounds.origin;
    let own_coverage = sdf_coverage(quad_own_sdf(point, half_size, 0.0),
        quad_own_sdf(point + 0.5 * pixel_x, half_size, 0.0)
            - quad_own_sdf(point - 0.5 * pixel_x, half_size, 0.0),
        quad_own_sdf(point + 0.5 * pixel_y, half_size, 0.0)
            - quad_own_sdf(point - 0.5 * pixel_y, half_size, 0.0));

    // The mask's whole outline is cut here (`clip_distances` is padded by `AA_MARGIN` and only
    // culls). An underline carries no rounded cutout of its own, so there is no mask-equals-own
    // skip.
    let mask_coverage = content_mask_coverage(
        transpose(underline.transformation.rotation_scale) * input.local_position
            + underline.transformation.translation,
        underline.content_mask, underline.content_mask_corner_radii,
        underline.bounds.origin, underline.bounds.size, underline.content_mask_corner_radii,
        false);
    let coverage = min(own_coverage, mask_coverage);

    if (underline.wavy == 0u)
    {
        return blend_color(input.color, coverage);
    }

    let thickness = underline.thickness;
    let alpha = sdf_coverage(
        wavy_underline_sdf(point, underline.bounds.size, thickness),
        wavy_underline_sdf(point + 0.5 * pixel_x, underline.bounds.size, thickness)
            - wavy_underline_sdf(point - 0.5 * pixel_x, underline.bounds.size, thickness),
        wavy_underline_sdf(point + 0.5 * pixel_y, underline.bounds.size, thickness)
            - wavy_underline_sdf(point - 0.5 * pixel_y, underline.bounds.size, thickness));
    return blend_color(input.color, min(alpha, coverage));
}

// --- monochrome sprites --- //

struct MonochromeSprite {
    order: u32,
    pad: u32,
    bounds: Bounds,
    content_mask: Bounds,
    content_mask_corner_radii: Corners,
    color: Hsla,
    tile: AtlasTile,
    transformation: TransformationMatrix,
}
@group(1) @binding(0) var<storage, read> b_mono_sprites: array<MonochromeSprite>;

struct MonoSpriteVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) tile_position: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
    @location(3) clip_distances: vec4<f32>,
    // Position in the sprite's untransformed (local) coordinate space, used by the fragment
    // shader for the content-mask coverage so it isn't distorted by the transform.
    @location(2) local_position: vec2<f32>,
    @location(4) @interpolate(flat) sprite_id: u32,
}

@vertex
fn vs_mono_sprite(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> MonoSpriteVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let sprite = b_mono_sprites[instance_id];

    var out = MonoSpriteVarying();
    // Deliberately NOT grown by `AA_MARGIN`, unlike every other primitive here. A glyph's shape is
    // its texture's alpha, not its quad's outline, and this shader has no own-coverage term to
    // taper the extra geometry with: the only coverage it applies is the *content mask's*, which is
    // 1.0 across the whole interior. Growing the quad would therefore paint one more pixel of the
    // atlas tile's edge texel at full alpha all the way around every glyph, which reads as text
    // that is both bolder and blurrier.
    let local_position = unit_vertex * vec2<f32>(sprite.bounds.size) + sprite.bounds.origin;
    let transformed_position =
        transpose(sprite.transformation.rotation_scale) * local_position + sprite.transformation.translation;
    out.position = to_device_position_impl(transformed_position);

    out.tile_position = to_tile_position(unit_vertex, sprite.tile);
    out.color = hsla_to_rgba(sprite.color);
    out.sprite_id = instance_id;
    out.local_position = local_position;
    out.clip_distances = distance_from_clip_rect_aa_impl(transformed_position, sprite.content_mask);
    return out;
}

@fragment
fn fs_mono_sprite(input: MonoSpriteVarying) -> @location(0) vec4<f32> {
    let sample = textureSample(t_sprite, s_sprite, input.tile_position).r;
    let alpha_corrected = apply_contrast_and_gamma_correction(sample, input.color.rgb, gamma_params.grayscale_enhanced_contrast, gamma_params.gamma_ratios);

    // Alpha clip after using the derivatives.
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    let sprite = b_mono_sprites[input.sprite_id];
    // Rounded content-mask cutout: `vs_mono_sprite` clipped the mask's straight edges, so only
    // its rounded-corner arcs are cut here. Monochrome sprites carry no own rounded cutout in
    // the shader, so there is no mask-equals-own skip.
    let mask_coverage = content_mask_coverage(
        transpose(sprite.transformation.rotation_scale) * input.local_position
            + sprite.transformation.translation,
        sprite.content_mask, sprite.content_mask_corner_radii,
        sprite.bounds.origin, sprite.bounds.size, sprite.content_mask_corner_radii,
        false);

    return blend_color(input.color, alpha_corrected * mask_coverage);
}

// --- polychrome sprites --- //

struct PolychromeSprite {
    order: u32,
    pad: u32,
    grayscale: u32,
    opacity: f32,
    bounds: Bounds,
    content_mask: Bounds,
    content_mask_corner_radii: Corners,
    corner_radii: Corners,
    tile: AtlasTile,
    transformation: TransformationMatrix,
}
@group(1) @binding(0) var<storage, read> b_poly_sprites: array<PolychromeSprite>;

struct PolySpriteVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) tile_position: vec2<f32>,
    @location(1) @interpolate(flat) sprite_id: u32,
    @location(3) clip_distances: vec4<f32>,
    // Position in the sprite's untransformed (local) coordinate space, used by the
    // fragment shader for the corner-radius SDF so it isn't distorted by the transform.
    @location(2) local_position: vec2<f32>,
}

@vertex
fn vs_poly_sprite(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> PolySpriteVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let sprite = b_poly_sprites[instance_id];

    var out = PolySpriteVarying();
    // Grow the sprite rect so its own corner-radius SDF and the mask's ramp have fragments
    // outside the outline (see `AA_MARGIN`). The atlas lookup stays clamped inside the sprite's
    // tile, so the added fragments - whose coverage is at most a fraction of a pixel - replay
    // the tile's edge texel instead of a neighbouring tile's.
    let margin = aa_margin_local(sprite.transformation.rotation_scale);
    let local_position = unit_vertex * (vec2<f32>(sprite.bounds.size) + 2.0 * margin)
        + (sprite.bounds.origin - margin);
    let transformed_position =
        transpose(sprite.transformation.rotation_scale) * local_position + sprite.transformation.translation;
    out.position = to_device_position_impl(transformed_position);
    out.tile_position = to_tile_position(saturate(unit_vertex), sprite.tile);
    out.sprite_id = instance_id;
    // A mask that coincides with the sprite's own bounds is the sprite's own cutout (see
    // `fs_poly_sprite`): the straight-edge clip would slice the transformed sprite at its
    // untransformed rect, so it is skipped — the sprite's local-space outline already bounds
    // the painted area.
    let mask_is_own = all(sprite.content_mask.origin == sprite.bounds.origin) &&
        all(sprite.content_mask.size == sprite.bounds.size);
    out.clip_distances = select(
        distance_from_clip_rect_aa_impl(transformed_position, sprite.content_mask),
        vec4<f32>(1.0),
        mask_is_own);
    out.local_position = local_position;
    return out;
}

@fragment
fn fs_poly_sprite(input: PolySpriteVarying) -> @location(0) vec4<f32> {
    let sample = textureSample(t_sprite, s_sprite, input.tile_position);
    // Alpha clip after using the derivatives.
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    let sprite = b_poly_sprites[input.sprite_id];
    let pixel_x = dpdx(input.local_position);
    let pixel_y = dpdy(input.local_position);
    let half_size = sprite.bounds.size / 2.0;
    let point = input.local_position - sprite.bounds.origin;
    let corner_radius = pick_corner_radius(point - half_size, sprite.corner_radii);

    // Rounded content-mask cutout: the vertex shader clipped the mask's straight edges, so only
    // its rounded-corner arcs are cut here. A mask that coincides with the sprite's own
    // *untransformed* bounds and radii is the sprite's own antialiased rounded cutout — the SDF
    // above cuts that outline in local space (following any transform), so the mask must not cut
    // it again in scene space.
    let mask_coverage = content_mask_coverage(
        transpose(sprite.transformation.rotation_scale) * input.local_position
            + sprite.transformation.translation,
        sprite.content_mask, sprite.content_mask_corner_radii,
        sprite.bounds.origin, sprite.bounds.size,
        sprite.corner_radii,
        true);

    var color = sample;
    if (sprite.grayscale != 0u) {
        let grayscale = dot(color.rgb, GRAYSCALE_FACTORS);
        color = vec4<f32>(vec3<f32>(grayscale), sample.a);
    }
    return blend_color(color, sprite.opacity * min(sdf_coverage(quad_own_sdf(point, half_size, corner_radius),
        quad_own_sdf(point + 0.5 * pixel_x, half_size, corner_radius)
            - quad_own_sdf(point - 0.5 * pixel_x, half_size, corner_radius),
        quad_own_sdf(point + 0.5 * pixel_y, half_size, corner_radius)
            - quad_own_sdf(point - 0.5 * pixel_y, half_size, corner_radius)), mask_coverage));
}

// --- surfaces --- //

struct SurfaceParams {
    bounds: Bounds,
    content_mask: Bounds,
}

@group(1) @binding(0) var<uniform> surface_locals: SurfaceParams;
@group(1) @binding(1) var t_surface: texture_2d<f32>;
@group(1) @binding(2) var s_surface: sampler;

struct SurfaceVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) texture_position: vec2<f32>,
    @location(3) clip_distances: vec4<f32>,
}

@vertex
fn vs_surface(@builtin(vertex_index) vertex_id: u32) -> SurfaceVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));

    var out = SurfaceVarying();
    out.position = to_device_position(unit_vertex, surface_locals.bounds);
    out.texture_position = unit_vertex;
    // Exactly, not loosened: this fragment samples a texture and nothing else, so the clip is
    // the only thing that cuts it (see `distance_from_clip_rect_impl`).
    out.clip_distances = distance_from_clip_rect_impl(
        unit_vertex * vec2<f32>(surface_locals.bounds.size) + surface_locals.bounds.origin,
        surface_locals.content_mask);
    return out;
}

@fragment
fn fs_surface(input: SurfaceVarying) -> @location(0) vec4<f32> {
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    return textureSampleLevel(t_surface, s_surface, input.texture_position, 0.0);
}

// --- blur --- //
//
// Backdrop and content filters share these passes:
//   1. `fs_blur_downsample` copies a source texture into the half-resolution blur texture
//      (also reused to blit the offscreen scene into the swapchain).
//   2. `fs_blur` runs one axis of a separable gaussian; the host invokes it twice.
//   3. `fs_blur_composite` samples the blurred texture and composites it into a rounded
//      rectangle, clipped and modulated by opacity.
//
// Notes (review #5, #7): the scene/blur textures use the swapchain's (typically non-sRGB)
// format, so the gaussian runs on gamma-encoded values rather than linear light — consistent
// with the rest of gpui's compositing and close to what browsers do; bright detail darkens
// slightly.
//
// A content-filter group's texture is transparent outside the painted subtree, so taps that left
// the element's box would read that surround and dissolve the element's border into whatever is
// behind it (a soft edge ring, ~3σ wide). The downsample reflects those taps back inside the box
// instead — see `mirror_into_rect`.

// Reflect a sample back into `rect`. Taps that leave an element's box mirror its edge content
// rather than reading the transparent surround, which keeps a blurred element's border crisp,
// opaque and even — CSS `backdrop-filter`'s edge behaviour (Chrome mirrors since 129; `duplicate`
// smears the edge line instead), and what `filter: blur` has to do to look like it in practice.
//
// The fold uses `floor` rather than `%`/`fract`: `mod`/`fmod` disagree about the sign of negative
// inputs across WGSL, HLSL and MSL, and `floor` does not.
fn mirror_into_rect(p: vec2<f32>, rect: Bounds) -> vec2<f32> {
    if (rect.size.x <= 0.0 || rect.size.y <= 0.0) {
        return p;
    }
    let t = (p - rect.origin) / rect.size;
    // Triangle wave of period 2, folded into [0, 1]: 0.25 -> 0.25, 1.75 -> 0.25, -0.25 -> 0.25.
    let m = t - 2.0 * floor(t * 0.5);
    return rect.origin + (1.0 - abs(m - 1.0)) * rect.size;
}

struct BlurParams {
    // The composite quad (composite pass), or the element's box the source taps are mirrored back
    // into (downsample pass), in device pixels.
    bounds: Bounds,
    content_mask: Bounds,
    // The content mask's corner radii (tl, tr, br, bl), in device pixels: the clip an element is
    // painted under may itself be rounded (`overflow_hidden` + a corner radius), and a filter's
    // output is clipped by it like any other painting — corners included.
    content_mask_radii: vec4<f32>,
    corner_radii: vec4<f32>,
    direction: vec2<f32>,
    sigma: f32,
    opacity: f32,
    tap_count: f32,
    // Spacing between taps, in pixels. >1 when the radius is so large the kernel would need more
    // than `tap_count` taps to span ±3σ — the taps spread out instead of truncating the gaussian.
    tap_step: f32,
    // 1.0 = clip the composite to the rounded rect (backdrop); 0.0 = let the blurred result fade
    // out on its own (content `filter` bleeds past the element box like CSS).
    clip_rounded: f32,
    // 1.0 = snapped 2:1 box downsample (anchor the half-res grid to a fixed 2px grid at the origin
    // so a stationary element blurs identically at every window size); 0.0 = 1:1 copy (scene blit).
    downsample: f32,
}

@group(1) @binding(0) var<uniform> blur_locals: BlurParams;
@group(1) @binding(1) var t_blur: texture_2d<f32>;
@group(1) @binding(2) var s_blur: sampler;

struct BlurVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(3) clip_distances: vec4<f32>,
}

@vertex
fn vs_blur_fullscreen(@builtin(vertex_index) vertex_id: u32) -> BlurVarying {
    // A single triangle large enough to cover the whole framebuffer.
    let uv = vec2<f32>(f32((vertex_id << 1u) & 2u), f32(vertex_id & 2u));
    var out = BlurVarying();
    out.uv = uv;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.clip_distances = vec4<f32>(1.0);
    return out;
}

@fragment
fn fs_blur_downsample(input: BlurVarying) -> @location(0) vec4<f32> {
    if (blur_locals.downsample > 0.5) {
        // Snapped 2:1 box downsample. Half-res texel `px` samples source at full-res coordinate
        // 2*px + 1 (the boundary between source texels 2*px and 2*px+1), so one bilinear tap
        // averages exactly that pair. The grid is anchored to the origin and independent of the
        // viewport size, so an element at fixed pixels blurs identically at every window size —
        // otherwise the implicit `floor(W/2)` grid stretches and the halo wobbles by ~1px on resize.
        let dst = floor(input.position.xy);
        // The taps that would leave the element's box read its mirrored edge instead of the
        // transparent surround (`blur_locals.bounds` in this pass is that box, not the composite
        // rect).
        let src_px = mirror_into_rect(dst * 2.0 + 1.0, blur_locals.bounds);
        let src_uv = src_px / globals.viewport_size;
        return textureSampleLevel(t_blur, s_blur, src_uv, 0.0);
    }
    // 1:1 copy at matching resolution (used to blit the offscreen scene into the swapchain).
    return textureSampleLevel(t_blur, s_blur, input.uv, 0.0);
}

@fragment
fn fs_blur(input: BlurVarying) -> @location(0) vec4<f32> {
    let sigma = blur_locals.sigma;
    let taps = i32(blur_locals.tap_count);
    let step = blur_locals.tap_step;
    var color = vec4<f32>(0.0);
    var weight_sum = 0.0;
    for (var i = -taps; i <= taps; i = i + 1) {
        let offset = f32(i) * step;
        let weight = gaussian(offset, sigma);
        let uv = input.uv + blur_locals.direction * offset;
        color += textureSampleLevel(t_blur, s_blur, uv, 0.0) * weight;
        weight_sum += weight;
    }
    return color / max(weight_sum, 1e-5);
}

@vertex
fn vs_blur_composite(@builtin(vertex_index) vertex_id: u32) -> BlurVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    var out = BlurVarying();
    // Grow the composite quad so the rounded cut in the fragment has fragments on both sides of
    // the outline (see `AA_MARGIN`). The fragment samples the blur by *screen position*, so the
    // extra area needs no uv compensation, and `blur_locals.bounds` itself is left alone — it is
    // what the cut measures from.
    let bounds = Bounds(
        blur_locals.bounds.origin - vec2<f32>(AA_MARGIN),
        blur_locals.bounds.size + 2.0 * vec2<f32>(AA_MARGIN));
    out.position = to_device_position(unit_vertex, bounds);
    out.uv = unit_vertex;
    out.clip_distances = distance_from_clip_rect(unit_vertex, bounds, blur_locals.content_mask);
    return out;
}

@fragment
fn fs_blur_composite(input: BlurVarying) -> @location(0) vec4<f32> {
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    // Sample the half-res blur by screen position, using the SAME fixed 2:1 grid the snapped
    // downsample wrote (anchored at the origin, independent of viewport parity). `2*floor(W/2)` is
    // the source span the half-res texture covers; dividing by it maps screen pixel p to half-res
    // texel p/2 at every window size, so the composite stays put rather than wobbling on resize.
    let blur_span = 2.0 * floor(globals.viewport_size * 0.5);
    let uv = input.position.xy / blur_span;
    let blurred = textureSampleLevel(t_blur, s_blur, uv, 0.0);

    let corner_radii = Corners(
        blur_locals.corner_radii.x,
        blur_locals.corner_radii.y,
        blur_locals.corner_radii.z,
        blur_locals.corner_radii.w,
    );
    // Backdrop blur clips to the rounded rect (the frosted panel has a defined shape). Content
    // blur does not — it bleeds past the element box like CSS `filter: blur`, so the soft fade
    // isn't sharply truncated at the edge; its shape comes from the blurred group's own alpha.
    let distance = quad_sdf(input.position.xy, blur_locals.bounds, corner_radii);
    let coverage = select(1.0, saturate(0.5 - distance), blur_locals.clip_rounded > 0.5);
    // The clip the element was painted under is part of its shape too: a rounded `overflow_hidden`
    // ancestor clips the filter's output like anything else it contains, corners included.
    let mask_radii = Corners(
        blur_locals.content_mask_radii.x,
        blur_locals.content_mask_radii.y,
        blur_locals.content_mask_radii.z,
        blur_locals.content_mask_radii.w,
    );
    let mask_coverage = saturate(
        0.5 - quad_sdf(input.position.xy, blur_locals.content_mask, mask_radii),
    );

    // The blurred sample is premultiplied (blurring against the transparent, rgb=0 surround scales
    // rgb with the fading alpha), so output premultiplied and let the pipeline blend premultiplied.
    // A backdrop's scene is opaque (alpha ~= 1) so this replaces; a content-filter group is
    // transparent outside its subtree, so the target shows through there instead of darkening.
    let c = min(coverage, mask_coverage) * blur_locals.opacity;
    return vec4<f32>(blurred.rgb * c, blurred.a * c);
}
