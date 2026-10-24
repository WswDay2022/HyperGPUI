#[wgsl_rs::wgsl]
pub mod surface {
    use super::super::common::*;
    use wgsl_rs::std::*;

    #[repr(C)]
    #[derive(Clone, Copy, Wgsl)]
    pub struct SurfaceUniforms {
        pub bounds: Bounds,
        pub content_mask: ContentMask,
        pub color_format: SurfaceColorFormat,
        pub opacity: f32,
        pub padding0: u32,
        pub padding1: u32,
        pub padding2: u32,
        pub padding3: u32,
        pub padding4: u32,
        pub padding5: u32,
    }
    uniform!(group(1), binding(0), SURFACE_LOCALS: SurfaceUniforms);
    texture!(group(1), binding(1), SURFACE_TEXTURE: Texture2D<f32>);
    texture!(group(1), binding(2), SURFACE_CHROMA_TEXTURE: Texture2D<f32>);
    sampler!(group(1), binding(3), SURFACE_SAMPLER: Sampler);

    pub const YCBCR_TO_LINEAR_RGB: Mat4x4f = mat4x4f(
        vec4f(1.0000, 1.0000, 1.0000, 0.0),
        vec4f(0.0000, -0.3441, 1.7720, 0.0),
        vec4f(1.4020, -0.7141, 0.0000, 0.0),
        vec4f(-0.7010, 0.5291, -0.8860, 1.0),
    );

    pub fn sample_yuv_surface(texture_position: Vec2f) -> Vec4f {
        let luma =
            texture_sample_level(SURFACE_TEXTURE, SURFACE_SAMPLER, texture_position, 0.0).x;
        let chroma = texture_sample_level(
            SURFACE_CHROMA_TEXTURE,
            SURFACE_SAMPLER,
            texture_position,
            0.0,
        )
        .xy();
        YCBCR_TO_LINEAR_RGB * vec4f(luma, chroma.x, chroma.y, 1.0)
    }

    #[derive(Wgsl)]
    pub struct SurfaceVarying {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub texture_position: Vec2f,
        #[location(3)]
        pub clip_distances: Vec4f,
    }

    #[vertex]
    pub fn surface_vertex(#[builtin(vertex_index)] vertex_id: u32) -> SurfaceVarying {
        let unit_vertex = rectangle_corner(vertex_id);
        let locals = get!(SURFACE_LOCALS);
        SurfaceVarying {
            position: to_device_position(unit_vertex, locals.bounds),
            texture_position: unit_vertex,
            clip_distances: distance_from_clip_rect(
                unit_vertex,
                locals.bounds,
                locals.content_mask.bounds,
            ),
        }
    }

    #[fragment]
    pub fn surface_fragment(input: SurfaceVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let locals = get!(SURFACE_LOCALS);
        let mut color = transparent();
        if locals.color_format == SurfaceColorFormat::Yuv {
            color = sample_yuv_surface(input.texture_position);
        } else {
            color = texture_sample_level(
                SURFACE_TEXTURE,
                SURFACE_SAMPLER,
                input.texture_position,
                0.0,
            );
        }
        let mask_coverage = content_mask_coverage(
            input.position.xy(),
            locals.content_mask.bounds,
            locals.content_mask.corner_radii,
            locals.content_mask.fade_out,
            vec2f(0.0, 0.0),
            vec2f(0.0, 0.0),
            locals.content_mask.corner_radii,
            false,
        );
        color * locals.opacity * mask_coverage
    }
}

#[wgsl_rs::wgsl]
pub mod blur {
    use super::super::common::*;
    use wgsl_rs::std::*;

    #[repr(C)]
    #[derive(Clone, Copy, Wgsl)]
    pub struct BlurUniforms {
        pub bounds: Bounds,
        pub content_mask: Bounds,
        pub content_mask_radii: Vec4f,
        pub content_mask_fade_out: Vec4f,
        pub corner_radii: Vec4f,
        pub direction: Vec2f,
        pub sigma: f32,
        pub opacity: f32,
        pub tap_count: f32,
        pub tap_step: f32,
        pub clip_rounded: f32,
        pub downsample: f32,
    }
    uniform!(group(1), binding(0), BLUR_LOCALS: BlurUniforms);
    texture!(group(1), binding(1), BLUR_TEXTURE: Texture2D<f32>);
    sampler!(group(1), binding(2), BLUR_SAMPLER: Sampler);

    #[derive(Wgsl)]
    pub struct BlurVarying {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub uv: Vec2f,
        #[location(3)]
        pub clip_distances: Vec4f,
    }

    pub fn blur_fullscreen_uv(vertex_id: u32) -> Vec2f {
        vec2f(
            ((vertex_id << 1u32) & 2u32) as f32,
            (vertex_id & 2u32) as f32,
        )
    }

    #[vertex]
    pub fn blur_downsample_vertex(#[builtin(vertex_index)] vertex_id: u32) -> BlurVarying {
        let uv = blur_fullscreen_uv(vertex_id);
        BlurVarying {
            position: vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0),
            uv,
            clip_distances: unclipped_distances(),
        }
    }

    #[vertex]
    pub fn blur_vertex(#[builtin(vertex_index)] vertex_id: u32) -> BlurVarying {
        let uv = blur_fullscreen_uv(vertex_id);
        BlurVarying {
            position: vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0),
            uv,
            clip_distances: unclipped_distances(),
        }
    }

    #[fragment]
    pub fn blur_downsample_fragment(input: BlurVarying) -> Vec4f {
        if get!(BLUR_LOCALS).downsample > 0.5 {
            let dst = floor(input.position.xy());
            let src_px = mirror_into_rect(dst * 2.0 + 1.0, get!(BLUR_LOCALS).bounds);
            let src_uv = src_px / get!(GLOBALS).viewport_size;
            return texture_sample_level(BLUR_TEXTURE, BLUR_SAMPLER, src_uv, 0.0);
        }
        texture_sample_level(BLUR_TEXTURE, BLUR_SAMPLER, input.uv, 0.0)
    }

    #[fragment]
    pub fn blur_fragment(input: BlurVarying) -> Vec4f {
        let sigma = get!(BLUR_LOCALS).sigma;
        let taps = get!(BLUR_LOCALS).tap_count as i32;
        let step_size = get!(BLUR_LOCALS).tap_step;
        let mut color = transparent();
        let mut weight_sum = 0.0;
        let mut i = -taps;
        while i <= taps {
            let offset = i as f32 * step_size;
            let weight = gaussian(offset, sigma);
            let uv = input.uv + get!(BLUR_LOCALS).direction * offset;
            color = color + texture_sample_level(BLUR_TEXTURE, BLUR_SAMPLER, uv, 0.0) * weight;
            weight_sum += weight;
            i += 1;
        }
        color / max(weight_sum, 1e-5)
    }

    #[vertex]
    pub fn blur_composite_vertex(#[builtin(vertex_index)] vertex_id: u32) -> BlurVarying {
        let unit_vertex = rectangle_corner(vertex_id);
        let mut geometry = get!(BLUR_LOCALS).bounds;
        if get!(BLUR_LOCALS).clip_rounded > 0.5 {
            geometry.origin = geometry.origin - vec2f(AA_MARGIN, AA_MARGIN);
            geometry.size = geometry.size + 2.0 * vec2f(AA_MARGIN, AA_MARGIN);
        }
        BlurVarying {
            position: to_device_position(unit_vertex, geometry),
            uv: unit_vertex,
            clip_distances: distance_from_clip_rect_aa(
                unit_vertex,
                geometry,
                get!(BLUR_LOCALS).content_mask,
            ),
        }
    }

    #[fragment]
    pub fn blur_composite_fragment(input: BlurVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }

        // The half-resolution blur texture covers the same window as the scene, so the
        // full-resolution position maps to it through the viewport size. Rounding the span down
        // to an even size (the old `2 * floor(viewport / 2)`) stretches the blur by up to one
        // pixel on odd-sized windows, which shows up as the blurred backdrop sliding relative to
        // the sharp content painted over it.
        let uv = input.position.xy() / get!(GLOBALS).viewport_size;
        let blurred = texture_sample_level(BLUR_TEXTURE, BLUR_SAMPLER, uv, 0.0);

        let corner_radii = Corners {
            top_left: get!(BLUR_LOCALS).corner_radii.x,
            top_right: get!(BLUR_LOCALS).corner_radii.y,
            bottom_right: get!(BLUR_LOCALS).corner_radii.z,
            bottom_left: get!(BLUR_LOCALS).corner_radii.w,
        };
        let distance = quad_sdf(input.position.xy(), get!(BLUR_LOCALS).bounds, corner_radii);
        let coverage = select(
            1.0,
            saturate(0.5 - distance),
            get!(BLUR_LOCALS).clip_rounded > 0.5,
        );

        let mask_radii = Corners {
            top_left: get!(BLUR_LOCALS).content_mask_radii.x,
            top_right: get!(BLUR_LOCALS).content_mask_radii.y,
            bottom_right: get!(BLUR_LOCALS).content_mask_radii.z,
            bottom_left: get!(BLUR_LOCALS).content_mask_radii.w,
        };
        let mask_fade_out = Edges {
            top: get!(BLUR_LOCALS).content_mask_fade_out.x,
            right: get!(BLUR_LOCALS).content_mask_fade_out.y,
            bottom: get!(BLUR_LOCALS).content_mask_fade_out.z,
            left: get!(BLUR_LOCALS).content_mask_fade_out.w,
        };
        let mask_coverage = saturate(
            0.5 - quad_sdf(
                input.position.xy(),
                get!(BLUR_LOCALS).content_mask,
                mask_radii,
            ),
        ) * content_mask_fade(
            get!(BLUR_LOCALS).content_mask,
            mask_fade_out,
            input.position.xy(),
        );

        let c = min(coverage, mask_coverage) * get!(BLUR_LOCALS).opacity;
        vec4f(blurred.x * c, blurred.y * c, blurred.z * c, blurred.w * c)
    }
}
