//! Placement resolution: eligibility, explicit overrides, contiguity, and the
//! two closure rules (the worker band must publish `output`, and must not feed
//! an inline pass from a target that never leaves its device).

use compositor_pipeline_bundle_graph_base::graph::plan;
use compositor_pipeline_bundle_manifest_base::manifest::parse;
use compositor_pipeline_build_place_base::place::{place, Env, Offload, Site};

/// A session that CAN offload. Every test below is about what the manifest and
/// the graph allow, so they all run against a worker that exists — the case where
/// it does not has its own test.
const WITH_WORKER: Env = Env { worker: true };

fn resolve(src: &str) -> (Vec<Site>, Offload, Vec<String>) {
    let m = parse(src).expect("parses");
    let p = plan(&m).expect("plans");
    let r = place(&m, &p, WITH_WORKER);
    // Denials are asserted where they are the point (`denied_*` tests); a test
    // asserting on notes must not silently pass while a pass was refused.
    (r.sites, r.offload, r.notes)
}

/// Everything a pass explicitly asked for and did not get.
fn denials(src: &str) -> Vec<String> {
    let m = parse(src).expect("parses");
    let p = plan(&m).expect("plans");
    place(&m, &p, WITH_WORKER).denied
}

/// With no worker in the session, a graph is not "placed inline" — a pass that
/// ASKED for the worker is refused, and says so.
///
/// This is the case placement used to be blind to: `place()` never read the
/// setting, so with triple buffering off it still reported `offload=Whole` while
/// nothing was offloaded and nothing anywhere said otherwise.
#[test]
fn without_a_worker_an_explicit_request_is_denied() {
    let src = r#"{ "name": "n", "passes": [
        { "name": "a", "shader": "a.wgsl", "output": "output", "place": "worker" } ] }"#;
    let m = parse(src).unwrap();
    let p = plan(&m).unwrap();
    let r = place(&m, &p, Env { worker: false });
    assert_eq!(r.offload, Offload::None);
    assert_eq!(r.sites, vec![Site::Compositor]);
    assert_eq!(r.denied.len(), 1, "{:?}", r.denied);
    assert!(r.denied[0].contains("triple buffering"), "{:?}", r.denied);
}

/// And an `auto` pass is NOT denied by the same session — it asked for nothing,
/// so running inline is the resolution working, not a refusal.
#[test]
fn without_a_worker_an_auto_pass_is_not_denied() {
    let src = r#"{ "name": "n", "passes": [
        { "name": "a", "shader": "a.wgsl", "output": "output" } ] }"#;
    let m = parse(src).unwrap();
    let p = plan(&m).unwrap();
    let r = place(&m, &p, Env { worker: false });
    assert_eq!(r.offload, Offload::None);
    assert!(r.denied.is_empty(), "{:?}", r.denied);
}

/// A closed before-only chain — the `bloom` / `tb-chain-6` shape.
const CLOSED: &str = r#"{
  "name": "closed",
  "targets": { "a": {}, "b": {} },
  "passes": [
    { "name": "seed", "shader": "p/s.wgsl", "output": "a" },
    { "name": "mid",  "shader": "p/m.wgsl", "inputs": {"src":"a"}, "output": "b" },
    { "name": "out",  "shader": "p/o.wgsl", "inputs": {"src":"b"}, "output": "output" }
  ]
}"#;

/// backdrop + an after-content pass — the `tb-after-trivial` shape.
const AFTER: &str = r#"{
  "name": "after",
  "passes": [
    { "name": "backdrop", "shader": "p/b.wgsl", "output": "output" },
    { "name": "tint", "shader": "p/t.wgsl", "when": "after-content",
      "inputs": {"scene":"content"}, "requires": ["composited_scene"], "output": "output" }
  ]
}"#;

#[test]
fn closed_before_chain_offloads_whole() {
    let (sites, offload, notes) = resolve(CLOSED);
    assert_eq!(sites, vec![Site::Worker; 3]);
    assert_eq!(offload, Offload::Whole);
    assert!(notes.is_empty(), "{notes:?}");
}

#[test]
fn after_content_splits_into_a_before_band() {
    let (sites, offload, notes) = resolve(AFTER);
    assert_eq!(sites, vec![Site::Worker, Site::Compositor]);
    assert_eq!(offload, Offload::BeforeBand);
    assert!(notes.is_empty(), "{notes:?}");
}

#[test]
fn both_window_interfaces_are_worker_eligible() {
    // Geometry is numbers through a shared slot; textures are client dmabufs the
    // worker imports itself, duping the fd. Neither shares a lifetime with the
    // compositor, so the MANIFEST places both on the worker.
    //
    // What cannot cross is a SHM surface, which has no fd — but that is a property
    // of the live desktop, not of the bundle, so it is decided per frame at
    // dispatch (`ParallaxBackground::worker_can_render`) and deliberately not here.
    for req in ["window_geometry", "window_textures"] {
        let src = format!(
            r#"{{ "name": "n", "passes": [
                 {{ "name": "b", "shader": "b.wgsl", "output": "output",
                    "requires": ["{req}"] }} ] }}"#
        );
        let (sites, offload, notes) = resolve(&src);
        assert_eq!(sites, vec![Site::Worker], "{req}");
        assert_eq!(offload, Offload::Whole, "{req}");
        assert!(notes.is_empty(), "{req}: {notes:?}");
    }
}

/// `windows: pipeline` used to force the whole graph inline, because the engine's
/// window suppression keyed on a pipeline op being in the COMPOSITOR's frame and
/// an offloaded graph leaves none. That intent now has its own channel
/// (`bridge.window::own`), so such a graph offloads like any other.
#[test]
fn an_ownership_mode_can_offload_now_that_intent_has_a_channel() {
    let src = r#"{
      "name": "w", "windows": "pipeline",
      "passes": [
        { "name": "b", "shader": "b.wgsl", "output": "output",
          "requires": ["window_geometry"] }
      ]
    }"#;
    let (sites, offload, _) = resolve(src);
    assert_eq!(sites, vec![Site::Worker]);
    assert_eq!(offload, Offload::Whole);
}

#[test]
fn worker_passes_must_be_contiguous_from_the_start() {
    // `seed` is pinned inline, so `out` (otherwise eligible) cannot be the only
    // worker pass — that would be compositor -> worker, a second boundary.
    let src = r#"{
      "name": "interleaved",
      "targets": { "a": {} },
      "passes": [
        { "name": "seed", "shader": "p/s.wgsl", "output": "a", "place": "compositor" },
        { "name": "out",  "shader": "p/o.wgsl", "inputs": {"src":"a"}, "output": "output" }
      ]
    }"#;
    let (sites, offload, notes) = resolve(src);
    assert_eq!(sites, vec![Site::Compositor, Site::Compositor]);
    assert_eq!(offload, Offload::None);
    assert!(notes.iter().any(|n| n.contains("contiguously")), "{notes:?}");
}

#[test]
fn a_band_that_never_writes_output_is_demoted() {
    // The before pass fills an intermediate; only the after pass writes `output`,
    // so the worker would have no finished image to publish.
    let src = r#"{
      "name": "no-publish",
      "targets": { "a": {} },
      "passes": [
        { "name": "seed", "shader": "p/s.wgsl", "output": "a" },
        { "name": "post", "shader": "p/p.wgsl", "when": "after-content",
          "inputs": {"scene":"content"}, "requires": ["composited_scene"], "output": "output" }
      ]
    }"#;
    let (sites, offload, notes) = resolve(src);
    assert_eq!(sites, vec![Site::Compositor, Site::Compositor]);
    assert_eq!(offload, Offload::None);
    assert!(notes.iter().any(|n| n.contains("never writes `output`")), "{notes:?}");
}

#[test]
fn a_band_feeding_an_inline_pass_is_demoted() {
    // The band publishes `output`, but an after pass also samples `a` — an
    // intermediate that would only ever exist on the worker's device.
    let src = r#"{
      "name": "leak",
      "targets": { "a": {} },
      "passes": [
        { "name": "seed", "shader": "p/s.wgsl", "output": "a" },
        { "name": "base", "shader": "p/b.wgsl", "inputs": {"src":"a"}, "output": "output" },
        { "name": "post", "shader": "p/p.wgsl", "when": "after-content",
          "inputs": {"scene":"content","extra":"a"}, "requires": ["composited_scene"],
          "output": "output" }
      ]
    }"#;
    let (sites, offload, notes) = resolve(src);
    assert_eq!(sites, vec![Site::Compositor; 3]);
    assert_eq!(offload, Offload::None);
    assert!(notes.iter().any(|n| n.contains("never leave the worker device")), "{notes:?}");
}

#[test]
fn place_compositor_pins_an_otherwise_eligible_graph_inline() {
    let src = r#"{
      "name": "pinned",
      "passes": [
        { "name": "only", "shader": "p/o.wgsl", "output": "output", "place": "compositor" }
      ]
    }"#;
    let (sites, offload, notes) = resolve(src);
    assert_eq!(sites, vec![Site::Compositor]);
    assert_eq!(offload, Offload::None);
    assert!(notes.is_empty(), "{notes:?}");
}

/// Stage 4 — the worker taking a graph's TAIL — is DISABLED, and this pins it.
///
/// `place: "worker"` on an after-content pass is refused, with a note, and the
/// graph resolves to the head-only shape every bundle had before stage 4. The
/// tail machinery is still present (`Offload::AfterBand`, the transport, the
/// present path) and unreachable; `shader.place::eligible` says why, and says
/// what re-enabling it would require.
///
/// This is the test to flip FIRST when that work resumes: it should then expect
/// `AfterBand` and an empty `notes`.
#[test]
fn the_tail_offload_is_disabled_and_an_after_pass_stays_inline() {
    for (target, req) in
        [("content", "composited_scene"), ("history", "previous_frame"), ("windows", "window_layer")]
    {
        let src = format!(
            r#"{{ "name": "d", "passes": [
                {{ "name": "b", "shader": "b.wgsl", "output": "output" }},
                {{ "name": "a", "shader": "a.wgsl", "when": "after-content",
                   "inputs": {{"t":"{target}"}}, "requires": ["{req}"],
                   "output": "output", "place": "worker" }}
            ] }}"#
        );
        let (sites, offload, notes) = resolve(&src);
        // The head still offloads — that half was never in question.
        assert_eq!(sites, vec![Site::Worker, Site::Compositor], "{target}");
        assert_eq!(offload, Offload::BeforeBand, "{target}");
        assert!(notes.is_empty(), "{target}: nothing was downgraded unasked — {notes:?}");
        // The refusal lands in `denied`, not `notes`, and that is what makes the
        // producer refuse the bundle instead of running it in a shape its author
        // did not choose.
        let denied = denials(&src);
        assert_eq!(denied.len(), 1, "{target}: {denied:?}");
        assert!(denied[0].contains("asks for the worker"), "{target}: {denied:?}");
    }
}
