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
}
