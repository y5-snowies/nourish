//! Ahead-of-time compilation of the Phase-0 passthrough shader (WGSL → SPIR-V
//! via naga), the same mechanism `pipeline.composite`/`pipeline.hdr` use.

use std::path::Path;

fn main() {
    let src_path = "shaders/passthrough.wgsl";
    println!("cargo:rerun-if-changed={src_path}");
    let src = std::fs::read_to_string(src_path).unwrap_or_else(|e| panic!("read {src_path}: {e}"));
    let module =
        naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("passthrough parse: {e:?}"));
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("passthrough validate: {e:?}"));
    // Geometry is authored directly in Vulkan clip space — disable the WGSL→API
    // Y-flip, matching aa.wgsl / the runtime shader loader.
    let mut opts = naga::back::spv::Options::default();
    opts.flags
        .remove(naga::back::spv::WriterFlags::ADJUST_COORDINATE_SPACE);
    let words = naga::back::spv::write_vec(&module, &info, &opts, None)
        .unwrap_or_else(|e| panic!("passthrough spv: {e:?}"));
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("passthrough.spv");
    std::fs::write(out, bytes).unwrap_or_else(|e| panic!("write passthrough.spv: {e}"));
}
