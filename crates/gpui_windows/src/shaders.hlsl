#include "alpha_correction.hlsl"

cbuffer GlobalParams: register(b0) {
    float4 gamma_ratios;
    float2 global_viewport_size;
    float grayscale_enhanced_contrast;
    float subpixel_enhanced_contrast;
    uint is_bgr;
    uint3 global_pad;
};

Texture2D<float4> t_sprite: register(t0);
SamplerState s_sprite: register(s0);

struct SubpixelSpriteFragmentOutput {
    float4 foreground : SV_Target0;
    float4 alpha : SV_Target1;
};

struct Bounds {
    float2 origin;
    float2 size;
};

struct Corners {
    float top_left;
    float top_right;
    float bottom_right;
    float bottom_left;
};

struct Edges {
    float top;
    float right;
    float bottom;
    float left;
};

struct Hsla {
    float h;
    float s;
    float l;
    float a;
};

struct LinearColorStop {
    Hsla color;
    float percentage;
};

struct Background {
    // 0u is Solid
    // 1u is LinearGradient
    // 2u is PatternSlash
    // 3u is Checkerboard
    // 4u is RadialGradient
    // 5u is RadialGradientCircle
    uint tag;
    // 0u is sRGB linear color
    // 1u is Oklab color
    uint color_space;
    Hsla solid;
    float gradient_angle_or_pattern_height;
    LinearColorStop colors[2];
    // Radial gradients store the center's vertical fraction of the element's size here; padding
    // for alignment otherwise.
    float radial_center_y;
    // The ending-shape size keyword of a radial gradient (0 = farthest-corner, 1 = closest-side,
    // 2 = farthest-side, 3 = closest-corner); unused by other backgrounds.
    uint radial_size;
    // Keeps the struct 8-byte sized, matching the Rust layout the other backends mirror.
    uint pad;
};

struct GradientColor {
  float4 solid;
  float4 color0;
  float4 color1;
};

struct AtlasTextureId {
    uint index;
    uint kind;
};

struct AtlasBounds {
    int2 origin;
    int2 size;
};

struct AtlasTile {
    AtlasTextureId texture_id;
    uint tile_id;
    uint padding;
    AtlasBounds bounds;
};

struct TransformationMatrix {
    float2x2 rotation_scale;
    float2 translation;
};

static const float M_PI_F = 3.141592653f;
static const float3 GRAYSCALE_FACTORS = float3(0.2126f, 0.7152f, 0.0722f);

// How far outside its own outline a primitive is still rasterized, in device pixels. The
// rasterizer only emits fragments for pixels whose *centre* is inside the primitive, so a
// shader that shapes its own edge from a signed distance field — as every shader here does —
// can otherwise only ever *shrink* coverage: an edge landing in the far half of a pixel has
// no fragment to taper, and its partial pixel is lost outright (half a pixel of coverage on
// average, always in the same direction, which also shifts the edge's apparent position onto
// the pixel grid). Expanding the geometry by this much gives the distance ramp a fragment in
// every pixel it can reach; the ramp then drives the coverage to zero on its own well inside
// the margin, so nothing is actually painted outside the shape.
static const float AA_MARGIN = 1.0;

// Per-axis expansion, in a primitive's local units, that widens its local rect by
// `AA_MARGIN` device pixels on every side. `M` maps local units to device pixels with row 0
// the image of the local x axis, so a unit step along local x spans `length(M[0])` pixels and
// the local x extent scales by `length(M[0])`; the local rect therefore has to grow by
// `AA_MARGIN * length(M[1]) / |det M|` along x for the edges that run along y to move
// `AA_MARGIN` pixels outward (a y-side edge is spanned by the local x axis, so shifting it
// costs `length(M[0])` pixels per local unit and the perpendicular displacement is that,
// times the sine of the angle between the axes, i.e. `|det M| / length(M[1])`).
float2 aa_margin_local(float2x2 M) {
    float2 axis = float2(length(M[0]), length(M[1]));
    float det = abs(M[0][0] * M[1][1] - M[0][1] * M[1][0]);
    return AA_MARGIN * axis.yx / max(det, 1e-3);
}

float4 to_device_position_impl(float2 position) {
    float2 device_position = position / global_viewport_size * float2(2.0, -2.0) + float2(-1.0, 1.0);
    return float4(device_position, 0., 1.);
}

float4 to_device_position(float2 unit_vertex, Bounds bounds) {
    float2 position = unit_vertex * bounds.size + bounds.origin;
    return to_device_position_impl(position);
}

// The exact clip: the fragment is discarded at the mask's edge. This is what a primitive that
// has no cut of its own must use — the path rasterizer draws through this and nothing else, so
// padding it here would let every path and SVG leak a pixel out of every clip.
float4 distance_from_clip_rect_impl(float2 position, Bounds clip_bounds) {
    float2 tl = position - clip_bounds.origin;
    float2 br = clip_bounds.origin + clip_bounds.size - position;
    return float4(tl.x, br.x, tl.y, br.y);
}

// The same, a margin looser. `content_mask_coverage` cuts the mask's *whole* outline again in
// the fragment — with a one-pixel ramp, so the edge follows the arc instead of the pixel grid —
// and this clip is then only the coarse cull that keeps fragments far outside from being shaded
// at all. Only the primitives that apply that cut may use this, or the margin becomes a hole.
float4 distance_from_clip_rect_aa_impl(float2 position, Bounds clip_bounds) {
    return distance_from_clip_rect_impl(position, clip_bounds) + AA_MARGIN;
}

float4 distance_from_clip_rect(float2 unit_vertex, Bounds bounds, Bounds clip_bounds) {
    float2 position = unit_vertex * bounds.size + bounds.origin;
    return distance_from_clip_rect_aa_impl(position, clip_bounds);
}

// Coverage of a distance field over one device pixel, from the field sampled at the four
// quarter points of that pixel: `s00` is (-1/4, -1/4) of a pixel from its centre, `s01`
// (-1/4, +1/4), `s10` (+1/4, -1/4) and `s11` (+1/4, +1/4).
//
// Dividing a distance by the length of the field's *screen-space* gradient turns it into pixels
// measured from the pixel centre, and a straight edge then integrates to exactly
// `saturate(0.5 - pixels)`. The gradient has to be the field's screen-space gradient for that
// to hold under a transform, so it is read off these samples — each pair sits half a pixel
// apart, so twice their difference is the change over one pixel. (A one-sided difference over a
// whole pixel, which is what `ddx(field)` gives, reads the slope tens of percent low at a corner
// of a pixel or two radius and the widened ramp then swallows the arc; the same happens to the
// average of the two axes' step lengths, which is blind to direction.)
//
// The field's value at the pixel centre is the average of the four samples (exactly, for the
// linear fields a straight edge has). Averaging the four *coverages* instead would be wrong:
// each sample would be asking about a box a quarter of a pixel off centre, so a pixel that the
// shape fully covers would come out at 0.875 from the samples that sit nearest its edge.
float sdf_coverage(float s00, float s01, float s10, float s11) {
    float2 per_pixel = float2((s10 + s11) - (s00 + s01), (s01 + s11) - (s00 + s10));
    float scale = max(length(per_pixel), 1e-6);
    float centre = 0.25 * (s00 + s01 + s10 + s11);
    return saturate(0.5 - centre / scale);
}

// Encode a linear RGB color as non-linear (gamma-encoded) sRGB — the IEC 61966-2-1 transfer
// function, spelled the same way as in `shaders.wgsl` and `shaders.metal` so gradients render
// identically on every backend. A plain `pow(color, 2.2)` would be cheaper but drifts ~2% from
// the standard in the mid-tones, which is visible when the same gradient is compared across
// backends.
float3 linear_to_srgb(float3 color) {
    return color < 0.0031308
        ? color * 12.92
        : 1.055 * pow(color, 1.0 / 2.4) - 0.055;
}

// Convert a non-linear (gamma-encoded) sRGB color to linear RGB, IEC 61966-2-1.
float3 srgb_to_linear(float3 color) {
    return color < 0.04045
        ? color / 12.92
        : pow((color + 0.055) / 1.055, 2.4);
}

/// Hsla to linear RGBA conversion.
float4 hsla_to_rgba(Hsla hsla) {
    float h = hsla.h * 6.0; // Now, it's an angle but scaled in [0, 6) range
    float s = hsla.s;
    float l = hsla.l;
    float a = hsla.a;

    float c = (1.0 - abs(2.0 * l - 1.0)) * s;
    float x = c * (1.0 - abs(fmod(h, 2.0) - 1.0));
    float m = l - c / 2.0;

    float r = 0.0;
    float g = 0.0;
    float b = 0.0;

    if (h >= 0.0 && h < 1.0) {
        r = c;
        g = x;
    } else if (h >= 1.0 && h < 2.0) {
        r = x;
        g = c;
    } else if (h >= 2.0 && h < 3.0) {
        g = c;
        b = x;
    } else if (h >= 3.0 && h < 4.0) {
        g = x;
        b = c;
    } else if (h >= 4.0 && h < 5.0) {
        r = x;
        b = c;
    } else {
        r = c;
        b = x;
    }

    float4 rgba;
    rgba.x = (r + m);
    rgba.y = (g + m);
    rgba.z = (b + m);
    rgba.w = a;
    return rgba;
}

// Converts a sRGB color to the Oklab color space.
// Reference: https://bottosson.github.io/posts/oklab/#converting-from-linear-srgb-to-oklab
float4 srgb_to_oklab(float4 color) {
    // Convert non-linear sRGB to linear sRGB
    color = float4(srgb_to_linear(color.rgb), color.a);

    float l = 0.4122214708 * color.r + 0.5363325363 * color.g + 0.0514459929 * color.b;
    float m = 0.2119034982 * color.r + 0.6806995451 * color.g + 0.1073969566 * color.b;
    float s = 0.0883024619 * color.r + 0.2817188376 * color.g + 0.6299787005 * color.b;

    float l_ = pow(l, 1.0/3.0);
    float m_ = pow(m, 1.0/3.0);
    float s_ = pow(s, 1.0/3.0);

    return float4(
        0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
        1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
        0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
        color.a
    );
}

// Converts an Oklab color to the sRGB color space.
float4 oklab_to_srgb(float4 color) {
    float l_ = color.r + 0.3963377774 * color.g + 0.2158037573 * color.b;
    float m_ = color.r - 0.1055613458 * color.g - 0.0638541728 * color.b;
    float s_ = color.r - 0.0894841775 * color.g - 1.2914855480 * color.b;

    float l = l_ * l_ * l_;
    float m = m_ * m_ * m_;
    float s = s_ * s_ * s_;

    float3 linear_rgb = float3(
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s
    );

    // Convert linear sRGB to non-linear sRGB
    return float4(linear_to_srgb(linear_rgb), color.a);
}

// This approximates the error function, needed for the gaussian integral
float2 erf(float2 x) {
    float2 s = sign(x);
    float2 a = abs(x);
    x = 1. + (0.278393 + (0.230389 + 0.078108 * (a * a)) * a) * a;
    float2 x2 = x * x;
    return s - s / (x2 * x2);
}

float blur_along_x(float x, float y, float sigma, float corner, float2 half_size) {
    float delta = min(half_size.y - corner - abs(y), 0.);
    float curved = half_size.x - corner + sqrt(max(0., corner * corner - delta * delta));
    float2 integral = 0.5 + 0.5 * erf((x + float2(-curved, curved)) * (sqrt(0.5) / sigma));
    return integral.y - integral.x;
}

// A standard gaussian function, used for weighting samples
float gaussian(float x, float sigma) {
    return exp(-(x * x) / (2. * sigma * sigma)) / (sqrt(2. * M_PI_F) * sigma);
}

float4 over(float4 below, float4 above) {
    float4 result;
    float alpha = above.a + below.a * (1.0 - above.a);
    result.rgb = (above.rgb * above.a + below.rgb * below.a * (1.0 - above.a)) / alpha;
    result.a = alpha;
    return result;
}

float2 to_tile_position(float2 unit_vertex, AtlasTile tile) {
    float2 atlas_size;
    t_sprite.GetDimensions(atlas_size.x, atlas_size.y);
    return (float2(tile.bounds.origin) + unit_vertex * float2(tile.bounds.size)) / atlas_size;
}

// Selects corner radius based on quadrant.
float pick_corner_radius(float2 center_to_point, Corners corner_radii) {
    if (center_to_point.x < 0.) {
        if (center_to_point.y < 0.) {
            return corner_radii.top_left;
        } else {
            return corner_radii.bottom_left;
        }
    } else {
        if (center_to_point.y < 0.) {
            return corner_radii.top_right;
        } else {
            return corner_radii.bottom_right;
        }
    }
}

// Implementation of quad signed distance field
float quad_sdf_impl(float2 corner_center_to_point, float corner_radius) {
    if (corner_radius == 0.0) {
        // Fast path for unrounded corners
        return max(corner_center_to_point.x, corner_center_to_point.y);
    } else {
        // Signed distance of the point from a quad that is inset by corner_radius
        // It is negative inside this quad, and positive outside
        float signed_distance_to_inset_quad =
            // 0 inside the inset quad, and positive outside
            length(max(float2(0.0, 0.0), corner_center_to_point)) +
            // 0 outside the inset quad, and negative inside
            min(0.0, max(corner_center_to_point.x, corner_center_to_point.y));

        return signed_distance_to_inset_quad - corner_radius;
    }
}

// The quad's own outline as a distance field of a point in its local space. The corner radius
// is supplied rather than picked here so that the samples `quad_fragment` takes around the
// pixel centre all describe one corner's field — `pick_corner_radius` switches quadrant at the
// centre lines, where the radii of neighbouring corners may differ.
float quad_own_sdf(float2 the_point, float2 half_size, float corner_radius) {
    float2 corner_to_point = abs(the_point - half_size) - half_size;
    return quad_sdf_impl(corner_to_point + corner_radius, corner_radius);
}

// The quad's own coverage, from `quad_own_sdf` sampled at the four quarter points of the pixel
// (see `sdf_coverage`). `step_x`/`step_y` are a quarter of a pixel in the quad's local space.
float quad_own_coverage(float2 the_point, float2 step_x, float2 step_y, float2 half_size,
                        float corner_radius) {
    return sdf_coverage(
        quad_own_sdf(the_point - step_x - step_y, half_size, corner_radius),
        quad_own_sdf(the_point - step_x + step_y, half_size, corner_radius),
        quad_own_sdf(the_point + step_x - step_y, half_size, corner_radius),
        quad_own_sdf(the_point + step_x + step_y, half_size, corner_radius));
}

// ---------------------------------------------------------------------------------------------
// Exact area coverage for a rounded corner.
//
// The one-pixel ramp above is exact for a straight edge but only first order where the boundary
// curves, and a corner's arc is exactly that: measured against a 32x32 supersampled reference it
// is off by up to 0.06 of a pixel (15 levels of 255) — visible as a corner that is a shade too
// full, which is the "not as smooth as CSS" complaint. Skia's CPU rasterizer resolves curves by
// area rather than by a ramp (`SkScan_AAAPath.cpp`: "we analytically compute the coverage of this
// horizontal strip ... ground-truth coverages"), and the one curve here has a closed form.
//
// A pixel's coverage is the area of the pixel inside the shape, and the shape near a corner is
// the rectangle minus the notch the arc cuts out of the corner's `radius x radius` square. The
// rectangle's own coverage is exact per axis (a half-plane over a pixel is a ramp). The notch's
// area is that square clipped to the pixel, minus the area of the same box inside the disc —
// which is the circle's antiderivative, with the box split at the disc's axes because the area of
// the disc is even in both.

// `∫₀^x sqrt(r² - t²) dt`: the area under the circle's upper half out to x.
float circle_area_to(float x, float r) {
    x = clamp(x, 0.0, r);
    float half_width = sqrt(max(r * r - x * x, 0.0));
    // `max(r, ...)`: a zero radius must not turn the division into a NaN, and its term is zero
    // anyway.
    return 0.5 * (x * half_width + r * r * asin(clamp(x / max(r, 1e-6), -1.0, 1.0)));
}

// The area of `[0, x] x [0, y]` inside the disc of radius r centred at the origin: the circle's
// half-width integrated along x, clipped to the box's height.
float disc_area_first_quadrant(float x, float y, float r) {
    x = clamp(x, 0.0, r);
    y = clamp(y, 0.0, r);
    // Past this point the circle's half-width is below the box's top, so the integrand is the
    // box's height rather than the circle.
    float pinch = sqrt(max(r * r - y * y, 0.0));
    float flat = min(pinch, x);
    return y * flat + (circle_area_to(x, r) - circle_area_to(flat, r));
}

// The area of the axis-aligned box [lo, hi] inside the disc of radius r centred at the origin,
// for a box anywhere in the plane. `disc_area_first_quadrant` only measures boxes that start at
// the axes, so the box is split there first — keeping each piece's distance from the axis, which
// is what decides whether it is anywhere near the disc — and each piece is then the four-corner
// sum of that cumulative area.
float disc_area_box(float2 lo, float2 hi, float r) {
    // A piece is empty exactly when the box does not reach that side of the axis, in which case
    // both its ends come out at zero and the four-corner sum below cancels to nothing.
    float2 x_neg = float2(max(-min(hi.x, 0.0), 0.0), max(-lo.x, 0.0));
    float2 x_pos = float2(max(lo.x, 0.0), max(hi.x, 0.0));
    float2 y_neg = float2(max(-min(hi.y, 0.0), 0.0), max(-lo.y, 0.0));
    float2 y_pos = float2(max(lo.y, 0.0), max(hi.y, 0.0));

    float total = 0.0;
    [unroll]
    for (int i = 0; i < 4; ++i) {
        float2 xs = (i & 1) == 0 ? x_neg : x_pos;
        float2 ys = (i & 2) == 0 ? y_neg : y_pos;
        total += disc_area_first_quadrant(xs.y, ys.y, r)
            - disc_area_first_quadrant(xs.x, ys.y, r)
            - disc_area_first_quadrant(xs.y, ys.x, r)
            + disc_area_first_quadrant(xs.x, ys.x, r);
    }
    return total;
}

// The exact coverage of a pixel at `corner_to_point` (the point relative to the corner, both
// components <= 0 inside the quad, as `quad_own_sdf` mirrors it) by a quad whose corner has the
// given radius. `step` is one screen pixel in the quad's local units, per axis — only valid when
// the transform maps the pixel to an axis-aligned box, which is why the caller checks that first.
float rounded_corner_coverage(float2 corner_to_point, float2 step, float radius) {
    // The rectangle, whose two straight edges are exact per axis.
    float rect = clamp(0.5 - corner_to_point.x / step.x, 0.0, 1.0)
        * clamp(0.5 - corner_to_point.y / step.y, 0.0, 1.0);
    // The pixel's box in this mirrored space, clipped to the square the arc is inscribed in —
    // only there does the rounding cut anything.
    float2 half = 0.5 * step;
    float2 lo = max(corner_to_point - half, -radius);
    float2 hi = min(corner_to_point + half, 0.0);
    lo = min(lo, hi);
    float box_area = (hi.x - lo.x) * (hi.y - lo.y);
    // The arc's centre is the corner of that square; shift it to the disc's origin.
    float disc_area = disc_area_box(lo + radius, hi + radius, radius);
    // Both areas are in local units squared and the pixel is `step` of them per side.
    return clamp(rect - (box_area - disc_area) / (step.x * step.y), 0.0, 1.0);
}

float quad_sdf(float2 pt, Bounds bounds, Corners corner_radii) {
    float2 half_size = bounds.size / 2.;
    float2 center = bounds.origin + half_size;
    float2 center_to_point = pt - center;
    float corner_radius = pick_corner_radius(center_to_point, corner_radii);
    float2 corner_to_point = abs(center_to_point) - half_size;
    float2 corner_center_to_point = corner_to_point + corner_radius;
    return quad_sdf_impl(corner_center_to_point, corner_radius);
}

// Content-mask cutout, as an alpha multiplier (straight-alpha convention: only the alpha
// channel is scaled).
//
// The whole mask — straight edges as well as rounded corners — is shaped here from its
// distance field, so an edge that does not land on the pixel grid tapers over exactly one
// pixel instead of being cut off hard. `SV_ClipDistance` still culls the primitive, but it is
// padded by `AA_MARGIN` (see `distance_from_clip_rect_impl`), so it never decides the edge.
//
// Returns 1.0 when the mask coincides with the primitive's own antialiased outline
// (`owns_cutout`, with `own_origin`/`own_size`/`own_radii` expressed in mask space): the
// primitive's own SDF already cuts that outline in local space, and cutting the same edge
// twice would take the coverage to a squared ramp instead of a single one.
float content_mask_coverage(float2 mask_point, Bounds mask_bounds, Corners mask_radii,
                            float2 own_origin, float2 own_size, Corners own_radii,
                            bool owns_cutout) {
    bool mask_matches_own = owns_cutout &&
        all(mask_bounds.origin == own_origin) &&
        all(mask_bounds.size == own_size) &&
        mask_radii.top_left == own_radii.top_left &&
        mask_radii.top_right == own_radii.top_right &&
        mask_radii.bottom_left == own_radii.bottom_left &&
        mask_radii.bottom_right == own_radii.bottom_right;
    if (mask_matches_own) {
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
    float2 mask_half_size = mask_bounds.size / 2.0;
    float2 mask_center_to_point = mask_point - (mask_bounds.origin + mask_half_size);
    float mask_corner_radius = pick_corner_radius(mask_center_to_point, mask_radii);
    float2 mask_corner_to_point = abs(mask_center_to_point) - mask_half_size;
    if (mask_corner_to_point.x + mask_corner_radius >= 0.0
        && mask_corner_to_point.y + mask_corner_radius >= 0.0) {
        return rounded_corner_coverage(mask_corner_to_point, float2(1.0, 1.0), mask_corner_radius);
    }
    float mask_distance = quad_sdf(mask_point, mask_bounds, mask_radii);
    return saturate(0.5 - mask_distance);
}

// Stripe pattern (background tag 2): the signed distance from `pt` to the nearest stripe
// edge, measured across the stripes. `period` is the stripe period and `half_stripe` half the
// painted stripe's width, both in the pattern's own (local) units.
float stripe_sdf(float2 pt, float2 origin, float2x2 rotation, float period, float half_stripe) {
    float2 rotated = mul(pt - origin, rotation);
    float wrapped = fmod(rotated.x, period);
    return min(wrapped, period - wrapped) - half_stripe;
}

// Checkerboard (background tag 3): the signed distance from `pt` to the nearest cell edge,
// in local units. The coloured cells are half the board, so the field is symmetric.
float checker_sdf(float2 pt, float2 origin, float cell) {
    float2 cell_position = frac((pt - origin) / cell);
    float2 edge = min(cell_position, 1.0 - cell_position);
    return min(edge.x, edge.y) * cell;
}

// Coverage of a pattern at `pt`. `pixel_x`/`pixel_y` are one screen pixel expressed in the
// pattern's own space, taken by the caller from the interpolated local position's derivatives,
// so a scaled pattern keeps a one-pixel edge like everything else.
//
// These do *not* scale the ramp by the field's own derivative the way `sdf_coverage` does: a
// pattern's field is symmetric about the very edge the coverage is about, so its derivative
// reads zero there and the ramp would collapse to a hard step — which is exactly what happened
// to the checkerboard when this was written the other way. How far a screen pixel moves in the
// pattern's space is the scale that never vanishes.
float stripe_coverage(float2 pt, float2 origin, float2x2 rotation, float period,
                      float half_stripe, float2 pixel_x, float2 pixel_y) {
    float centre = stripe_sdf(pt, origin, rotation, period, half_stripe);
    float per_pixel = max(0.5 * (length(pixel_x) + length(pixel_y)), 1e-5);
    return saturate(0.5 - centre / per_pixel);
}

float checker_coverage(float2 pt, float2 origin, float cell,
                       float2 pixel_x, float2 pixel_y) {
    float centre = checker_sdf(pt, origin, cell);
    float per_pixel = max(0.5 * (length(pixel_x) + length(pixel_y)), 1e-5);
    return saturate(0.5 - centre / per_pixel);
}

GradientColor prepare_gradient_color(uint tag, uint color_space, Hsla solid, LinearColorStop colors[2]) {
    GradientColor output;
    if (tag == 0 || tag == 2 || tag == 3) {
        output.solid = hsla_to_rgba(solid);
    } else if (tag == 1 || tag == 4 || tag == 5) {
        output.color0 = hsla_to_rgba(colors[0].color);
        output.color1 = hsla_to_rgba(colors[1].color);

        // Prepare color space in vertex for avoid conversion
        // in fragment shader for performance reasons
        if (color_space == 1) {
            // Oklab
            output.color0 = srgb_to_oklab(output.color0);
            output.color1 = srgb_to_oklab(output.color1);
        }
    }

    return output;
}

float2x2 rotate2d(float angle) {
    float s = sin(angle);
    float c = cos(angle);
    return float2x2(c, -s, s, c);
}

// Resolves a gradient's ramp parameter into a color: applies the color stops, mixes in the
// requested color space, and dithers to hide 8-bit banding. Shared by the linear and radial
// gradients so both ramps behave identically.
float4 gradient_ramp_color(Background background, float t, float2 position,
                           float4 color0, float4 color1) {
    // Adjust t based on the stop percentages.
    t = clamp(
        (t - background.colors[0].percentage)
            / (background.colors[1].percentage - background.colors[0].percentage),
        0.0, 1.0);

    float4 color;
    switch (background.color_space) {
        case 1: {
            float4 oklab_color = lerp(color0, color1, t);
            color = oklab_to_srgb(oklab_color);
            break;
        }
        default:
            color = lerp(color0, color1, t);
            break;
    }

    // Dither to reduce banding in gradients (especially dark/alpha).
    // Triangular-distributed noise breaks up 8-bit quantization steps.
    // ±2/255 for RGB (enough for dark-on-dark compositing),
    // ±3/255 for alpha (needs more because alpha × dark color = tiny steps).
    {
        float2 seed = position * 0.6180339887; // golden ratio spread
        float r1 = frac(sin(dot(seed, float2(12.9898, 78.233))) * 43758.5453);
        float r2 = frac(sin(dot(seed, float2(39.3460, 11.135))) * 24634.6345);
        float tri = r1 + r2 - 1.0; // triangular PDF, range [-1, +1]
        color.rgb += tri * 2.0 / 255.0;
        color.a   += tri * 3.0 / 255.0;
    }

    return color;
}

float4 gradient_color(Background background,
                      float2 position,
                      Bounds bounds,
                      float4 solid_color, float4 color0, float4 color1) {
    float4 color;

    switch (background.tag) {
        case 0:
            color = solid_color;
            break;
        case 1: {
            // -90 degrees to match the CSS gradient angle.
            float gradient_angle = background.gradient_angle_or_pattern_height;
            float radians = (fmod(gradient_angle, 360.0) - 90.0) * (M_PI_F / 180.0);
            float2 direction = float2(cos(radians), sin(radians));

            // Expand the short side to be the same as the long side
            if (bounds.size.x > bounds.size.y) {
                direction.y *= bounds.size.y / bounds.size.x;
            } else {
                direction.x *=  bounds.size.x / bounds.size.y;
            }

            // Get the t value for the linear gradient with the color stop percentages.
            float2 half_size = bounds.size * 0.5;
            float2 center = bounds.origin + half_size;
            float2 center_to_point = position - center;
            float t = dot(center_to_point, direction) / length(direction);
            // Check the direct to determine the use x or y
            if (abs(direction.x) > abs(direction.y)) {
                t = (t + half_size.x) / bounds.size.x;
            } else {
                t = (t + half_size.y) / bounds.size.y;
            }

            color = gradient_ramp_color(background, t, position, color0, color1);
            break;
        }
        case 2: {
            float gradient_angle_or_pattern_height = background.gradient_angle_or_pattern_height;
            float pattern_width = (gradient_angle_or_pattern_height / 65535.0f) / 255.0f;
            float pattern_interval = fmod(gradient_angle_or_pattern_height, 65535.0f) / 255.0f;
            float pattern_height = pattern_width + pattern_interval;
            float stripe_angle = M_PI_F / 4.0;
            float pattern_period = pattern_height * sin(stripe_angle);
            float2x2 rotation = rotate2d(stripe_angle);
            float half_stripe = pattern_period * (pattern_width / pattern_height) * 0.5f;
            color = solid_color;
            color.a *= stripe_coverage(position, bounds.origin, rotation, pattern_period,
                half_stripe, ddx(position), ddy(position));
            break;
        }
        case 3: {
            // checkerboard
            float size = background.gradient_angle_or_pattern_height;
            float2 relative_position = position - bounds.origin;

            float x_index = floor(relative_position.x / size);
            float y_index = floor(relative_position.y / size);
            float should_be_colored = (x_index + y_index) % 2.0;

            // Antialias the cell edges, or the alternating squares show hard diagonal
            // staircases wherever they meet at an angle.
            float cell_coverage = checker_coverage(position, bounds.origin, size,
                ddx(position), ddy(position));

            color = solid_color;
            color.a *= saturate(should_be_colored) * cell_coverage;
            break;
        }
        case 4:
        case 5: {
            // Radial gradient: CSS `radial-gradient(<shape> <size> at <center>, ...)`. The center
            // is stored as fractions of the element's size and may lie outside it.
            float2 center = bounds.origin
                + float2(background.gradient_angle_or_pattern_height, background.radial_center_y)
                    * bounds.size;
            float2 offset = position - center;
            // The ending-shape size: side sizes use the distance from the center to the
            // nearest/farthest side on each axis, corner sizes scale those by sqrt(2) so the
            // ending shape meets the chosen corner exactly at t = 1.
            bool is_closest = background.radial_size == 1 || background.radial_size == 3;
            bool is_corner = background.radial_size == 0 || background.radial_size == 3;
            float2 closest_side = min(center - bounds.origin, bounds.origin + bounds.size - center);
            float2 farthest_side = max(center - bounds.origin, bounds.origin + bounds.size - center);
            float2 side = is_closest ? closest_side : farthest_side;
            // Ellipses take the per-axis distances; circles take a single radius — the nearest or
            // farthest side, or the corner distance.
            float2 radii = is_corner ? side * 1.41421356 : side;
            if (background.tag == 5) {
                float side_radius = is_closest ? min(side.x, side.y) : max(side.x, side.y);
                float radius = is_corner ? length(side) : side_radius;
                radii = float2(radius, radius);
            }
            // A zero-sized box would divide by zero; fall back to the last stop.
            float t = 1.0;
            if (radii.x > 0.0 && radii.y > 0.0) {
                t = length(offset / radii);
            }
            color = gradient_ramp_color(background, t, position, color0, color1);
            break;
        }
    }

    return color;
}

// Returns the dash velocity of a corner given the dash velocity of the two
// sides, by returning the slower velocity (larger dashes).
//
// Since 0 is used for dash velocity when the border width is 0 (instead of
// +inf), this returns the other dash velocity in that case.
//
// An alternative to this might be to appropriately interpolate the dash
// velocity around the corner, but that seems overcomplicated.
float corner_dash_velocity(float dv1, float dv2) {
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
float dash_alpha(
    float t, float period, float length, float dash_velocity,
    float antialias_threshold
) {
    float half_period = period / 2.0;
    float half_length = length / 2.0;
    // Value in [-half_period, half_period]
    // The dash is in [-half_length, half_length]
    float centered = fmod(t + half_period - half_length, period) - half_period;
    // Signed distance for the dash, negative values are inside the dash
    float signed_distance = abs(centered) - half_length;
    // Antialiased alpha based on the signed distance
    return saturate(antialias_threshold - signed_distance / dash_velocity);
}

// Distance to the nearest point on a quarter ellipse, to first order: the implicit value
// F = |p / r| - 1 divided by its gradient magnitude. This is exact for circular corners and
// far more accurate than an average-radius scaling for eccentric ones.
// The components of `point` are expected to be positive.
//
// Negative on the outside and positive on the inside.
float quarter_ellipse_sdf(float2 pt, float2 radii) {
    float2 circle_vec = pt / radii;
    float len = length(circle_vec);
    float2 gradient = circle_vec / max(radii, 1e-5) / max(len, 1e-5);
    return (1.0 - len) / max(length(gradient), 1e-5);
}

// The border's inner edge as a distance field of a point in the quad's local space, mirroring
// the branch `quad_fragment` picks at the pixel centre (0 straight, 1 provably outside,
// 2 circular, 3 ellipse) so the samples around the centre stay on one continuous field.
float quad_inner_sdf(float2 the_point, float2 half_size, float corner_radius,
                     float2 reduced_border, float outer_sdf, uint branch) {
    float2 corner_to_point = abs(the_point - half_size) - half_size;
    if (branch == 0u) {
        float2 straight = corner_to_point + reduced_border;
        return -max(straight.x, straight.y);
    } else if (branch == 1u) {
        return -1.0;
    } else if (branch == 2u) {
        return -(outer_sdf + reduced_border.x);
    } else {
        float2 ellipse_radii =
            max(float2(0.0, 0.0), float2(corner_radius, corner_radius) - reduced_border);
        return quarter_ellipse_sdf(corner_to_point + corner_radius, ellipse_radii);
    }
}

// The border ring's inner edge coverage, from `quad_inner_sdf` at the four quarter points (see
// `sdf_coverage`). The branch is the one the pixel centre picked, so the samples stay on a
// single continuous field.
float quad_inner_coverage(float2 the_point, float2 step_x, float2 step_y, float2 half_size,
                          float corner_radius, float2 reduced_border, uint branch) {
    return sdf_coverage(
        quad_inner_sdf(the_point - step_x - step_y, half_size, corner_radius, reduced_border,
            quad_own_sdf(the_point - step_x - step_y, half_size, corner_radius), branch),
        quad_inner_sdf(the_point - step_x + step_y, half_size, corner_radius, reduced_border,
            quad_own_sdf(the_point - step_x + step_y, half_size, corner_radius), branch),
        quad_inner_sdf(the_point + step_x - step_y, half_size, corner_radius, reduced_border,
            quad_own_sdf(the_point + step_x - step_y, half_size, corner_radius), branch),
        quad_inner_sdf(the_point + step_x + step_y, half_size, corner_radius, reduced_border,
            quad_own_sdf(the_point + step_x + step_y, half_size, corner_radius), branch));
}


/*
**
**              Quads
**
*/

struct Quad {
    uint order;
    uint border_style;
    // Non-zero when the quad paints only its fully covered interior — a later layer repaints
    // the same outline, so the antialiased band is left to it. See
    // `Scene::collapse_covered_outlines`.
    uint suppress_partial_coverage;
    uint pad;
    Bounds bounds;
    Bounds content_mask;
    Corners content_mask_corner_radii;
    Background background;
    Hsla border_color;
    Corners corner_radii;
    Edges border_widths;
    TransformationMatrix transformation;
};

struct QuadVertexOutput {
    nointerpolation uint quad_id: TEXCOORD0;
    float4 position: SV_Position;
    nointerpolation float4 border_color: COLOR0;
    nointerpolation float4 background_solid: COLOR1;
    nointerpolation float4 background_color0: COLOR2;
    nointerpolation float4 background_color1: COLOR3;
    float4 clip_distance: SV_ClipDistance;
    // Position in the quad's untransformed (local) coordinate space, used by the fragment
    // shader for the SDF/gradient so transforms don't distort corners or borders.
    float2 local_position: TEXCOORD4;
};

struct QuadFragmentInput {
    nointerpolation uint quad_id: TEXCOORD0;
    float4 position: SV_Position;
    nointerpolation float4 border_color: COLOR0;
    nointerpolation float4 background_solid: COLOR1;
    nointerpolation float4 background_color0: COLOR2;
    nointerpolation float4 background_color1: COLOR3;
    // Must be declared here too, in the same position as in `QuadVertexOutput`. FXC assigns
    // hardware input registers in declaration order, so this slot otherwise shifts
    // `local_position` (TEXCOORD4) to a different register than the VS emits it on and the
    // draw fails to link: "Semantic 'TEXCOORD' is defined for mismatched hardware registers"
    // — the quad pixel shader never runs and no rectangles appear.
    float4 clip_distance: SV_ClipDistance;
    // Position in the quad's untransformed (local) coordinate space, used by the fragment
    // shader for the SDF/gradient so transforms don't distort corners or borders.
    float2 local_position: TEXCOORD4;
};

StructuredBuffer<Quad> quads: register(t1);

QuadVertexOutput quad_vertex(uint vertex_id: SV_VertexID, uint quad_id: SV_InstanceID) {
    float2 unit_vertex = float2(float(vertex_id & 1u), 0.5 * float(vertex_id & 2u));
    Quad quad = quads[quad_id];
    // Grow the local rect so the rasterizer still emits fragments for the pixels the coverage
    // ramp below reaches outside the outline (see `AA_MARGIN`).
    float2 margin = aa_margin_local(quad.transformation.rotation_scale);
    float2 local_position = unit_vertex * (quad.bounds.size + 2.0 * margin)
        + (quad.bounds.origin - margin);
    // Apply the transform around the quad's center, matching `CssTransform::to_matrix` on the
    // Rust side. FXC packs structured-buffer matrices column-major (m00 m10 / m01 m11), i.e.
    // the transpose of the Rust row-major `TransformationMatrix`, so the row-vector form
    // `mul(vector, matrix)` — not `mul(matrix, vector)` — applies the Rust matrix correctly.
    float2 transformed_position = mul(local_position, quad.transformation.rotation_scale)
        + quad.transformation.translation;
    float4 device_position = to_device_position_impl(transformed_position);

    GradientColor gradient = prepare_gradient_color(
        quad.background.tag,
        quad.background.color_space,
        quad.background.solid,
        quad.background.colors
    );
    // A mask that coincides with the quad's own bounds is the quad's own cutout (see
    // `quad_fragment`): the quad's local-space outline already bounds the painted area, so
    // clipping the straight edges against it would slice the transformed quad at its
    // untransformed rect. Skip the clip in that case.
    bool mask_is_own = all(quad.content_mask.origin == quad.bounds.origin) &&
        all(quad.content_mask.size == quad.bounds.size);
    float4 clip_distance = mask_is_own
        ? float4(1.0, 1.0, 1.0, 1.0)
        : distance_from_clip_rect_aa_impl(transformed_position, quad.content_mask);
    float4 border_color = hsla_to_rgba(quad.border_color);

    QuadVertexOutput output;
    output.position = device_position;
    output.border_color = border_color;
    output.quad_id = quad_id;
    output.background_solid = gradient.solid;
    output.background_color0 = gradient.color0;
    output.background_color1 = gradient.color1;
    output.clip_distance = clip_distance;
    output.local_position = local_position;
    return output;
}

float4 quad_fragment(QuadFragmentInput input): SV_Target {
    Quad quad = quads[input.quad_id];

    float2 size = quad.bounds.size;
    float2 half_size = size / 2.;
    float2 the_point = input.local_position - quad.bounds.origin;
    float2 center_to_point = the_point - half_size;

    // Signed distance field threshold for inclusion of pixels. 0.5 is the
    // minimum distance between the center of the pixel and the edge.
    const float antialias_threshold = 0.5;

    // Radius of the nearest corner
    float corner_radius = pick_corner_radius(center_to_point, quad.corner_radii);

    // Vector from the corner of the quad bounds to the point, after mirroring
    // the point into the bottom right quadrant. Both components are <= 0.
    float2 corner_to_point = abs(center_to_point) - half_size;

    // Vector from the point to the center of the rounded corner's circle, also
    // mirrored into bottom right quadrant.
    float2 corner_center_to_point = corner_to_point + corner_radius;

    // One screen pixel, expressed in the quad's local space. The interpolated local position
    // is linear in the pixel coordinates, so these derivatives are exact for the affine
    // transform a quad carries — including rotation, skew and non-uniform scale, where the
    // two steps differ in both length and direction. They are taken here, before any branch,
    // because a derivative needs the whole 2x2 quad's lanes still live.
    float2 pixel_x = ddx(input.local_position);
    float2 pixel_y = ddy(input.local_position);

    // A quarter of a pixel, either way, in the quad's local space — where `sdf_coverage` takes
    // its four samples.
    float2 step_x = 0.25 * pixel_x;
    float2 step_y = 0.25 * pixel_y;

    // Signed distance of the point to the outside edge of the quad's border, and the quad's
    // own coverage of it. The centre sample sizes the field's gradient and drives the border
    // test below; the four quarter points give the coverage.
    //
    // Inside a rounded corner's quadrant the coverage is the pixel's exact area instead: that is
    // where the boundary curves, and the ramp is only first order there (see
    // `rounded_corner_coverage`). It needs the pixel to be an axis-aligned box in the quad's
    // local space, which a rotated or skewed transform does not give it.
    float outer_sdf = quad_own_sdf(the_point, half_size, corner_radius);
    bool near_rounded_corner = corner_center_to_point.x >= 0.0
        && corner_center_to_point.y >= 0.0;
    bool axis_aligned = quad.transformation.rotation_scale[0][1] == 0.0
        && quad.transformation.rotation_scale[1][0] == 0.0;
    float own_coverage;
    if (axis_aligned && near_rounded_corner) {
        own_coverage = rounded_corner_coverage(corner_to_point,
            float2(length(pixel_x), length(pixel_y)), corner_radius);
    } else {
        own_coverage = quad_own_coverage(the_point, step_x, step_y, half_size, corner_radius);
    }

    // Content-mask cutout. The whole mask — straight edges as well as corners — is shaped by
    // the mask's distance field (the vertex shader's `SV_ClipDistance` was padded by
    // `AA_MARGIN` and only culls). The mask lives in scene space, which *is* device pixels and
    // whose mapping from the screen is the identity, so its gradient is exactly 1 and its
    // `0.5 - distance` ramp is already one pixel wide — but it is cut as a plain field, so the
    // quad's own outline below is not asked to cut it again. A mask that coincides with the
    // quad's own *untransformed* bounds and radii is the quad's own rounded cutout, and is
    // skipped for exactly that reason.
    float mask_coverage = content_mask_coverage(
        mul(input.local_position, quad.transformation.rotation_scale)
            + quad.transformation.translation,
        quad.content_mask, quad.content_mask_corner_radii,
        quad.bounds.origin, quad.bounds.size,
        quad.corner_radii,
        true);

    // The quad's own coverage and the content mask combine with `min`: the mask may only take
    // coverage away, and both describe edges of the same kind, so the smaller ramp is the
    // right one where they meet.
    float coverage = min(own_coverage, mask_coverage);

    // A layer that repaints an outline another, later layer covers contributes nothing but the
    // interior: its band would be a second application of the same coverage, and stacked
    // coverages compose as `1 - (1 - c)^n` — the arc's partial pixels merge into a hard step,
    // which is what flattens a small radius into a straight diagonal. See
    // `Scene::collapse_covered_outlines`.
    if (quad.suppress_partial_coverage != 0u) {
        coverage = coverage >= 1.0 ? 1.0 : 0.0;
    }

    float4 background_color = gradient_color(quad.background, input.local_position, quad.bounds,
    input.background_solid, input.background_color0, input.background_color1);

    float2 border = float2(
        center_to_point.x < 0.0 ? quad.border_widths.left : quad.border_widths.right,
        center_to_point.y < 0.0 ? quad.border_widths.top : quad.border_widths.bottom
    );

    // 0-width borders are reduced so that `inner_sdf >= antialias_threshold`.
    // The purpose of this is to not draw antialiasing pixels in this case.
    float2 reduced_border = float2(
        border.x == 0.0 ? -antialias_threshold : border.x,
        border.y == 0.0 ? -antialias_threshold : border.y
    );

    // Whether the nearest point on the border is rounded (decided with the coverage above, so
    // that the exact-area path and this agree on which fragments are in the corner's quadrant)
    bool is_near_rounded_corner = near_rounded_corner;

    // Vector from straight border inner corner to point.
    //
    // 0-width borders are turned into width -1 so that inner_sdf is > 1.0 near
    // the border. Without this, antialiasing pixels would be drawn.
    float2 straight_border_inner_corner_to_point = corner_to_point + reduced_border;

    // Whether the point is beyond the inner edge of the straight border
    bool is_beyond_inner_straight_border =
        straight_border_inner_corner_to_point.x > 0.0 ||
        straight_border_inner_corner_to_point.y > 0.0;

    // Whether the point is far enough inside the quad, such that the pixels are
    // not affected by the straight border.
    bool is_within_inner_straight_border =
        straight_border_inner_corner_to_point.x < -antialias_threshold &&
        straight_border_inner_corner_to_point.y < -antialias_threshold;

    bool unrounded = quad.corner_radii.top_left == 0.0 &&
        quad.corner_radii.top_right == 0.0 &&
        quad.corner_radii.bottom_left == 0.0 &&
        quad.corner_radii.bottom_right == 0.0;

    // Fast path when the quad is not rounded and doesn't have any border: the outer distance
    // above is the whole story. (There is no transform-dependent variant any more: the SDF
    // coverage is exact for axis-aligned, scaled, skewed and rotated quads alike, so a quad
    // that is merely translated or scaled no longer needs a separate path — the old one
    // returned the background with no coverage at all, which is only correct while the
    // transform happens to keep the bounds on the pixel grid.)
    if (quad.border_widths.top == 0.0 &&
        quad.border_widths.left == 0.0 &&
        quad.border_widths.right == 0.0 &&
        quad.border_widths.bottom == 0.0 &&
        unrounded) {
        return background_color * float4(1.0, 1.0, 1.0, coverage);
    }

    // Fast path for points that must be part of the background
    if (is_within_inner_straight_border && !is_near_rounded_corner) {
        return background_color * float4(1.0, 1.0, 1.0, coverage);
    }

    // Approximate signed distance of the point to the inside edge of the quad's
    // border. It is negative outside this edge (within the border), and
    // positive inside.
    //
    // This is not always an accurate signed distance:
    // * The rounded portions with varying border width use an approximation of
    //   nearest-point-on-ellipse.
    // * When it is quickly known to be outside the edge, -1.0 is used.
    uint inner_branch = 3u;
    if (corner_center_to_point.x <= 0.0 || corner_center_to_point.y <= 0.0) {
        // Fast paths for straight borders
        inner_branch = 0u;
    } else if (is_beyond_inner_straight_border) {
        // Fast path for points that must be outside the inner edge
        inner_branch = 1u;
    } else if (reduced_border.x == reduced_border.y) {
        // Fast path for circular inner edge.
        inner_branch = 2u;
    }
    float inner_sdf = quad_inner_sdf(the_point, half_size, corner_radius, reduced_border,
                                     outer_sdf, inner_branch);

    // Negative when inside the border
    float border_sdf = max(inner_sdf, outer_sdf);

    float4 color = background_color;
    if (border_sdf < antialias_threshold) {
        float4 border_color = input.border_color;
        // Dashed border logic when border_style == 1
        if (quad.border_style == 1) {
            // Position along the perimeter in "dash space", where each dash
            // period has length 1
            float t = 0.0;

            // Total number of dash periods, so that the dash spacing can be
            // adjusted to evenly divide it
            float max_t = 0.0;

            // Border width is proportional to dash size. This is the behavior
            // used by browsers, but also avoids dashes from different segments
            // overlapping when dash size is smaller than the border width.
            //
            // Dash pattern: (2 * border width) dash, (1 * border width) gap
            const float dash_length_per_width = 2.0;
            const float dash_gap_per_width = 1.0;
            const float dash_period_per_width = dash_length_per_width + dash_gap_per_width;

            // Since the dash size is determined by border width, the density of
            // dashes varies. Multiplying a pixel distance by this returns a
            // position in dash space - it has units (dash period / pixels). So
            // a dash velocity of (1 / 10) is 1 dash every 10 pixels.
            float dash_velocity = 0.0;

            // Dividing this by the border width gives the dash velocity
            const float dv_numerator = 1.0 / dash_period_per_width;

            if (unrounded) {
                // When corners aren't rounded, the dashes are separately laid
                // out on each straight line, rather than around the whole
                // perimeter. This way each line starts and ends with a dash.
                bool is_horizontal = corner_center_to_point.x < corner_center_to_point.y;
                // Choosing the right border width for dashed borders.
                // TODO: A better solution exists taking a look at the whole file.
                // this does not fix single dashed borders at the corners
                float2 dashed_border = float2(
                    max(quad.border_widths.bottom, quad.border_widths.top),
                    max(quad.border_widths.right, quad.border_widths.left)
                );
                float border_width = is_horizontal ? dashed_border.x : dashed_border.y;
                dash_velocity = dv_numerator / border_width;
                t = is_horizontal ? the_point.x : the_point.y;
                t *= dash_velocity;
                max_t = is_horizontal ? size.x : size.y;
                max_t *= dash_velocity;
            } else {
                // When corners are rounded, the dashes are laid out clockwise
                // around the whole perimeter.

                float r_tr = quad.corner_radii.top_right;
                float r_br = quad.corner_radii.bottom_right;
                float r_bl = quad.corner_radii.bottom_left;
                float r_tl = quad.corner_radii.top_left;

                float w_t = quad.border_widths.top;
                float w_r = quad.border_widths.right;
                float w_b = quad.border_widths.bottom;
                float w_l = quad.border_widths.left;

                // Straight side dash velocities
                float dv_t = w_t <= 0.0 ? 0.0 : dv_numerator / w_t;
                float dv_r = w_r <= 0.0 ? 0.0 : dv_numerator / w_r;
                float dv_b = w_b <= 0.0 ? 0.0 : dv_numerator / w_b;
                float dv_l = w_l <= 0.0 ? 0.0 : dv_numerator / w_l;

                // Straight side lengths in dash space
                float s_t = (size.x - r_tl - r_tr) * dv_t;
                float s_r = (size.y - r_tr - r_br) * dv_r;
                float s_b = (size.x - r_br - r_bl) * dv_b;
                float s_l = (size.y - r_bl - r_tl) * dv_l;

                float corner_dash_velocity_tr = corner_dash_velocity(dv_t, dv_r);
                float corner_dash_velocity_br = corner_dash_velocity(dv_b, dv_r);
                float corner_dash_velocity_bl = corner_dash_velocity(dv_b, dv_l);
                float corner_dash_velocity_tl = corner_dash_velocity(dv_t, dv_l);

                // Corner lengths in dash space
                float c_tr = r_tr * (M_PI_F / 2.0) * corner_dash_velocity_tr;
                float c_br = r_br * (M_PI_F / 2.0) * corner_dash_velocity_br;
                float c_bl = r_bl * (M_PI_F / 2.0) * corner_dash_velocity_bl;
                float c_tl = r_tl * (M_PI_F / 2.0) * corner_dash_velocity_tl;

                // Cumulative dash space upto each segment
                float upto_tr = s_t;
                float upto_r = upto_tr + c_tr;
                float upto_br = upto_r + s_r;
                float upto_b = upto_br + c_br;
                float upto_bl = upto_b + s_b;
                float upto_l = upto_bl + c_bl;
                float upto_tl = upto_l + s_l;
                max_t = upto_tl + c_tl;

                if (is_near_rounded_corner) {
                    float radians = atan2(corner_center_to_point.y, corner_center_to_point.x);
                    float corner_t = radians * corner_radius;

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
                            // Subtracted because radians is pi/1 to 0 when
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
                    bool is_horizontal = corner_center_to_point.x < corner_center_to_point.y;
                    if (is_horizontal) {
                        if (center_to_point.y < 0.0) {
                            dash_velocity = dv_t;
                            t = (the_point.x - r_tl) * dash_velocity;
                        } else {
                            dash_velocity = dv_b;
                            t = upto_bl - (the_point.x - r_bl) * dash_velocity;
                        }
                    } else {
                        if (center_to_point.x < 0.0) {
                            dash_velocity = dv_l;
                            t = upto_tl - (the_point.y - r_tl) * dash_velocity;
                        } else {
                            dash_velocity = dv_r;
                            t = upto_r + (the_point.y - r_tr) * dash_velocity;
                        }
                    }
                }
            }
            float dash_length = dash_length_per_width / dash_period_per_width;
            float desired_dash_gap = dash_gap_per_width / dash_period_per_width;

            // Straight borders should start and end with a dash, so max_t is
            // reduced to cause this.
            max_t -= unrounded ? dash_length : 0.0;
            if (max_t >= 1.0) {
                // Adjust dash gap to evenly divide max_t
                float dash_count = floor(max_t);
                float dash_period = max_t / dash_count;
                border_color.a *= dash_alpha(t, dash_period, dash_length, dash_velocity, antialias_threshold);
            } else if (unrounded) {
                // When there isn't enough space for the full gap between the
                // two start / end dashes of a straight border, reduce gap to
                // make them fit.
                float dash_gap = max_t - dash_length;
                if (dash_gap > 0.0) {
                    float dash_period = dash_length + dash_gap;
                    border_color.a *= dash_alpha(t, dash_period, dash_length, dash_velocity, antialias_threshold);
                }
            }
        }

        // Composite the border over the background, clipped to the inner edge by its coverage,
        // instead of interpolating between the two in *straight* alpha. Interpolating drags the
        // result towards the background's colour as the coverage falls, and a border quad's
        // background is transparent black, so every antialiased inner edge came out dragged
        // towards black — a dark fringe just inside each border, and a dark inner arc at every
        // rounded corner. Scaling the border's alpha by the coverage keeps its colour and ramps
        // only how much of it there is, which is what an antialiased edge is.
        float4 blended_border = float4(border_color.rgb,
            border_color.a * quad_inner_coverage(the_point, step_x, step_y, half_size,
                corner_radius, reduced_border, inner_branch));
        color = over(background_color, blended_border);
    }

    return color * float4(1.0, 1.0, 1.0, coverage);
}

/*
**
**              Shadows
**
*/

struct Shadow {
    uint order;
    float blur_radius;
    Bounds bounds;
    Corners corner_radii;
    Bounds content_mask;
    Corners content_mask_corner_radii;
    Hsla color;
    Bounds element_bounds;
    Corners element_corner_radii;
    uint inset;
    uint pad; // align to 8 bytes
};

struct ShadowVertexOutput {
    nointerpolation uint shadow_id: TEXCOORD0;
    float4 position: SV_Position;
    nointerpolation float4 color: COLOR;
    float4 clip_distance: SV_ClipDistance;
};

struct ShadowFragmentInput {
  nointerpolation uint shadow_id: TEXCOORD0;
  float4 position: SV_Position;
  nointerpolation float4 color: COLOR;
};

StructuredBuffer<Shadow> shadows: register(t1);

ShadowVertexOutput shadow_vertex(uint vertex_id: SV_VertexID, uint shadow_id: SV_InstanceID) {
    float2 unit_vertex = float2(float(vertex_id & 1u), 0.5 * float(vertex_id & 2u));
    Shadow shadow = shadows[shadow_id];

    Bounds bounds;
    if (shadow.inset != 0u) {
        // The inset shadow is bounded by the element's outline, whose own coverage ramp
        // (`saturate(0.5 - element_distance)` below) needs fragments on both sides of it.
        bounds = shadow.element_bounds;
        bounds.origin -= AA_MARGIN;
        bounds.size += 2.0 * AA_MARGIN;
    } else {
        // Leave room for the gaussian tail outside the shadow rect, plus the coverage ramp.
        float margin = 3.0 * shadow.blur_radius + AA_MARGIN;
        bounds = shadow.bounds;
        bounds.origin -= margin;
        bounds.size += 2.0 * margin;
    }

    float4 device_position = to_device_position(unit_vertex, bounds);
    float4 clip_distance = distance_from_clip_rect(unit_vertex, bounds, shadow.content_mask);
    float4 color = hsla_to_rgba(shadow.color);

    ShadowVertexOutput output;
    output.position = device_position;
    output.color = color;
    output.shadow_id = shadow_id;
    output.clip_distance = clip_distance;

    return output;
}

float4 shadow_fragment(ShadowFragmentInput input): SV_TARGET {
    Shadow shadow = shadows[input.shadow_id];

    // Rounded content-mask cutout: straight mask edges were already SV_ClipDistance-clipped in
    // `shadow_vertex`, so only the mask's rounded-corner arcs are cut here. Skipped when the
    // mask coincides with the shadow's own cutout — the non-inset shadow's rounded `bounds`,
    // or the inset shadow's rounded `element_bounds`, both cut below. (FXC can't ternary on
    // structs, so pick the own-cutout params with an if.)
    float2 own_origin = shadow.bounds.origin;
    float2 own_size = shadow.bounds.size;
    Corners own_radii = shadow.corner_radii;
    if (shadow.inset != 0u) {
        own_origin = shadow.element_bounds.origin;
        own_size = shadow.element_bounds.size;
        own_radii = shadow.element_corner_radii;
    }
    float mask_coverage = content_mask_coverage(
        input.position.xy, shadow.content_mask, shadow.content_mask_corner_radii,
        own_origin, own_size, own_radii,
        true);

    float2 half_size = shadow.bounds.size / 2.;
    float2 center = shadow.bounds.origin + half_size;
    float2 point0 = input.position.xy - center;
    float corner_radius = pick_corner_radius(point0, shadow.corner_radii);
    float blur_radius = shadow.blur_radius;

    float alpha;
    if (blur_radius == 0.) {
        float distance = quad_sdf(input.position.xy, shadow.bounds, shadow.corner_radii);
        alpha = saturate(0.5 - distance);
    } else {
        // The signal is only non-zero in a limited range, so don't waste samples
        float low = point0.y - half_size.y;
        float high = point0.y + half_size.y;
        float start = clamp(-3. * blur_radius, low, high);
        float end = clamp(3. * blur_radius, low, high);

        // Accumulate samples (we can get away with surprisingly few samples)
        float step = (end - start) / 4.;
        float y = start + step * 0.5;
        alpha = 0.;
        for (int i = 0; i < 4; i++) {
            alpha += blur_along_x(point0.x, point0.y - y, blur_radius,
                                corner_radius, half_size) *
                    gaussian(y, blur_radius) * step;
            y += step;
        }
    }

    if (shadow.inset != 0u) {
        // The inset shadow is the complement of the (blurred) hole rect, clipped to the element.
        // `saturate(0.5 - d)` gives a 1-pixel antialiased edge: d <= -0.5 -> 1, d >= 0.5 -> 0.
        alpha = 1.0 - alpha;
        float element_distance = quad_sdf(input.position.xy, shadow.element_bounds,
                                          shadow.element_corner_radii);
        alpha *= saturate(0.5 - element_distance);
    }

    alpha = min(alpha, mask_coverage);
    return input.color * float4(1., 1., 1., alpha);
}

/*
**
**              Path Rasterization
**
*/

struct PathRasterizationSprite {
    float2 xy_position;
    float2 st_position;
    Background color;
    Bounds bounds;
};

StructuredBuffer<PathRasterizationSprite> path_rasterization_sprites: register(t1);

struct PathVertexOutput {
    float4 position: SV_Position;
    float2 st_position: TEXCOORD0;
    nointerpolation uint vertex_id: TEXCOORD1;
    float4 clip_distance: SV_ClipDistance;
    // The gradient ramp is prepared per vertex, as `quad_vertex` does: running the same conversion
    // in the fragment shader cost a decode plus an Oklab conversion for every path pixel.
    nointerpolation float4 background_solid: COLOR0;
    nointerpolation float4 background_color0: COLOR1;
    nointerpolation float4 background_color1: COLOR2;
};

struct PathFragmentInput {
    float4 position: SV_Position;
    float2 st_position: TEXCOORD0;
    nointerpolation uint vertex_id: TEXCOORD1;
    // Must be declared here too, in the same position as in `PathVertexOutput`: FXC assigns
    // hardware registers in declaration order, so without this slot the COLOR varyings below land
    // on different registers than the VS emits them on and the draw fails to link.
    float4 clip_distance: SV_ClipDistance;
    nointerpolation float4 background_solid: COLOR0;
    nointerpolation float4 background_color0: COLOR1;
    nointerpolation float4 background_color1: COLOR2;
};

PathVertexOutput path_rasterization_vertex(uint vertex_id: SV_VertexID) {
    PathRasterizationSprite sprite = path_rasterization_sprites[vertex_id];

    PathVertexOutput output;
    output.position = to_device_position_impl(sprite.xy_position);
    output.st_position = sprite.st_position;
    output.vertex_id = vertex_id;
    output.clip_distance = distance_from_clip_rect_impl(sprite.xy_position, sprite.bounds);

    GradientColor gradient = prepare_gradient_color(
        sprite.color.tag, sprite.color.color_space, sprite.color.solid, sprite.color.colors
    );
    output.background_solid = gradient.solid;
    output.background_color0 = gradient.color0;
    output.background_color1 = gradient.color1;

    return output;
}

float4 path_rasterization_fragment(PathFragmentInput input): SV_Target {
    float2 dx = ddx(input.st_position);
    float2 dy = ddy(input.st_position);
    PathRasterizationSprite sprite = path_rasterization_sprites[input.vertex_id];

    Background background = sprite.color;
    Bounds bounds = sprite.bounds;

    // The gradient direction's x components: used as a vector for the length test and again
    // per component below.
    float2 d_st_x = float2(dx.x, dy.x);
    float alpha;
    if (length(d_st_x)) {
        alpha = 1.0;
    } else {
        float2 gradient = 2.0 * input.st_position.xx * d_st_x - float2(dx.y, dy.y);
        float f = input.st_position.x * input.st_position.x - input.st_position.y;
        float distance = f / length(gradient);
        alpha = saturate(0.5 - distance);
    }

    float4 color = gradient_color(background, input.position.xy, bounds,
        input.background_solid, input.background_color0, input.background_color1);
    return float4(color.rgb * color.a * alpha, alpha * color.a);
}

/*
**
**              Path Sprites
**
*/

struct PathSprite {
    Bounds bounds;
};

struct PathSpriteVertexOutput {
    float4 position: SV_Position;
    float2 texture_coords: TEXCOORD0;
};

StructuredBuffer<PathSprite> path_sprites: register(t1);

PathSpriteVertexOutput path_sprite_vertex(uint vertex_id: SV_VertexID, uint sprite_id: SV_InstanceID) {
    float2 unit_vertex = float2(float(vertex_id & 1u), 0.5 * float(vertex_id & 2u));
    PathSprite sprite = path_sprites[sprite_id];

    // Don't apply content mask because it was already accounted for when rasterizing the path
    float4 device_position = to_device_position(unit_vertex, sprite.bounds);

    float2 screen_position = sprite.bounds.origin + unit_vertex * sprite.bounds.size;
    float2 texture_coords = screen_position / global_viewport_size;

    PathSpriteVertexOutput output;
    output.position = device_position;
    output.texture_coords = texture_coords;
    return output;
}

float4 path_sprite_fragment(PathSpriteVertexOutput input): SV_Target {
    return t_sprite.Sample(s_sprite, input.texture_coords);
}

/*
**
**              Underlines
**
*/

struct Underline {
    uint order;
    uint pad;
    Bounds bounds;
    Bounds content_mask;
    Corners content_mask_corner_radii;
    Hsla color;
    float thickness;
    uint wavy;
    TransformationMatrix transformation;
};

struct UnderlineVertexOutput {
  nointerpolation uint underline_id: TEXCOORD0;
  float4 position: SV_Position;
  nointerpolation float4 color: COLOR;
  float4 clip_distance: SV_ClipDistance;
  // Position in the underline's untransformed (local) coordinate space, used by the
  // fragment shader for the wavy SDF so it isn't distorted by an ancestor transform.
  float2 local_position: TEXCOORD1;
};

struct UnderlineFragmentInput {
  nointerpolation uint underline_id: TEXCOORD0;
  float4 position: SV_Position;
  nointerpolation float4 color: COLOR;
  // Must be declared here too, in the same position as in `UnderlineVertexOutput`: FXC assigns
  // hardware input registers in declaration order, so a slot mismatch shifts `local_position`
  // to a different register than the VS emits it on and the draw fails to link.
  float2 local_position: TEXCOORD1;
};

StructuredBuffer<Underline> underlines: register(t1);

UnderlineVertexOutput underline_vertex(uint vertex_id: SV_VertexID, uint underline_id: SV_InstanceID) {
    float2 unit_vertex = float2(float(vertex_id & 1u), 0.5 * float(vertex_id & 2u));
    Underline underline = underlines[underline_id];
    // Grow the local rect so the coverage ramp below has fragments outside the outline
    // (see `AA_MARGIN`) — without them the rasterizer's pixel-centre rule would leave the
    // underline's ends hard regardless of sub-pixel position.
    float2 margin = aa_margin_local(underline.transformation.rotation_scale);
    float2 local_position = unit_vertex * (underline.bounds.size + 2.0 * margin)
        + (underline.bounds.origin - margin);
    float2 transformed_position =
        mul(local_position, underline.transformation.rotation_scale) + underline.transformation.translation;
    float4 device_position = to_device_position_impl(transformed_position);
    float4 clip_distance = distance_from_clip_rect_aa_impl(transformed_position, underline.content_mask);
    float4 color = hsla_to_rgba(underline.color);

    UnderlineVertexOutput output;
    output.position = device_position;
    output.color = color;
    output.underline_id = underline_id;
    output.clip_distance = clip_distance;
    output.local_position = local_position;
    return output;
}

// The wavy underline's stroke as a distance field of a point in the underline's local space:
// the distance across the stroke, signed, in local units.
float wavy_underline_sdf(float2 the_point, float2 size, float thickness) {
    const float WAVE_FREQUENCY = 2.0;
    const float WAVE_HEIGHT_RATIO = 0.8;
    float2 st = the_point / size.y - float2(0.0, 0.5);
    float frequency = (M_PI_F * WAVE_FREQUENCY * thickness) / size.y;
    float amplitude = (thickness * WAVE_HEIGHT_RATIO) / size.y;
    float sine = sin(st.x * frequency) * amplitude;
    float dSine = cos(st.x * frequency) * amplitude * frequency;
    float distance_in_pixels = ((st.y - sine) / sqrt(1. + dSine * dSine)) * size.y;
    float half_thickness = thickness * 0.5;
    return max(-(distance_in_pixels + half_thickness), distance_in_pixels - half_thickness);
}

float4 underline_fragment(UnderlineFragmentInput input): SV_Target {
    Underline underline = underlines[input.underline_id];
    // The underline's own rectangle. It is unrounded, so this is the plain box distance; it is
    // what keeps the geometry margin from painting past the underline's ends. The pixel steps
    // and both coverage terms are computed here, before the branch below, so their derivatives
    // are taken with every lane of the 2x2 quad still live.
    float2 pixel_x = ddx(input.local_position);
    float2 pixel_y = ddy(input.local_position);
    float2 step_x = 0.25 * pixel_x;
    float2 step_y = 0.25 * pixel_y;
    float2 half_size = underline.bounds.size / 2.0;
    float2 the_point = input.local_position - underline.bounds.origin;
    float own_coverage = quad_own_coverage(the_point, step_x, step_y, half_size, 0.0);
    // The mask's whole outline is cut here (`SV_ClipDistance` is padded by `AA_MARGIN` and only
    // culls). An underline has no rounded cutout of its own, so there is no mask-equals-own
    // skip.
    float mask_coverage = content_mask_coverage(
        mul(input.local_position, underline.transformation.rotation_scale)
            + underline.transformation.translation,
        underline.content_mask, underline.content_mask_corner_radii,
        underline.bounds.origin, underline.bounds.size, underline.content_mask_corner_radii,
        false);
    float coverage = min(own_coverage, mask_coverage);
    if (underline.wavy) {
        float2 size = underline.bounds.size;
        float thickness = underline.thickness;
        float alpha = sdf_coverage(
            wavy_underline_sdf(the_point - step_x - step_y, size, thickness),
            wavy_underline_sdf(the_point - step_x + step_y, size, thickness),
            wavy_underline_sdf(the_point + step_x - step_y, size, thickness),
            wavy_underline_sdf(the_point + step_x + step_y, size, thickness));
        return input.color * float4(1., 1., 1., min(alpha, coverage));
    } else {
        return input.color * float4(1., 1., 1., coverage);
    }
}

/*
**
**              Monochrome sprites
**
*/

struct MonochromeSprite {
    uint order;
    uint pad;
    Bounds bounds;
    Bounds content_mask;
    Corners content_mask_corner_radii;
    Hsla color;
    AtlasTile tile;
    TransformationMatrix transformation;
};

struct MonochromeSpriteVertexOutput {
    float4 position: SV_Position;
    float2 tile_position: POSITION;
    nointerpolation float4 color: COLOR;
    float4 clip_distance: SV_ClipDistance;
    nointerpolation uint sprite_id: TEXCOORD0;
    // Position in the sprite's untransformed (local) coordinate space — the fragment shader
    // reconstructs the scene-space content-mask point from it via `sprite.transformation`.
    float2 local_position: TEXCOORD1;
};

struct MonochromeSpriteFragmentInput {
    float4 position: SV_Position;
    float2 tile_position: POSITION;
    nointerpolation float4 color: COLOR;
    float4 clip_distance: SV_ClipDistance;
    // Must be declared here too, in the same order as in `MonochromeSpriteVertexOutput`: FXC
    // assigns hardware input registers in declaration order, so a slot mismatch shifts these
    // to different registers than the VS emits them on and the draw fails to link.
    nointerpolation uint sprite_id: TEXCOORD0;
    float2 local_position: TEXCOORD1;
};

StructuredBuffer<MonochromeSprite> mono_sprites: register(t1);

MonochromeSpriteVertexOutput monochrome_sprite_vertex(uint vertex_id: SV_VertexID, uint sprite_id: SV_InstanceID) {
    float2 unit_vertex = float2(float(vertex_id & 1u), 0.5 * float(vertex_id & 2u));
    MonochromeSprite sprite = mono_sprites[sprite_id];
    // Deliberately NOT grown by `AA_MARGIN`, unlike every other primitive here. A glyph's shape
    // is its texture's alpha, not its quad's outline, and this shader has no own-coverage term
    // to taper the extra geometry with: the only coverage it applies is the *content mask's*,
    // which is 1.0 across the whole interior. Growing the quad would therefore paint one more
    // pixel of the atlas tile's edge texel at full alpha all the way around every glyph, which
    // reads as text that is both bolder and blurrier.
    float2 local_position = unit_vertex * sprite.bounds.size + sprite.bounds.origin;
    float2 transformed_position =
        mul(local_position, sprite.transformation.rotation_scale) + sprite.transformation.translation;
    float4 device_position = to_device_position_impl(transformed_position);
    float4 clip_distance = distance_from_clip_rect_aa_impl(transformed_position, sprite.content_mask);
    float2 tile_position = to_tile_position(unit_vertex, sprite.tile);
    float4 color = hsla_to_rgba(sprite.color);

    MonochromeSpriteVertexOutput output;
    output.position = device_position;
    output.tile_position = tile_position;
    output.color = color;
    output.clip_distance = clip_distance;
    output.sprite_id = sprite_id;
    output.local_position = local_position;
    return output;
}

float4 monochrome_sprite_fragment(MonochromeSpriteFragmentInput input): SV_Target {
    float sample = t_sprite.Sample(s_sprite, input.tile_position).r;
    float alpha_corrected = apply_contrast_and_gamma_correction(sample, input.color.rgb, grayscale_enhanced_contrast, gamma_ratios);
    // Glyph quads carry no own rounded cutout in this shader, so the ancestor mask's rounded
    // corners are always cut here (straight-alpha convention: only the alpha channel scales).
    MonochromeSprite sprite = mono_sprites[input.sprite_id];
    float mask_coverage = content_mask_coverage(
        mul(input.local_position, sprite.transformation.rotation_scale)
            + sprite.transformation.translation,
        sprite.content_mask, sprite.content_mask_corner_radii,
        sprite.bounds.origin, sprite.bounds.size, sprite.content_mask_corner_radii,
        false);
    return float4(input.color.rgb, input.color.a * alpha_corrected * mask_coverage);
}

MonochromeSpriteVertexOutput subpixel_sprite_vertex(uint vertex_id: SV_VertexID, uint sprite_id: SV_InstanceID) {
    return monochrome_sprite_vertex(vertex_id, sprite_id);
}

SubpixelSpriteFragmentOutput subpixel_sprite_fragment(MonochromeSpriteFragmentInput input) {
    float3 sample = t_sprite.Sample(s_sprite, input.tile_position).rgb;
    if (is_bgr) {
        sample = sample.bgr;
    }
    float3 alpha_corrected = apply_contrast_and_gamma_correction3(sample, input.color.rgb, subpixel_enhanced_contrast, gamma_ratios);
    // Same mask cut as `monochrome_sprite_fragment`, applied to the alpha target only; the
    // foreground (opaque cover color) is left untouched.
    MonochromeSprite sprite = mono_sprites[input.sprite_id];
    float mask_coverage = content_mask_coverage(
        mul(input.local_position, sprite.transformation.rotation_scale)
            + sprite.transformation.translation,
        sprite.content_mask, sprite.content_mask_corner_radii,
        sprite.bounds.origin, sprite.bounds.size, sprite.content_mask_corner_radii,
        false);

    SubpixelSpriteFragmentOutput output;
    output.foreground = float4(input.color.rgb, 1.0f);
    output.alpha = float4(input.color.a * alpha_corrected * mask_coverage, 1.0f);
    return output;
}

/*
**
**              Polychrome sprites
**
*/

struct PolychromeSprite {
    uint order;
    uint pad;
    uint grayscale;
    float opacity;
    Bounds bounds;
    Bounds content_mask;
    Corners content_mask_corner_radii;
    Corners corner_radii;
    AtlasTile tile;
    TransformationMatrix transformation;
};

struct PolychromeSpriteVertexOutput {
    nointerpolation uint sprite_id: TEXCOORD0;
    float4 position: SV_Position;
    float2 tile_position: POSITION;
    float4 clip_distance: SV_ClipDistance;
    // Position in the sprite's untransformed (local) coordinate space, used by the
    // fragment shader for the corner-radius SDF so it isn't distorted by the transform.
    float2 local_position: TEXCOORD1;
};

struct PolychromeSpriteFragmentInput {
    nointerpolation uint sprite_id: TEXCOORD0;
    float4 position: SV_Position;
    float2 tile_position: POSITION;
    // Must be declared here too, in the same position as in `PolychromeSpriteVertexOutput`:
    // FXC assigns hardware input registers in declaration order, so a slot mismatch shifts
    // `local_position` to a different register than the VS emits it on and the draw fails to link.
    float2 local_position: TEXCOORD1;
};

StructuredBuffer<PolychromeSprite> poly_sprites: register(t1);

PolychromeSpriteVertexOutput polychrome_sprite_vertex(uint vertex_id: SV_VertexID, uint sprite_id: SV_InstanceID) {
    float2 unit_vertex = float2(float(vertex_id & 1u), 0.5 * float(vertex_id & 2u));
    PolychromeSprite sprite = poly_sprites[sprite_id];
    // Grow the sprite rect so its own corner-radius SDF and the mask's ramp have fragments
    // outside the outline (see `AA_MARGIN`). The atlas lookup stays clamped inside the
    // sprite's tile, so the added fragments — whose coverage is at most a fraction of a
    // pixel — replay the tile's edge texel instead of a neighbouring tile's.
    float2 margin = aa_margin_local(sprite.transformation.rotation_scale);
    float2 local_position = unit_vertex * (sprite.bounds.size + 2.0 * margin)
        + (sprite.bounds.origin - margin);
    float2 transformed_position =
        mul(local_position, sprite.transformation.rotation_scale) + sprite.transformation.translation;
    float4 device_position = to_device_position_impl(transformed_position);
    // A mask that coincides with the sprite's own bounds is the sprite's own cutout (see
    // `polychrome_sprite_fragment`): the sprite's local-space outline already bounds the
    // painted area, so clipping the straight edges against it would slice the transformed
    // sprite at its untransformed rect. Skip the clip in that case.
    bool mask_is_own = all(sprite.content_mask.origin == sprite.bounds.origin) &&
        all(sprite.content_mask.size == sprite.bounds.size);
    float4 clip_distance = mask_is_own
        ? float4(1.0, 1.0, 1.0, 1.0)
        : distance_from_clip_rect_aa_impl(transformed_position, sprite.content_mask);
    float2 tile_position = to_tile_position(saturate(unit_vertex), sprite.tile);

    PolychromeSpriteVertexOutput output;
    output.position = device_position;
    output.tile_position = tile_position;
    output.sprite_id = sprite_id;
    output.clip_distance = clip_distance;
    output.local_position = local_position;
    return output;
}

float4 polychrome_sprite_fragment(PolychromeSpriteFragmentInput input): SV_Target {
    PolychromeSprite sprite = poly_sprites[input.sprite_id];
    float4 sample = t_sprite.Sample(s_sprite, input.tile_position);
    float2 pixel_x = ddx(input.local_position);
    float2 pixel_y = ddy(input.local_position);
    float2 half_size = sprite.bounds.size / 2.0;
    float2 the_point = input.local_position - sprite.bounds.origin;
    float corner_radius = pick_corner_radius(the_point - half_size, sprite.corner_radii);

    // Rounded content-mask cutout: straight mask edges were already `SV_ClipDistance`-clipped
    // in `polychrome_sprite_vertex`; only the mask's rounded-corner arcs are cut here. A mask
    // that coincides with the sprite's own *untransformed* bounds and radii is the sprite's own
    // antialiased cutout — the own SDF below cuts that outline in local space (following any
    // transform), so the mask must not cut it again in scene space.
    float mask_coverage = content_mask_coverage(
        mul(input.local_position, sprite.transformation.rotation_scale)
            + sprite.transformation.translation,
        sprite.content_mask, sprite.content_mask_corner_radii,
        sprite.bounds.origin, sprite.bounds.size,
        sprite.corner_radii,
        true);

    float4 color = sample;
    if (sprite.grayscale != 0u) {
        float3 grayscale = dot(color.rgb, GRAYSCALE_FACTORS);
        color = float4(grayscale, sample.a);
    }
    color.a *= sprite.opacity * min(
        quad_own_coverage(the_point, 0.25 * pixel_x, 0.25 * pixel_y, half_size, corner_radius),
        mask_coverage);
    return color;
}

/*
**
**              Blur (backdrop-filter / filter)
**
** Shared by backdrop and content blur. The source texture is bound at t0 (t_sprite) and the
** parameters in the BlurParams constant buffer at b1. Three passes: downsample (full -> half
** res), separable gaussian (run twice), and a composite into a rounded rectangle.
*/

cbuffer BlurParams: register(b1) {
    // The composite quad (composite pass), or the element's box the source taps are mirrored
    // back into (downsample pass), in device pixels.
    Bounds blur_bounds;
    Bounds blur_content_mask;
    // The content mask's corner radii (tl, tr, br, bl), in device pixels: the clip the element
    // was painted under may itself be rounded (an ancestor's `overflow_hidden` + corner radius),
    // and the filter's output is clipped by it exactly like any other painting.
    float4 blur_content_mask_radii;
    float4 blur_corner_radii;
    float2 blur_direction;
    float blur_sigma;
    float blur_opacity;
    float blur_tap_count;
    float blur_clip_rounded;
    // 1.0 = snapped 2:1 box downsample (anchor the half-res grid to a fixed 2px grid at the origin
    // so a stationary element blurs identically at every window size); 0.0 = 1:1 copy (scene blit).
    float blur_downsample;
    // Spacing between taps in pixels (gaussian passes only); >1 lets `blur_tap_count` taps span
    // very large radii without truncating the gaussian.
    float blur_tap_step;
};

struct BlurVertexOutput {
    float4 position: SV_Position;
    float2 uv: TEXCOORD0;
};

// Reflect a sample back into `rect` (origin/size, in device pixels). Sampling outside an
// element's box mirrors its edge content instead of reading the transparent surround, which is
// what keeps a blurred element's own border crisp, opaque and even instead of dissolving into
// whatever is behind it — CSS `backdrop-filter`'s edge behaviour (Chrome mirrors since 129;
// `duplicate` smears the edge line instead), and what `filter: blur` needs to look like it in
// practice. Blurring with the surround would otherwise reach ~3 sigma *into* the element as well.
//
// The fold uses `floor`, whose behaviour is identical in HLSL, WGSL and MSL, unlike the
// `fmod`/`mod` pair: `fmod` keeps the sign of its dividend in HLSL/MSL, WGSL's `mod` does not.
float2 mirror_into_rect(float2 p, Bounds rect) {
    if (rect.size.x <= 0.0 || rect.size.y <= 0.0) {
        return p;
    }
    float2 t = (p - rect.origin) / rect.size;
    // Triangle wave of period 2, then fold into [0, 1]: t = 0.25 -> 0.25, t = 1.75 -> 0.25,
    // t = -0.25 -> 0.25.
    float2 m = t - 2.0 * floor(t * 0.5);
    return rect.origin + (1.0 - abs(m - 1.0)) * rect.size;
}

BlurVertexOutput blur_fullscreen(uint vertex_id) {
    float2 uv = float2(float((vertex_id << 1u) & 2u), float(vertex_id & 2u));
    BlurVertexOutput output;
    output.uv = uv;
    output.position = float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    return output;
}

BlurVertexOutput blur_downsample_vertex(uint vertex_id: SV_VertexID) {
    return blur_fullscreen(vertex_id);
}

float4 blur_downsample_fragment(BlurVertexOutput input): SV_Target {
    if (blur_downsample > 0.5) {
        // Snapped 2:1 box downsample. Half-res texel `px` samples source at full-res coordinate
        // 2*px + 1 (the boundary between source texels 2*px and 2*px+1), so one bilinear tap
        // averages exactly that pair. Anchored to the origin and independent of the viewport size,
        // so an element at fixed pixels blurs identically at every window size — otherwise the
        // implicit floor(W/2) grid stretches and the halo wobbles by ~1px on resize.
        uint sw, sh;
        t_sprite.GetDimensions(sw, sh);
        // The taps that would leave the element's box read its mirrored edge instead of the
        // transparent surround (`blur_bounds` in this pass is that box, not the composite rect).
        float2 src_px = mirror_into_rect(floor(input.position.xy) * 2.0 + 1.0, blur_bounds);
        float2 src_uv = src_px / float2(sw, sh);
        return t_sprite.SampleLevel(s_sprite, src_uv, 0.0);
    }
    // 1:1 copy at matching resolution (used to blit the offscreen scene into the swapchain).
    return t_sprite.SampleLevel(s_sprite, input.uv, 0.0);
}

BlurVertexOutput blur_vertex(uint vertex_id: SV_VertexID) {
    return blur_fullscreen(vertex_id);
}

float4 blur_fragment(BlurVertexOutput input): SV_Target {
    int taps = int(blur_tap_count);
    float4 color = float4(0.0, 0.0, 0.0, 0.0);
    float weight_sum = 0.0;
    [loop]
    for (int i = -taps; i <= taps; i++) {
        float offset = float(i) * blur_tap_step;
        float weight = gaussian(offset, blur_sigma);
        color += t_sprite.SampleLevel(s_sprite, input.uv + blur_direction * offset, 0.0) * weight;
        weight_sum += weight;
    }
    return color / max(weight_sum, 1e-5);
}

struct BlurCompositeVertexOutput {
    float4 position: SV_Position;
    float4 clip_distance: SV_ClipDistance;
};

struct BlurCompositeFragmentInput {
    float4 position: SV_Position;
};

BlurCompositeVertexOutput blur_composite_vertex(uint vertex_id: SV_VertexID) {
    float2 unit_vertex = float2(float(vertex_id & 1u), 0.5 * float(vertex_id & 2u));
    // Grow the composite quad so the rounded cut below has fragments on both sides of the
    // outline (see `AA_MARGIN`). The fragment samples the blur by *screen position*, so the
    // extra area needs no uv compensation, and `blur_bounds` itself is left alone — it is what
    // the cut measures from.
    Bounds bounds;
    bounds.origin = blur_bounds.origin - AA_MARGIN;
    bounds.size = blur_bounds.size + 2.0 * AA_MARGIN;
    BlurCompositeVertexOutput output;
    output.position = to_device_position(unit_vertex, bounds);
    output.clip_distance = distance_from_clip_rect(unit_vertex, bounds, blur_content_mask);
    return output;
}

float4 blur_composite_fragment(BlurCompositeFragmentInput input): SV_Target {
    // Sample the half-res blur by screen position, on the SAME fixed 2:1 grid the snapped downsample
    // wrote (anchored at the origin, independent of viewport parity): 2 * the half-res texture size
    // maps screen pixel p to half-res texel p/2 at every window size, so it doesn't wobble on resize.
    uint hw, hh;
    t_sprite.GetDimensions(hw, hh);
    float2 uv = input.position.xy / (2.0 * float2(hw, hh));
    float4 blurred = t_sprite.SampleLevel(s_sprite, uv, 0.0);
    Corners radii;
    radii.top_left = blur_corner_radii.x;
    radii.top_right = blur_corner_radii.y;
    radii.bottom_right = blur_corner_radii.z;
    radii.bottom_left = blur_corner_radii.w;
    float distance = quad_sdf(input.position.xy, blur_bounds, radii);
    // Backdrop clips to the rounded rect (the panel has a defined shape); content blur bleeds past
    // its bounds like CSS `filter: blur`, so its shape comes from the blurred group's own alpha.
    float coverage = blur_clip_rounded > 0.5 ? saturate(0.5 - distance) : 1.0;
    // The clip the element was painted under is part of that shape too: a rounded `overflow_hidden`
    // ancestor clips the filter's output like anything else it contains, corners included.
    Corners mask_radii;
    mask_radii.top_left = blur_content_mask_radii.x;
    mask_radii.top_right = blur_content_mask_radii.y;
    mask_radii.bottom_right = blur_content_mask_radii.z;
    mask_radii.bottom_left = blur_content_mask_radii.w;
    float mask_coverage =
        saturate(0.5 - quad_sdf(input.position.xy, blur_content_mask, mask_radii));
    // The blurred sample is premultiplied (blurring against the transparent surround scales rgb
    // with the fading alpha), so output premultiplied and use a premultiplied-blend state. A
    // backdrop's scene is opaque (so this replaces); a content-filter group is transparent outside
    // its subtree (so the target shows through there instead of darkening).
    float a = min(coverage, mask_coverage) * blur_opacity;
    return float4(blurred.rgb * a, blurred.a * a);
}
