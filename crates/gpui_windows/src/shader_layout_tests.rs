//! HLSL struct-layout probe for the DirectX backend.
//!
//! Compiles the real `shaders.hlsl` entry points with the actual runtime compiler
//! (D3DCompileFromFile — the same path debug builds use) and dumps the structured-buffer
//! `ld_structured` byte offsets from the disassembly. This pins down how FXC packs the
//! GPU-side `Quad`/`Shadow` structs (matrices and vectors inside structured buffers pack
//! differently than in cbuffers) so we can verify they match the Rust `repr(C)` layouts:
//!
//! - `Quad`: 184 bytes; `transformation.rotation_scale` at offset 160, `translation` at 176
//! - `Shadow`: 112 bytes, `element_corner_radii` ending at 112
//!
//! Any `ld_structured` byteOffset for `transformation` other than 160/176 means the Rust
//! and HLSL layouts disagree and every quad except element 0 reads garbage transforms.

#![cfg(all(test, not(feature = "wgpu")))]

use anyhow::Result;

use windows::core::{HSTRING, PCSTR};
use windows::Win32::Graphics::Direct3D::{
    Fxc::{D3DCOMPILE_DEBUG, D3DCOMPILE_SKIP_OPTIMIZATION, D3DCompileFromFile, D3DDisassemble},
    ID3DBlob,
};

fn compile(entry: &str, target: &str) -> Result<ID3DBlob> {
    unsafe {
        use windows::Win32::Graphics::{
            Direct3D::ID3DInclude, Hlsl::D3D_COMPILE_STANDARD_FILE_INCLUDE,
        };

        let shader_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/shaders.hlsl")
            .canonicalize()?;

        let entry_c = format!("{entry}\0");
        let target_c = format!("{target}\0");
        let include_handler =
            &std::mem::transmute::<usize, ID3DInclude>(D3D_COMPILE_STANDARD_FILE_INCLUDE as usize);

        let mut blob = None;
        let mut error_blob = None;
        let ret = D3DCompileFromFile(
            &HSTRING::from(shader_path.to_str().unwrap()),
            None,
            include_handler,
            PCSTR::from_raw(entry_c.as_ptr()),
            PCSTR::from_raw(target_c.as_ptr()),
            D3DCOMPILE_DEBUG | D3DCOMPILE_SKIP_OPTIMIZATION,
            0,
            &mut blob,
            Some(&mut error_blob),
        );
        if ret.is_err() {
            if let Some(error_blob) = error_blob {
                let err = std::ffi::CStr::from_ptr(error_blob.GetBufferPointer() as *const i8)
                    .to_string_lossy();
                eprintln!("[shader_layout] {entry} compile error: {err}");
                anyhow::bail!("shader compile failed: {err}");
            }
            ret?;
        } else if let Some(warning_blob) = error_blob {
            let warn =
                std::ffi::CStr::from_ptr(warning_blob.GetBufferPointer() as *const i8).to_string_lossy();
            eprintln!("[shader_layout] {entry} warnings:\n{warn}");
        }
        Ok(blob.unwrap())
    }
}

fn disassemble(blob: &ID3DBlob) -> Result<String> {
    unsafe {
        let out = D3DDisassemble(blob.GetBufferPointer(), blob.GetBufferSize(), 0, PCSTR::null())?;
        let bytes = std::slice::from_raw_parts(out.GetBufferPointer() as *const u8, out.GetBufferSize());
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }
}

/// Prints the full `// Signature:` block of each compiled entry point. The runtime draw fails
/// with "Semantic 'TEXCOORD' is defined for mismatched hardware registers between the output
/// stage and input stage" — the signature blocks show exactly which register FXC assigned
/// each semantic in VS vs PS.
#[test]
fn hlsl_quad_signatures_probe() -> Result<()> {
    for (entry, target) in [("quad_vertex", "vs_4_1"), ("quad_fragment", "ps_4_1")] {
        let blob = compile(entry, target)?;
        let asm = disassemble(&blob)?;
        println!("===== {entry} ({target}) =====");
        // Everything until the first instruction line (`xxx:`) is comments + signature.
        for line in asm.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") {
                println!("{trimmed}");
            } else {
                break;
            }
        }
    }
    Ok(())
}

#[test]
fn hlsl_quad_layout_probe() -> Result<()> {
    for (entry, target) in [
        ("quad_vertex", "vs_4_1"),
        ("quad_fragment", "ps_4_1"),
        ("shadow_vertex", "vs_4_1"),
    ] {
        let blob = compile(entry, target)?;
        let asm = disassemble(&blob)?;
        println!("===== {entry} ({target}) =====");
        for line in asm.lines() {
            let line = line.trim();
            if line.contains("ld_structured") && line.contains("t1") {
                println!("  {line}");
            }
        }
    }
    Ok(())
}
