//! Every shipped `pipeline.json` bundle must actually LOAD: sources resolve,
//! `#import`s compose, and every pass compiles to SPIR-V.
//!
//! This is the net for the failure that is otherwise invisible. A WGSL typo, a
//! renamed pass file, or a broken relative path does not surface as an error at
//! runtime — `load_multipass` logs and returns `None`, and the compositor quietly
//! draws the stock background instead. That looks like "my shader did nothing",
//! which is a bad way to find out. Here it is a test failure with the message.
//!
//! It also pins the `-inline` twins, whose `shader` paths point at the ORIGINAL
//! bundle (`../<name>/passes/...`) so the pair can never drift apart. Nothing else
//! checks that indirection resolves.
//!
//! No GPU: `load_pipeline` runs naga_oil composition and naga SPIR-V generation,
//! both host-side. Skips when the examples tree is absent (vendored builds).

/// These tests are about COMPILING a bundle, not about where it runs, so they
/// all load against a session that has a worker. Placement itself is covered
/// by `shader.place`.
const WORKER: compositor_pipeline_build_place_base::place::Env =
    compositor_pipeline_build_place_base::place::Env { worker: true };

use compositor_pipeline_build_pipeline_base::pipeline::load_pipeline;
use std::path::PathBuf;

fn examples() -> Option<PathBuf> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../document/shader-examples");
    d.is_dir().then(|| d)
}

#[test]
fn every_shipped_bundle_compiles() {
    let Some(root) = examples() else { return };
    let mut loaded = 0;
    let mut names: Vec<String> = Vec::new();
    for e in std::fs::read_dir(&root).expect("readable") {
        let dir = e.expect("entry").path();
        if !dir.join("pipeline.json").is_file() {
            continue; // single-pass example, no manifest
        }
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        match load_pipeline(&dir, WORKER) {
            Ok(cp) => {
                assert!(
                    !cp.before.is_empty() || !cp.after.is_empty(),
                    "{name}: loaded with no passes"
                );
                names.push(name);
                loaded += 1;
            }
            Err(e) => panic!("{name}: failed to load — {e}"),
        }
    }
    assert!(loaded >= 20, "only {loaded} bundles found; expected the full set");
    // The twins must exist and must have resolved their `../` sources.
    for t in ["bloom-inline", "tb-heavy-single-inline", "tb-chain-6-inline"] {
        assert!(names.iter().any(|n| n == t), "{t} missing from the shipped set");
    }
}

/// An `-inline` twin must compile to the same shape as its original — same pass
/// counts and same targets — differing only in where the passes are placed.
#[test]
fn inline_twins_match_their_originals() {
    let Some(root) = examples() else { return };
    for o in [
        "bloom",
        "tb-chain-6",
        "tb-heavy-single",
        "tb-many-targets",
        "tb-mixed-scale",
        "tb-wide-32f",
    ] {
        let a = load_pipeline(&root.join(o), WORKER).unwrap_or_else(|e| panic!("{o}: {e}"));
        let b = load_pipeline(&root.join(format!("{o}-inline")), WORKER)
            .unwrap_or_else(|e| panic!("{o}-inline: {e}"));
        assert_eq!(a.before.len(), b.before.len(), "{o}: before-band pass count drifted");
        assert_eq!(a.after.len(), b.after.len(), "{o}: after-band pass count drifted");
        assert_eq!(a.targets.len(), b.targets.len(), "{o}: target count drifted");
        assert_eq!(a.requires, b.requires, "{o}: the requirement set drifted");
    }
}
