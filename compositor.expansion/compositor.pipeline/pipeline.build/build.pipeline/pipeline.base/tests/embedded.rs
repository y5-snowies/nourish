//! The compiled-in multipass bundles must load from the embedded table alone.
//!
//! `shipped.rs` next door proves these bundles compile FROM DISK. That says
//! nothing about whether the binary can load them, which is a different failure
//! with the same symptom: a bundle whose `include_str!` set is missing a file its
//! manifest names loads perfectly from `document/shader-examples` and not at all
//! from the binary, and the user sees the stock parallax with no error.
//!
//! It is a real hazard rather than a hypothetical one, because the `mp-*` bundles
//! share sources ACROSS folders — every filter samples `mp-parallax`'s backdrop
//! through `../mp-parallax/passes/backdrop.wgsl`. The disk loader gets that from
//! the filesystem for free; the embedded one has to normalise the `..` itself and
//! have the target in the table under the key it normalises to.
//!
//! Loading against `embed::path_of` is the point: that path does not exist, so
//! anything reached by `std::fs` here fails rather than silently passing on the
//! developer's installed copy.

use compositor_pipeline_bundle_embed_base::embed;
use compositor_pipeline_build_pipeline_base::pipeline::load_pipeline;

const WORKER: compositor_pipeline_build_place_base::place::Env =
    compositor_pipeline_build_place_base::place::Env { worker: true };

#[test]
fn every_embedded_bundle_loads_without_touching_the_disk() {
    assert!(!embed::BUNDLES.is_empty(), "the embedded set is empty — nothing ships compiled in");
    for name in embed::BUNDLES {
        let path = embed::path_of(name);
        assert!(!path.exists(), "{name}: the virtual root must not be a real directory");
        assert!(embed::has_manifest(&path), "{name}: no embedded pipeline.json");
        match load_pipeline(&path, WORKER) {
            Ok(cp) => assert!(
                !cp.before.is_empty() || !cp.after.is_empty(),
                "{name}: loaded with no passes"
            ),
            Err(e) => panic!("{name}: failed to load from the embedded table — {e}"),
        }
    }
}

/// Loading a bundle from the table and loading the same folder from disk must
/// agree — the two READ paths, over the same bytes.
///
/// Not a tautology, because the embedded path resolves `..` and normalises keys
/// itself while the disk path hands that to the filesystem. A bundle whose table
/// rows are complete but whose cross-folder references normalise to the wrong key
/// loads to a DIFFERENT shape rather than failing, and the difference is a pass
/// silently missing from the graph.
#[test]
fn embedded_matches_the_bundle_on_disk() {
    let root = embed::source_dir();
    if !root.is_dir() {
        return; // sources pruned; the table is still compiled in
    }
    for name in embed::BUNDLES {
        let disk = load_pipeline(&root.join(name), WORKER).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mem = load_pipeline(&embed::path_of(name), WORKER)
            .unwrap_or_else(|e| panic!("{name} (embedded): {e}"));
        assert_eq!(disk.before.len(), mem.before.len(), "{name}: before-band pass count differs");
        assert_eq!(disk.after.len(), mem.after.len(), "{name}: after-band pass count differs");
        assert_eq!(disk.targets.len(), mem.targets.len(), "{name}: target count differs");
        assert_eq!(disk.requires, mem.requires, "{name}: requirement set differs");
        let names = |p: &[String]| p.to_vec();
        assert_eq!(
            names(&disk.properties.iter().map(|p| p.name.clone()).collect::<Vec<_>>()),
            names(&mem.properties.iter().map(|p| p.name.clone()).collect::<Vec<_>>()),
            "{name}: the variable union differs, so the settings panel would edit different slots"
        );
    }
}

/// A selection id resolves to the embedded bundle, and an unknown one does not.
#[test]
fn only_the_shipped_names_resolve() {
    for name in embed::BUNDLES {
        assert_eq!(embed::bundle_of(&embed::id_of(name)), Some(*name));
    }
    assert_eq!(embed::bundle_of("builtin:not-a-bundle"), None);
    assert_eq!(embed::bundle_of("mp-crt"), None, "a bare name is a DISK bundle");
    assert_eq!(embed::bundle_of("builtin:aurora"), None, "that is a single-source built-in");
}
