//! Ahead-of-time compilation of the world anti-aliasing shader.
//!
//! `aa.wgsl` (the anti-aliasing composite: sampler-based aniso/trilinear +
//! in-shader N×N supersample) is compiled to SPIR-V by naga at build time, the
//! same mechanism `pipeline.hdr` uses — no external glslang/glslc. The plain
//! composite (`quad.vert`/`tex.frag`/`solid.frag`) stays checked-in glslang SPIR-V.

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
    // The WGSL geometry is authored directly in Vulkan clip space (ported from
    // the GLSL quad), so disable naga's WGSL→API Y-flip — otherwise every AA
    // quad renders upside down.
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
    compile("aa");
}
