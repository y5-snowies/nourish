//! Ahead-of-time parallax shader compilation.
//!
//! Compiles the WGSL parallax shaders to SPIR-V via naga at build time; `vulkan.rs`
//! embeds the results with `include_bytes!`. Both modules expose `vs_main` +
//! `fs_main`:
//! - parallax.wgsl:           SDR parallax background
//! - parallax_hdr.wgsl:       HDR-graded (BT.2020 + PQ) parallax background
//! - parallax_optimized.wgsl: the cheap SDR variant, for the per-world
//!   "Optimized" toggle — same scene and same 112-byte push, far less noise

use std::path::Path;

fn compile(name: &str) {
    let src_path = format!("shaders/{name}.wgsl");
    println!("cargo:rerun-if-changed={src_path}");
    let src = std::fs::read_to_string(&src_path).unwrap_or_else(|e| panic!("read {src_path}: {e}"));
    let module = naga::front::wgsl::parse_str(&src)
        .unwrap_or_else(|e| panic!("{name}.wgsl parse: {e:?}"));
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{name}.wgsl validate: {e:?}"));
    // Our WGSL geometry is authored in Vulkan's clip space directly (ported from
    // the GLSL shaders), so disable naga's WGSL→API Y-flip — otherwise the pass
    // renders upside down.
    let mut opts = naga::back::spv::Options::default();
    opts.flags
        .remove(naga::back::spv::WriterFlags::ADJUST_COORDINATE_SPACE);
    let words = naga::back::spv::write_vec(&module, &info, &opts, None)
        .unwrap_or_else(|e| panic!("{name}.wgsl spv: {e:?}"));
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join(format!("{name}.spv"));
    std::fs::write(out, bytes).unwrap_or_else(|e| panic!("write {name}.spv: {e}"));
}

fn main() {
    compile("parallax");
    compile("parallax_hdr");
    compile("parallax_optimized");
}
