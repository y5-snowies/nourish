//! The two persistence features, at the level the loader decides them.
//!
//! Neither can be tested for its EFFECT here — that needs a device and a second
//! frame. What is testable is what the manifest resolves to, which is where both
//! features are actually decided: a target that forgot to be persistent and a
//! budget that failed to refuse both produce a bundle that loads happily and is
//! quietly wrong on hardware.

use compositor_pipeline_build_pipeline_base::pipeline::{STORAGE_BUDGET, load_pipeline};
use std::path::PathBuf;

const WORKER: compositor_pipeline_build_place_base::place::Env =
    compositor_pipeline_build_place_base::place::Env { worker: true };

fn examples() -> Option<PathBuf> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../document/shader-examples");
    d.is_dir().then_some(d)
}

/// `persist` reaches the compiled pipeline, and only where it was asked for.
#[test]
fn a_persistent_target_is_marked_and_others_are_not() {
    let Some(root) = examples() else { return };
    let cp = load_pipeline(&root.join("tb-persist-trail"), WORKER).expect("loads");
    assert_eq!(cp.targets.len(), 1);
    assert!(cp.targets[0].persist, "the trail target lost its persistence");

    // The control: an ordinary multipass bundle must not have gained it. `persist`
    // defaults to false, and a default that flipped would double every bundle's
    // target memory silently.
    let plain = load_pipeline(&root.join("bloom"), WORKER).expect("bloom loads");
    assert!(!plain.targets.is_empty());
    assert!(plain.targets.iter().all(|t| !t.persist), "an ordinary target became persistent");
}

/// The storage declaration reaches the compiled pipeline, sized and rounded.
#[test]
fn storage_is_declared_rounded_and_absent_by_default() {
    let Some(root) = examples() else { return };
    let cp = load_pipeline(&root.join("tb-storage-histogram"), WORKER).expect("loads");
    assert_eq!(cp.storage.len(), 1, "the bins buffer did not survive the load");
    assert_eq!(cp.storage[0].bytes, 1024);
    assert_eq!(cp.storage[0].bytes % 16, 0, "a `vec4`-strided array could run off the end");

    // Storage is what gates a whole device feature, so a bundle that declares
    // none must report none — otherwise every bundle would be refused on a device
    // without `fragmentStoresAndAtomics`.
    let plain = load_pipeline(&root.join("bloom"), WORKER).expect("bloom loads");
    assert!(plain.storage.is_empty(), "a bundle declaring no storage claimed some");
}

/// Over budget is REFUSED, not trimmed.
///
/// Trimming is the tempting behaviour and the wrong one: a shader indexes the
/// size it declared, so a buffer quietly made smaller is written off the end of.
/// The budget is also summed across buffers, or twenty small ones walk past it.
#[test]
fn an_oversized_storage_declaration_is_refused_with_the_figure() {
    let dir = std::env::temp_dir().join("y5-test-storage-budget");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("passes")).expect("mkdir");
    std::fs::write(
        dir.join("passes/p.wgsl"),
        "@fragment\nfn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }\n",
    )
    .expect("write");
    // Two buffers, each inside the budget, together past it.
    let half = STORAGE_BUDGET / 2 + 16;
    std::fs::write(
        dir.join("pipeline.json"),
        format!(
            r#"{{"name":"over","version":1,
                 "storage":{{"a":{{"bytes":{half}}},"b":{{"bytes":{half}}}}},
                 "passes":[{{"name":"p","shader":"passes/p.wgsl","output":"output"}}]}}"#
        ),
    )
    .expect("write");

    let err = match load_pipeline(&dir, WORKER) {
        Err(e) => e,
        Ok(_) => panic!("an over-budget bundle must not load"),
    };
    assert!(err.contains("storage"), "the message must name what was refused: {err}");
    assert!(
        err.contains(&STORAGE_BUDGET.to_string()),
        "the message must state the budget so an author can size against it: {err}",
    );
    let _ = std::fs::remove_dir_all(&dir);
}
