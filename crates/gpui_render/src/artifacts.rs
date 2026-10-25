//! Shader artifacts generated at Cargo build time from the Rust-authored shader modules.

pub const BASE_WGSL: &str = include_str!(concat!(env!("OUT_DIR"), "/base.wgsl"));
pub const SUBPIXEL_WGSL: &str = include_str!(concat!(env!("OUT_DIR"), "/subpixel.wgsl"));

/// D3D11 bytecode generated from HLSL at build time.
#[derive(Clone, Copy)]
pub struct Dx11Bytecode {
    pub vertex: &'static [u8],
    pub fragment: &'static [u8],
    /// Present for every pipeline whose vertex shader reads `instance_index`.
    ///
    /// Direct3D 11 does not add `StartInstanceLocation` to `SV_InstanceID`, so the
    /// generated vertex shader adds this constant itself. The renderer must upload a
    /// [`Dx11DrawConstants`] to this register before every draw.
    pub draw_constants_register: Option<u32>,
}

/// The per-draw cbuffer Naga's HLSL backend declares as `NagaConstants`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Dx11DrawConstants {
    pub first_vertex: i32,
    pub first_instance: i32,
    pub other: u32,
    pub padding: u32,
}

impl Dx11DrawConstants {
    pub fn for_instances(first_instance: u32) -> Self {
        Self {
            first_vertex: 0,
            first_instance: i32::try_from(first_instance)
                .expect("instance bases are bounded by the D3D11 buffer limit"),
            other: 0,
            padding: 0,
        }
    }
}

const _: () = assert!(std::mem::size_of::<Dx11DrawConstants>() == 16);

#[derive(Clone, Copy)]
pub enum Dx11Shader {
    Sm50(Dx11Bytecode),
    NativeWindowsBuildRequired,
}

pub struct NativeShader {
    pub label: &'static str,
    pub vertex_entry: &'static str,
    pub fragment_entry: &'static str,
    pub pipeline: &'static crate::shaders::interface::Pipeline,
    pub dx11: Dx11Shader,
    pub msl: &'static str,
    pub hlsl: &'static str,
}

impl NativeShader {
    pub fn dx11(&self) -> Dx11Bytecode {
        match self.dx11 {
            Dx11Shader::Sm50(bytecode) => bytecode,
            Dx11Shader::NativeWindowsBuildRequired => panic!(
                "{} has no DX11 bytecode: build the Windows target on a Windows host",
                self.label
            ),
        }
    }
}

include!(concat!(env!("OUT_DIR"), "/native_shaders.rs"));

#[cfg(test)]
mod tests {
    use super::{Dx11Shader, NATIVE_SHADERS};
    use crate::shaders::interface;

    /// Every declared pipeline, including the smoothed variants, must have been
    /// generated and compiled by Naga (WGSL/MSL) and FXC (DXBC) at build time.
    #[test]
    fn every_pipeline_has_native_artifacts() {
        let mut pipeline_count = 0;
        for pipeline in interface::ALL {
            pipeline_count += 1;
            let shader = NATIVE_SHADERS
                .iter()
                .find(|shader| shader.label == pipeline.label)
                .unwrap_or_else(|| panic!("missing native shader for {}", pipeline.label));
            assert_eq!(shader.vertex_entry, pipeline.vertex_entry);
            assert_eq!(shader.fragment_entry, pipeline.fragment_entry);
            assert!(!shader.msl.is_empty(), "{} has empty MSL", pipeline.label);
            #[cfg(windows)]
            assert!(
                matches!(shader.dx11, Dx11Shader::Sm50(_)),
                "{} has no FXC bytecode",
                pipeline.label
            );
        }
        let emoji = NATIVE_SHADERS
            .iter()
            .find(|shader| shader.label == interface::EMOJI_RASTERIZATION.label)
            .expect("missing emoji shader");
        assert!(!emoji.msl.is_empty());
        #[cfg(windows)]
        assert!(matches!(emoji.dx11, Dx11Shader::Sm50(_)));
        assert_eq!(
            pipeline_count,
            interface::ALL.len(),
            "pipeline table and native shader table must stay in lockstep"
        );
    }

    fn registers(source: &str, class: char) -> Vec<u32> {
        let needle = format!("register({class}");
        source
            .match_indices(&needle)
            .filter_map(|(index, _)| {
                let rest = &source[index + needle.len()..];
                let end = rest.find(')')?;
                rest[..end].parse().ok()
            })
            .collect()
    }

    /// The frame-wide content-mask arena must use one dedicated register/slot in every native
    /// dialect. A collision here is a runtime binding bug that FXC/WGSL validation alone does not
    /// catch (two resources would silently share a register).
    #[test]
    fn content_mask_arena_has_a_dedicated_native_binding() {
        for shader in NATIVE_SHADERS {
            if shader.label == interface::EMOJI_RASTERIZATION.label {
                continue;
            }
            assert!(
                shader.hlsl.contains("CONTENT_MASKS : register(t5)"),
                "{} does not bind CONTENT_MASKS at t5",
                shader.label
            );
            let texture_registers = registers(shader.hlsl, 't');
            assert_eq!(
                texture_registers.iter().filter(|&&register| register == 5).count(),
                1,
                "{} has a t5 collision: {texture_registers:?}",
                shader.label
            );
            for class in ['b', 's'] {
                let mut seen = registers(shader.hlsl, class);
                seen.sort_unstable();
                seen.dedup();
                let mut all = registers(shader.hlsl, class);
                all.sort_unstable();
                assert_eq!(all, seen, "{} has duplicate {} registers", shader.label, class);
            }

            // MSL only declares resources the entry points actually use (naga prunes unused
            // ones, e.g. the path-sprite blit), so presence is conditional. What must never
            // happen is another resource occupying buffer(4): that would silently alias the
            // mask arena wherever both are used.
            let mask_bindings = shader.msl.matches("CONTENT_MASKS [[buffer(4)]]").count();
            let buffer_four_bindings = shader.msl.matches("[[buffer(4)]]").count();
            assert_eq!(
                mask_bindings, buffer_four_bindings,
                "{} has an MSL buffer(4) collision",
                shader.label
            );
        }
    }
}
