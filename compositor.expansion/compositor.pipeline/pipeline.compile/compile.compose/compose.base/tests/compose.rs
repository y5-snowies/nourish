//! naga_oil composition: `#import` a shared module, pair a fullscreen vertex for
//! fragment-only passes, and select a variant with a shader-def.

use compositor_pipeline_compile_compose_base::compose::{compose_wgsl, Module};
use std::collections::BTreeMap;

const NOISE: &str = r#"
#define_import_path noise
fn value(uv: vec2<f32>) -> f32 {
    return fract(sin(dot(uv, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}
"#;

const PASS: &str = r#"
#import noise::value
struct VsOut { @builtin(position) pos: vec4<f32> };
@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    let uv = vec2<f32>(f32((vid << 1u) & 2u), f32(vid & 2u));
    var o: VsOut;
    o.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return o;
}
@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let n = value(frag.xy * 0.01);
    return vec4<f32>(n, n, n, 1.0);
}
"#;

#[test]
fn composes_imported_module() {
    let modules = [Module { source: NOISE, file_path: "lib/noise.wgsl" }];
    let m = compose_wgsl(PASS, "passes/base.wgsl", &modules, &BTreeMap::new(), 42)
        .expect("compose ok");
    assert_eq!(m.id, 42);
    assert!(!m.spv.is_empty());
    assert!(m.vert_spv.is_none()); // source carried its own vs_main
    assert_eq!(&*m.frag_entry, "fs_main");
}

#[test]
fn fragment_only_pairs_fullscreen_vertex() {
    const FRAG: &str = r#"
@fragment
fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }
"#;
    let m = compose_wgsl(FRAG, "passes/solid.wgsl", &[], &BTreeMap::new(), 1).expect("ok");
    assert!(m.vert_spv.is_some()); // fullscreen vertex paired in
}

#[test]
fn shader_def_selects_variant() {
    const V: &str = r#"
@fragment
fn fs_main() -> @location(0) vec4<f32> {
#ifdef HORIZONTAL
    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
#else
    return vec4<f32>(0.0, 1.0, 0.0, 1.0);
#endif
}
"#;
    let mut defs = BTreeMap::new();
    defs.insert("HORIZONTAL".to_string(), "true".to_string());
    assert!(compose_wgsl(V, "v.wgsl", &[], &defs, 1).is_ok());
    // And the other branch compiles too, with no def set.
    assert!(compose_wgsl(V, "v.wgsl", &[], &BTreeMap::new(), 2).is_ok());
}
