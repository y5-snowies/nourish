//! Which bundles need the whole element list — the rule has no other guard.

use compositor_pipeline_build_pipeline_base::pipeline::load_pipeline;
use compositor_pipeline_abi_seam_base::base::WorldOwn;
use std::path::PathBuf;

const WORKER: compositor_pipeline_build_place_base::place::Env =
    compositor_pipeline_build_place_base::place::Env { worker: true };

/// A shipped bundle by name, from wherever it ships: the compiled-in `mp-*` set
/// resolves to its embedded path (no disk at all), everything else to the
/// examples tree.
fn bundle(name: &str) -> compositor_pipeline_build_pipeline_base::pipeline::CompiledPipeline {
    use compositor_pipeline_bundle_embed_base::embed;
    let dir = match embed::bundle_of(&embed::id_of(name)) {
        Some(b) => embed::path_of(b),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../../document/shader-examples")
            .join(name),
    };
    load_pipeline(&dir, WORKER).unwrap_or_else(|e| panic!("{name} compiles: {e:?}"))
}

/// Owning the band composes whole; so does declaring any requirement, without
/// owning anything.
#[test]
fn owning_or_requiring_composes_whole() {
    let owned = bundle("tb-window-owned");
    assert_ne!(owned.owns, WorldOwn::Engine);
    assert!(owned.composes_whole_frame());

    // `mp-levels`, not `mp-effects`: the latter OWNS the band now, so it proves
    // the first half twice over and the requires-without-owning half not at all.
    let reader = bundle("mp-levels");
    assert_eq!(reader.owns, WorldOwn::Engine);
    assert!(!reader.requires.is_empty());
    assert!(reader.composes_whole_frame());
}

/// The damaged composite survives for the case it was written for: a graph that
/// asks the engine for nothing and takes over nothing is a picture.
#[test]
fn a_plain_bundle_keeps_the_damaged_composite() {
    let p = bundle("mp-parallax");
    assert_eq!(p.owns, WorldOwn::Engine);
    assert!(p.requires.is_empty());
    assert!(!p.composes_whole_frame());
}
