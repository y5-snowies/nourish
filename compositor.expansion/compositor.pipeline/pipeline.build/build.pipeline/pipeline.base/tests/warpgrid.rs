//! The generated grid pass must land on the SAME nodes the CPU baker does.
//!
//! Two producers fill one grid and one indexer reads it, so a disagreement about
//! where cell `(x, y)` lives is not a rounding difference — it is a pointer that
//! drifts further the further it gets from centre, and only under one of the two
//! `evaluate` modes. That is close to unfindable by eye, so it is pinned here.

use compositor_pipeline_build_pipeline_base::pipeline::{animated_source, warp_pass_src};

/// `Map::sample` places node `x` at `x / (EDGE - 1)`; a fragment centre sits at
/// `px + 0.5`. The generated pass must undo that, or every cell is stored half a
/// texel off and scaled by `EDGE / (EDGE - 1)`.
#[test]
fn the_grid_pass_samples_at_node_positions() {
    let src = warp_pass_src("demo::warp", false);
    assert!(
        src.contains("(frag.xy - vec2<f32>(0.5)) / vec2<f32>(128.0 - 1.0)"),
        "grid pass must map fragment centres onto baker nodes:\n{src}"
    );
    // The naive form is the bug; make sure it cannot come back unnoticed.
    assert!(!src.contains("frag.xy / vec2<f32>(128.0, 128.0)"), "{src}");
}

/// `res` stays the OUTPUT resolution. Correcting the aspect by the grid's own
/// 128×128 would make the curve round on a square that is not the screen.
#[test]
fn the_grid_pass_keeps_the_output_resolution() {
    let src = warp_pass_src("demo::warp", false);
    assert!(src.contains("let res = max(pc.res_zoom_time.xy"), "{src}");
}

/// The clock is passed only when the warp asks for it, and asking is the fourth
/// argument — the same arity rule the interpreter uses.
#[test]
fn the_clock_is_passed_exactly_when_the_warp_takes_it() {
    assert!(warp_pass_src("d::w", true).contains("pc.res_zoom_time.w"));
    assert!(!warp_pass_src("d::w", false).contains("pc.res_zoom_time.w"));

    assert!(animated_source(
        "fn hit_inverse(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>, t: f32) -> vec2<f32> {"
    ));
    assert!(!animated_source(
        "fn hit_inverse(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>) -> vec2<f32> {"
    ));
}
