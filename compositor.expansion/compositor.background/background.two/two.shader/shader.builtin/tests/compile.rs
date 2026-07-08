//! Guards for the built-in background shaders. Their WGSL is only compiled at
//! runtime (when a world is selected, via `shader.load` → `build_wgsl`), so
//! `cargo build` can't catch a typo in a shader. These tests exercise the same
//! naga WGSL→SPIR-V path up front, so a broken or contract-violating built-in
//! fails `cargo test` instead of at first GPU use.

use compositor_background_two_shader_builtin::{builtins, props};
use compositor_background_two_shader_spirv::build_wgsl;

/// Every registered built-in must parse, validate, and expose both entry points.
#[test]
fn all_builtins_compile() {
    for (i, b) in builtins().iter().enumerate() {
        let m = build_wgsl(b.wgsl, i as u64)
            .unwrap_or_else(|e| panic!("built-in {} failed to compile: {e}", b.id));
        assert_eq!(m.vert_entry, "vs_main", "{}: vertex entry", b.id);
        assert_eq!(m.frag_entry, "fs_main", "{}: fragment entry", b.id);
    }
}

/// `params[3]` (slots 12..16) is reserved as the sprite-sheet control vec4, so no
/// built-in may declare more than 12 `@prop`s — the 13th would land on slot 12
/// and collide with the reserved atlas controls (see the header block in each
/// shader, and `background-shader-texture-slot`).
#[test]
fn params_slot_3_reserved_for_sprite_sheet() {
    for b in builtins() {
        let n = props(b.id).map_or(0, |p| p.len());
        assert!(n <= 12, "built-in {} declares {n} props; slots 12..16 are reserved", b.id);
    }
}
