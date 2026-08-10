//! Plan validation: banding, target-reference checks, and rejection rules.

use compositor_pipeline_bundle_graph_base::graph::plan;
use compositor_pipeline_bundle_manifest_base::manifest::parse;

const BLOOM: &str = r#"{
  "name": "bloom",
  "targets": { "scene": {}, "bright": {"scale":0.5}, "blur_a": {"scale":0.5}, "blur_b": {"scale":0.5} },
  "passes": [
    { "name": "base",    "shader": "p/base.wgsl",    "output": "scene" },
    { "name": "bright",  "shader": "p/bright.wgsl",  "inputs": {"src":"scene"},  "output": "bright" },
    { "name": "blurH",   "shader": "p/blur.wgsl",    "inputs": {"src":"bright"}, "output": "blur_a" },
    { "name": "blurV",   "shader": "p/blur.wgsl",    "inputs": {"src":"blur_a"}, "output": "blur_b" },
    { "name": "combine", "shader": "p/combine.wgsl", "inputs": {"scene":"scene","bloom":"blur_b"}, "output": "output" }
  ]
}"#;

#[test]
fn bloom_plans_all_before_content() {
    let p = plan(&parse(BLOOM).unwrap()).expect("valid plan");
    assert_eq!(p.before, vec![0, 1, 2, 3, 4]);
    assert!(p.after.is_empty());
}

#[test]
fn after_content_pass_may_read_content() {
    let m = parse(r#"{
      "name": "vig",
      "passes": [
        { "name": "v", "shader": "v.wgsl", "when": "after-content",
          "inputs": {"scene":"content"}, "requires": ["composited_scene"], "output": "output" }
      ]
    }"#).unwrap();
    let p = plan(&m).unwrap();
    assert_eq!(p.after, vec![0]);
    assert!(p.before.is_empty());
}

#[test]
fn before_content_reading_content_is_rejected() {
    let m = parse(r#"{
      "name": "bad",
      "passes": [ { "name": "b", "shader": "b.wgsl", "inputs": {"s":"content"}, "output": "output" } ]
    }"#).unwrap();
    assert!(plan(&m).unwrap_err().contains("before-content"));
}

#[test]
fn after_content_pass_may_read_history() {
    let m = parse(r#"{
      "name": "mblur",
      "passes": [
        { "name": "b", "shader": "b.wgsl", "when": "after-content",
          "inputs": {"cur":"content","prev":"history"},
          "requires": ["composited_scene", "previous_frame"], "output": "output" }
      ]
    }"#).unwrap();
    let p = plan(&m).unwrap();
    assert_eq!(p.after, vec![0]);
}

#[test]
fn before_content_reading_history_is_rejected() {
    let m = parse(r#"{
      "name": "bad",
      "passes": [ { "name": "b", "shader": "b.wgsl", "inputs": {"h":"history"}, "output": "output" } ]
    }"#).unwrap();
    assert!(plan(&m).unwrap_err().contains("before-content"));
}

#[test]
fn writing_history_is_rejected() {
    let m = parse(r#"{
      "name": "bad",
      "passes": [ { "name": "b", "shader": "b.wgsl", "when": "after-content",
        "inputs": {"s":"content"}, "output": "history" } ]
    }"#).unwrap();
    assert!(plan(&m).unwrap_err().contains("engine target"));
}

#[test]
fn unknown_target_is_rejected() {
    let m = parse(r#"{
      "name": "bad",
      "passes": [ { "name": "b", "shader": "b.wgsl", "inputs": {"s":"nope"}, "output": "output" } ]
    }"#).unwrap();
    assert!(plan(&m).unwrap_err().contains("unknown target"));
}

#[test]
fn must_write_output() {
    let m = parse(r#"{
      "name": "bad",
      "targets": { "scene": {} },
      "passes": [ { "name": "b", "shader": "b.wgsl", "output": "scene" } ]
    }"#).unwrap();
    assert!(plan(&m).unwrap_err().contains("output"));
}

#[test]
fn cannot_write_engine_target() {
    let m = parse(r#"{
      "name": "bad",
      "passes": [ { "name": "b", "shader": "b.wgsl", "output": "content" } ]
    }"#).unwrap();
    assert!(plan(&m).unwrap_err().contains("engine target"));
}

/// Both ownership modes hand compositing to the graph, so at least one pass must
/// receive the world set — either interface counts. A bundle that deliberately
/// draws nothing (the suppression control) needs only rects.
///
/// `world` is checked alongside `pipeline` because it takes MORE out of the
/// engine's band — the iced-world panels too — so reaching it without a pass to
/// receive the set loses more than windows.
#[test]
fn every_ownership_mode_requires_a_window_need() {
    for mode in ["pipeline", "world"] {
        let m = |requires: &str| {
            format!(
                r#"{{ "name": "n", "windows": "{mode}",
                   "passes": [ {{ "name": "a", "shader": "a.wgsl", "output": "output"{requires} }} ] }}"#
            )
        };
        assert!(
            plan(&parse(&m("")).unwrap()).is_err(),
            "{mode}: no window need must be rejected"
        );
        assert!(
            plan(&parse(&m(r#", "requires": ["window_geometry"]"#)).unwrap()).is_ok(),
            "{mode}: rects alone is enough"
        );
        assert!(
            plan(&parse(&m(r#", "requires": ["window_textures"]"#)).unwrap()).is_ok(),
            "{mode}: textures alone is enough"
        );
    }
}

/// The error names the mode the manifest actually used — the message is the only
/// thing a bundle author sees when `load_multipass` falls back to the stock
/// background, and "`windows: pipeline`" pointing at a `world` manifest sends
/// them looking for a line that is not there.
#[test]
fn the_rejection_names_the_mode_that_was_written() {
    let src = r#"{ "name": "n", "windows": "world",
                   "passes": [ { "name": "a", "shader": "a.wgsl", "output": "output" } ] }"#;
    let err = plan(&parse(src).unwrap()).unwrap_err();
    assert!(err.contains("`windows: world`"), "{err}");
}

/// Cadence holds an intermediate between runs, so it is only meaningful there.
/// Skipping the `output` pass would leave the frame with no picture at all,
/// which is a different thing from a stale one.
#[test]
fn cadence_is_rejected_on_the_output_pass() {
    const ON_OUTPUT: &str = r#"{
      "name": "c",
      "passes": [ { "name": "a", "shader": "a.wgsl", "output": "output", "cadence": 2 } ]
    }"#;
    const ON_TARGET: &str = r#"{
      "name": "c",
      "targets": { "t": {} },
      "passes": [
        { "name": "a", "shader": "a.wgsl", "output": "t", "cadence": 4 },
        { "name": "b", "shader": "b.wgsl", "inputs": {"s":"t"}, "output": "output" }
      ]
    }"#;
    const ZERO: &str = r#"{
      "name": "c",
      "targets": { "t": {} },
      "passes": [
        { "name": "a", "shader": "a.wgsl", "output": "t", "cadence": 0 },
        { "name": "b", "shader": "b.wgsl", "inputs": {"s":"t"}, "output": "output" }
      ]
    }"#;
    assert!(plan(&parse(ON_OUTPUT).unwrap()).is_err(), "output pass must run every frame");
    assert!(plan(&parse(ON_TARGET).unwrap()).is_ok(), "cadence on an intermediate is fine");
    assert!(plan(&parse(ZERO).unwrap()).is_err(), "cadence 0 would divide by zero");
}

/// `world-rects`/`world-textures` widen what a NON-owning bundle is handed. Asking
/// for them alongside an ownership mode is a contradiction, not a preference: the
/// mode already fixed membership, and under `pipeline` the shader would draw
/// panels the engine is also drawing.
#[test]
fn the_world_set_opt_in_is_only_for_engine_mode() {
    let m = |mode: &str| {
        format!(
            r#"{{ "name": "n", "windows": "{mode}",
               "passes": [ {{ "name": "a", "shader": "a.wgsl", "output": "output",
                              "requires": ["world_geometry", "world_textures"] }} ] }}"#
        )
    };
    assert!(plan(&parse(&m("engine")).unwrap()).is_ok(), "engine mode may widen its set");
    for mode in ["pipeline", "world"] {
        let err = plan(&parse(&m(mode)).unwrap()).unwrap_err();
        assert!(err.contains("windows: engine"), "{mode}: {err}");
    }
}

/// And `world_*` satisfies the ownership requirement the same way `window_*` does
/// — it is the same two bindings, so a bundle is not left "receiving nothing".
#[test]
fn world_requirements_bind_the_same_two_slots() {
    use compositor_pipeline_bundle_manifest_base::manifest::{Requirement, Requires};
    let of = |r| Requires::of([r]);
    assert!(of(Requirement::WorldGeometry).world_set() && of(Requirement::WindowGeometry).world_set());
    assert!(of(Requirement::WorldTextures).textures() && of(Requirement::WindowTextures).textures());
    assert!(of(Requirement::WorldTextures).world_set(), "textures imply the set they index into");
    assert!(of(Requirement::WorldGeometry).whole_band() && !of(Requirement::WindowGeometry).whole_band());
    assert!(!of(Requirement::CompositedScene).world_set());
}

/// An empty set is the one that has to be right: it is what every gate reads when
/// no bundle is loaded, and each `false` here is one engine cost that stays off.
#[test]
fn empty_requires_turns_everything_off() {
    use compositor_pipeline_bundle_manifest_base::manifest::Requires;
    let none = Requires::NONE;
    assert!(none.is_empty());
    assert!(!none.world_set(), "no world set collected");
    assert!(!none.textures(), "no bindless array, no descriptor indexing");
    assert!(!none.whole_band());
    assert!(!none.offscreen(), "no content/windows/history image");
}

/// Sampling a built-in image without declaring it is refused, and the message
/// names the entry to add. Inferring it instead would put a VRAM allocation
/// behind how a shader happens to be written.
#[test]
fn sampling_a_builtin_undeclared_is_rejected() {
    let src = r#"{ "name": "n", "passes": [
        { "name": "v", "shader": "v.wgsl", "when": "after-content",
          "inputs": {"s":"content"}, "output": "output" } ] }"#;
    let err = plan(&parse(src).unwrap()).unwrap_err();
    assert!(err.contains("composited_scene"), "{err}");
}
