//! The id identifies a COMPILED pipeline, not a location.
//!
//! The renderer caches every pass's Vulkan pipeline by this id, and `GraphExec`
//! rebuilds nothing while the id is unchanged. So an id keyed on the bundle path
//! alone means re-loading an edited bundle produces fresh SPIR-V that the
//! renderer then ignores in favour of what it compiled from the same path
//! earlier: the edit has no effect on screen and nothing anywhere says why.
//!
//! That is the entire edit-and-reload loop — the gRPC `Shader::reload` call, and
//! equally the settings panel re-selecting a bundle the user has just changed.
//! Nothing else in the tree would catch it, because both halves are individually
//! correct.

use compositor_pipeline_build_pipeline_base::pipeline::load_pipeline;
use std::path::PathBuf;

const WORKER: compositor_pipeline_build_place_base::place::Env =
    compositor_pipeline_build_place_base::place::Env { worker: true };

/// A throwaway bundle whose one pass returns `col`.
fn write_bundle(dir: &std::path::Path, col: &str) {
    std::fs::create_dir_all(dir.join("passes")).expect("mkdir");
    std::fs::write(
        dir.join("passes/p.wgsl"),
        format!("@fragment\nfn fs_main() -> @location(0) vec4<f32> {{ return {col}; }}\n"),
    )
    .expect("write pass");
    std::fs::write(
        dir.join("pipeline.json"),
        r#"{"name":"r","version":1,
            "passes":[{"name":"p","shader":"passes/p.wgsl","output":"output"}]}"#,
    )
    .expect("write manifest");
}

#[test]
fn editing_a_bundle_changes_its_id_so_the_renderer_rebuilds() {
    let dir = std::env::temp_dir().join("y5-test-reload-id");
    let _ = std::fs::remove_dir_all(&dir);

    write_bundle(&dir, "vec4<f32>(1.0, 0.0, 0.0, 1.0)");
    let before = load_pipeline(&dir, WORKER).expect("loads");
    let before_pass = before.before[0].name.clone();
    let (id_before, pass_before) = (before.id, pass_id(&before));

    // Same path, same manifest, different shader source — the edit-and-reload case.
    write_bundle(&dir, "vec4<f32>(0.0, 1.0, 0.0, 1.0)");
    let after = load_pipeline(&dir, WORKER).expect("reloads");

    assert_eq!(after.before[0].name, before_pass, "the same bundle, not a different one");
    assert_ne!(
        id_before, after.id,
        "the pipeline id survived an edit — `GraphExec::prepare` would see no change and \
         keep every previously compiled pass",
    );
    assert_ne!(
        pass_before,
        pass_id(&after),
        "the per-pass id survived an edit — the pass cache is keyed on it, so the renderer \
         would draw with the old SPIR-V",
    );

    // …and re-loading the SAME bytes is the same pipeline, so an editor that
    // rewrites a file unchanged does not force a recompile of every pass.
    write_bundle(&dir, "vec4<f32>(0.0, 1.0, 0.0, 1.0)");
    let again = load_pipeline(&dir, WORKER).expect("loads");
    assert_eq!(after.id, again.id, "identical sources must be the same pipeline");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Two DIFFERENT bundles must still differ, or the path would have stopped
/// mattering when the content started to.
#[test]
fn two_bundles_with_identical_sources_are_still_distinct() {
    let a = std::env::temp_dir().join("y5-test-reload-a");
    let b = std::env::temp_dir().join("y5-test-reload-b");
    for d in [&a, &b] {
        let _ = std::fs::remove_dir_all(d);
        write_bundle(d, "vec4<f32>(1.0)");
    }
    let pa = load_pipeline(&a, WORKER).expect("a loads");
    let pb = load_pipeline(&b, WORKER).expect("b loads");
    assert_ne!(pa.id, pb.id, "two bundles collapsed onto one cache entry");
    for d in [&a, &b] {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// The shipped bundles still load and still have distinct ids — the id change
/// must not have made any two of them collide.
#[test]
fn the_shipped_ids_stay_unique() {
    use compositor_pipeline_bundle_embed_base::embed;
    let mut seen = std::collections::HashSet::new();
    for name in embed::BUNDLES {
        let cp = load_pipeline(&embed::path_of(name), WORKER).expect("shipped bundle loads");
        assert!(seen.insert(cp.id), "{name} collides with another shipped bundle's id");
    }
}

fn pass_id(cp: &compositor_pipeline_build_pipeline_base::pipeline::CompiledPipeline) -> u64 {
    cp.before.first().or_else(|| cp.after.first()).expect("a pass").module.id
}
