//! Which BAND a bundle's warp lives in, per evaluation mode.
//!
//! A warp in a `before-content` pass over a band the bundle composited itself
//! (`windows: "world"`) displaces only what that pass drew. A warp in an
//! `after-content` pass resamples the whole composited desktop — including the
//! engine's own world-band elements, of which the canvas cursor box is one: it is
//! the last element in the content band, smithay draws elements back to front, so
//! it precedes every window in the op list and `split_at` always leaves it inside
//! `content`.
//!
//! So the two bands need OPPOSITE cursor handling — corrected point under an
//! after-content warp, hand position without one — and `canvas.cursor` branches on
//! exactly that. This file pins that both branches stay reachable from every
//! evaluation mode. They did not: until `tb-crt-after-*` was added, `mp-crt` was
//! the only after-content warper in the tree and it is `map_static`, so the
//! `pointwise` and `map` interpreters had no after-content coverage at all and the
//! branch could regress under either without a single test noticing.
//!
//! Shape only — no GPU, no cursor. The positioning itself is geometry the renderer
//! does; what is checked here is that a bundle exists to exercise it.

use compositor_pipeline_bundle_manifest_base::manifest::Evaluate;
use compositor_pipeline_build_pipeline_base::pipeline::load_pipeline;
use std::path::PathBuf;

const WORKER: compositor_pipeline_build_place_base::place::Env =
    compositor_pipeline_build_place_base::place::Env { worker: true };

fn examples() -> Option<PathBuf> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../document/shader-examples");
    d.is_dir().then(|| d)
}

/// Every evaluation mode has a bundle whose warp is an after-content pass.
#[test]
fn every_evaluate_mode_has_an_after_content_warper() {
    let Some(root) = examples() else { return };
    let embedded = compositor_pipeline_bundle_embed_base::embed::path_of("mp-crt");
    for (bundle, dir, mode) in [
        ("mp-crt", embedded, Evaluate::MapStatic),
        ("tb-crt-after-pointwise", root.join("tb-crt-after-pointwise"), Evaluate::Pointwise),
        ("tb-crt-after-live", root.join("tb-crt-after-live"), Evaluate::Map),
    ] {
        let cp = load_pipeline(&dir, WORKER).unwrap_or_else(|e| panic!("{bundle}: {e}"));
        assert_eq!(cp.warp_evaluate, mode, "{bundle}: evaluation mode drifted");
        assert!(cp.warp.is_some(), "{bundle}: declares `hit` but parsed no warp");
        assert!(
            !cp.after.is_empty(),
            "{bundle}: no after-content pass, so the cursor box is not warped by it — \
             this is the coverage this file exists to hold",
        );
    }
}

/// …and the other band is still covered, or the opposite cursor branch goes
/// untested. `crt-input-map` composites the band itself and warps it BEFORE
/// content, so nothing displaces the engine's cursor solid.
#[test]
fn the_before_band_warpers_still_have_no_after_pass() {
    let Some(root) = examples() else { return };
    for bundle in ["crt-input-map", "crt-input-map-live", "tb-crt-spin"] {
        let dir = root.join(bundle);
        if !dir.join("pipeline.json").is_file() {
            continue;
        }
        let cp = load_pipeline(&dir, WORKER).unwrap_or_else(|e| panic!("{bundle}: {e}"));
        assert!(cp.warp.is_some(), "{bundle}: no warp to speak of");
        assert!(
            cp.after.is_empty(),
            "{bundle}: grew an after-content pass — it was the control for the \
             not-post-processed cursor branch, which now has one less bundle behind it",
        );
    }
}

/// The pair differs in `hit.evaluate` and in nothing else: same warp module, same
/// after-content pass, same variables in the same order. Anything else that
/// differs makes a comparison between them a comparison of two effects.
#[test]
fn the_after_content_pair_differs_only_in_evaluation() {
    let Some(root) = examples() else { return };
    let a = load_pipeline(&root.join("tb-crt-after-pointwise"), WORKER).expect("pointwise loads");
    let b = load_pipeline(&root.join("tb-crt-after-live"), WORKER).expect("live loads");
    assert_ne!(a.warp_evaluate, b.warp_evaluate, "the pair would be one bundle twice");
    assert_eq!(a.before.len(), b.before.len(), "before-band pass count differs");
    assert_eq!(a.after.len(), b.after.len(), "after-band pass count differs");
    assert_eq!(a.requires, b.requires, "requirement set differs");
    let names = |cp: &compositor_pipeline_build_pipeline_base::pipeline::CompiledPipeline| {
        cp.properties.iter().map(|p| p.name.clone()).collect::<Vec<_>>()
    };
    assert_eq!(names(&a), names(&b), "the variable union differs");
    assert_eq!(a.warp_slots[0], b.warp_slots[0], "`curve` resolves to a different slot");
}
