// --- subpixel sprites --- //

struct SubpixelSprite {
    order: u32,
    pad: u32,
    bounds: Bounds,
    content_mask: Bounds,
    content_mask_corner_radii: Corners,
    color: Hsla,
    tile: AtlasTile,
    transformation: TransformationMatrix,
}
@group(1) @binding(0) var<storage, read> b_subpixel_sprites: array<SubpixelSprite>;

struct SubpixelSpriteOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tile_position: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
    @location(3) clip_distances: vec4<f32>,
    // Position in the sprite's untransformed (local) coordinate space, used by the fragment
    // shader for the content-mask coverage so it isn't distorted by the transform.
    @location(2) local_position: vec2<f32>,
    @location(4) @interpolate(flat) sprite_id: u32,
}

struct SubpixelSpriteFragmentOutput {
    @location(0) @blend_src(0) foreground: vec4<f32>,
    @location(0) @blend_src(1) alpha: vec4<f32>,
}

@vertex
fn vs_subpixel_sprite(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> SubpixelSpriteOutput {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let sprite = b_subpixel_sprites[instance_id];

    var out = SubpixelSpriteOutput();
    // Exactly the geometry `vs_mono_sprite` uses (this is its subpixel twin), including its
    // deliberate lack of an `AA_MARGIN` expansion: a glyph's shape is its texture's alpha, so
    // there is nothing here for extra geometry to taper (see the note in `vs_mono_sprite`).
    let local_position = unit_vertex * vec2<f32>(sprite.bounds.size) + sprite.bounds.origin;
    let transformed_position =
        transpose(sprite.transformation.rotation_scale) * local_position +
        sprite.transformation.translation;
    out.position = to_device_position_impl(transformed_position);
    out.tile_position = to_tile_position(unit_vertex, sprite.tile);
    out.color = hsla_to_rgba(sprite.color);
    out.sprite_id = instance_id;
    out.local_position = local_position;
    out.clip_distances = distance_from_clip_rect_impl(transformed_position, sprite.content_mask);
    return out;
}

@fragment
fn fs_subpixel_sprite(input: SubpixelSpriteOutput) -> SubpixelSpriteFragmentOutput {
    var sample = textureSample(t_sprite, s_sprite, input.tile_position).rgb;
    if (gamma_params.is_bgr != 0u) {
        sample = sample.bgr;
    }
    let alpha_corrected = apply_contrast_and_gamma_correction3(sample, input.color.rgb, gamma_params.subpixel_enhanced_contrast, gamma_params.gamma_ratios);

    // Alpha clip after using the derivatives.
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return SubpixelSpriteFragmentOutput(vec4<f32>(0.0), vec4<f32>(0.0));
    }

    let sprite = b_subpixel_sprites[input.sprite_id];
    // Rounded content-mask cutout: the vertex shader clipped the mask's straight edges, so only
    // its rounded-corner arcs are cut here. Subpixel sprites carry no own rounded cutout in the
    // shader, so there is no mask-equals-own skip; the coverage cuts the dual-source alpha
    // output (the foreground color output stays untouched).
    let mask_coverage = content_mask_coverage(
        transpose(sprite.transformation.rotation_scale) * input.local_position
            + sprite.transformation.translation,
        sprite.content_mask, sprite.content_mask_corner_radii,
        sprite.bounds.origin, sprite.bounds.size, sprite.content_mask_corner_radii,
        false);

    var out = SubpixelSpriteFragmentOutput();
    out.foreground = vec4<f32>(input.color.rgb, 1.0);
    out.alpha = vec4<f32>(input.color.a * alpha_corrected * mask_coverage, 1.0);
    return out;
}
