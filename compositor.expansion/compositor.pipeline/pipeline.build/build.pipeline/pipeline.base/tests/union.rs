//! The bundle-union param mapping — the link that made a multipass shader's
//! variables actually move something.
//!
//! Three facts have to hold together, and none of them has another guard:
//!
//! 1. a bundle's variables are the UNION across its passes, in manifest order;
//! 2. `shader.load::properties_for` builds that same list, in that same order,
//!    from the manifest alone — it is what the settings panel reads for a bundle
//!    that is merely listed rather than loaded, and the index into it IS the param
//!    slot the panel edits;
//! 3. each pass's `union` maps its OWN slots back to that list.
//!
//! Break any one and the failure is silent: a slider moves the wrong variable, or
//! (as it did before this existed) moves nothing at all, because the push was
//! packed from the compile-time defaults and nothing ever wrote to them again.
//!
//! No GPU — `load_pipeline` is naga composition and SPIR-V generation, host-side.
//! Skips when the examples tree is absent (vendored builds).

const WORKER: compositor_pipeline_build_place_base::place::Env =
    compositor_pipeline_build_place_base::place::Env { worker: true };

use compositor_pipeline_build_pipeline_base::pipeline::load_pipeline;
use std::path::{Path, PathBuf};

fn examples() -> Option<PathBuf> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../document/shader-examples");
    d.is_dir().then(|| d)
}

/// A shipped bundle's directory by name. The `mp-*` set ships compiled INTO the
/// binary, so it resolves to its embedded path — which is not a real directory,
/// and deliberately so: these tests must exercise what the compositor loads.
fn dir(root: &Path, name: &str) -> PathBuf {
    use compositor_pipeline_bundle_embed_base::embed;
    match embed::bundle_of(&embed::id_of(name)) {
        Some(b) => embed::path_of(b),
        None => root.join(name),
    }
}

/// A pass's `union` entry must name the slot in `properties` holding the SAME
/// variable. Checked against every shipped bundle, because the mapping is built
/// per pass and one bundle proving it proves only that bundle.
#[test]
fn every_pass_union_resolves_to_its_own_prop() {
    let Some(root) = examples() else { return };
    let mut checked = 0;
    for e in std::fs::read_dir(&root).expect("readable") {
        let dir = e.expect("entry").path();
        if !dir.join("pipeline.json").is_file() {
            continue;
        }
        let Ok(cp) = load_pipeline(&dir, WORKER) else { continue };
        for pass in cp.before.iter().chain(cp.after.iter()) {
            for &ux in &pass.union {
                assert!(
                    ux < cp.properties.len(),
                    "{:?} pass '{}': union index {ux} past the {}-entry union",
                    dir.file_name().unwrap(),
                    pass.name,
                    cp.properties.len(),
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "no shipped bundle declared a single @prop — the mapping is untested");
}

/// The union a LOADED bundle carries and the union `properties_for` builds from
/// disk must be the same list in the same ORDER.
///
/// Order, not just membership: the settings panel indexes by position, and the
/// two are read on different paths — the panel prefers the loaded bundle and
/// falls back to disk, so a bundle that is listed but not selected goes through
/// the second. If they disagreed, selecting a shader would silently re-point
/// every edited value at a different variable.
#[test]
fn disk_union_matches_compiled_union() {
    let Some(root) = examples() else { return };
    for e in std::fs::read_dir(&root).expect("readable") {
        let dir = e.expect("entry").path();
        if !dir.join("pipeline.json").is_file() {
            continue;
        }
        let Ok(cp) = load_pipeline(&dir, WORKER) else { continue };
        let disk = compositor_pipeline_bundle_load_base::properties_for(&dir.to_string_lossy());
        let compiled: Vec<&str> = cp.properties.iter().map(|p| p.name.as_str()).collect();
        let from_disk: Vec<&str> = disk.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            compiled,
            from_disk,
            "{:?}: compiled union and on-disk union disagree",
            dir.file_name().unwrap(),
        );
    }
}

/// A bundle whose warp names a prop must read that prop's LIVE value, and read it
/// from the same array the passes are pushed from.
///
/// Two failures, one test. The by-name resolution is the one that regressed twice
/// — taking "the first four props" instead handed the warp whatever sat in slot 0
/// of an unrelated pass. The live half is newer and was introduced by fixing the
/// per-pass params: once a pass got live values while the warp kept the compiled
/// defaults, the picture and the pointer correction became two different
/// functions the moment anyone moved the slider. That agrees in every test that
/// only ever looks at defaults, which is why this one edits the value.
#[test]
fn the_warp_reads_its_named_prop_live() {
    let Some(root) = examples() else { return };
    for bundle in ["crt-input-map", "mp-crt"] {
        let d = dir(&root, bundle);
        if !compositor_pipeline_bundle_embed_base::embed::has_manifest(&d) {
            continue;
        }
        let cp = load_pipeline(&d, WORKER).expect("bundle loads");
        let slot = cp
            .properties
            .iter()
            .position(|p| p.name == "curve")
            .expect("declares `curve`");
        assert_eq!(cp.warp_slots[0], slot, "{bundle}: the warp's first lane is not `curve`");

        // The declared defaults resolve to the declared default…
        let mut live = compositor_pipeline_bundle_property_base::default_params(&cp.properties);
        assert_eq!(
            compositor_pipeline_build_seam_base::base::warp_values(&cp, &live)[0],
            cp.properties[slot].default.as_f32(),
            "{bundle}: default resolution is wrong",
        );
        // …and an edit follows, rather than the warp staying on the default.
        live[slot] = 0.37;
        assert_eq!(
            compositor_pipeline_build_seam_base::base::warp_values(&cp, &live)[0],
            0.37,
            "{bundle}: the warp did not follow an edited `curve` — the picture and \
             the pointer correction are two different functions",
        );
    }
}

/// A bundle with variables must expose them. This is the regression the whole
/// change started from: every `passes/`-only bundle read "This shader exposes no
/// variables" in the settings panel while its sources declared a dozen, because
/// the only lookup went through the single-pass format folders.
#[test]
fn multipass_bundles_expose_their_variables() {
    let Some(root) = examples() else { return };
    for (bundle, at_least) in [("glass", 1), ("bloom", 1), ("tb-crt", 1), ("crt-input-map", 3)] {
        let dir = root.join(bundle);
        if !dir.join("pipeline.json").is_file() {
            continue;
        }
        let props = compositor_pipeline_bundle_load_base::properties_for(&dir.to_string_lossy());
        assert!(
            props.len() >= at_least,
            "{bundle}: expected at least {at_least} variable(s), got {}",
            props.len(),
        );
    }
}

/// Every bundle in the curated `Multipass` set says so, every bundle from the
/// shader FOLDER is marked as coming from there, and an undeclared one still
/// lands under a heading.
///
/// The fallback is one load-bearing half: a folder someone drops into the shader
/// directory has to appear in the picker without its author having declared
/// anything, so "no category" must resolve to a heading rather than to nothing.
/// The mark is the other: the picker groups by this string, so a user bundle that
/// declares `Multipass` must not come back equal to the curated `Multipass` or
/// selecting one heading shows both sets.
#[test]
fn categories_resolve_for_every_shipped_bundle() {
    use compositor_pipeline_bundle_builtin_base::{USER_CATEGORY, USER_MARK};
    use compositor_pipeline_bundle_embed_base::embed;
    // The curated set, resolved through its SELECTION id — the string the picker
    // holds — so this also covers `category_for` reading an embedded manifest.
    assert!(!embed::BUNDLES.is_empty(), "the curated set is empty");
    for name in embed::BUNDLES {
        let category = compositor_pipeline_bundle_load_base::category_for(&embed::id_of(name));
        assert_eq!(category, "Multipass", "{name}: curated bundle lost its heading");
        assert!(!category.starts_with(USER_MARK), "{name}: a shipped bundle must not be marked");
    }

    let Some(root) = examples() else { return };
    for e in std::fs::read_dir(&root).expect("readable") {
        let dir = e.expect("entry").path();
        if !dir.is_dir() {
            continue;
        }
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        let category = compositor_pipeline_bundle_load_base::category_for(&dir.to_string_lossy());
        assert_eq!(
            category,
            format!("{USER_MARK}{USER_CATEGORY}"),
            "{name}: undeclared folder bundle should fall back, marked",
        );
    }
}

/// A folder bundle that declares the SAME heading as the curated set is still a
/// different heading. This is the whole point of the mark, and it is the one case
/// the shipped tree cannot cover — nothing in `document/shader-examples` declares
/// a category, so the collision only ever appears with a real user's bundle.
#[test]
fn a_folder_bundle_never_joins_a_shipped_heading() {
    use compositor_pipeline_bundle_builtin_base::USER_MARK;
    use compositor_pipeline_bundle_embed_base::embed;
    let dir = std::env::temp_dir().join("y5-category-collision/mp-lookalike");
    std::fs::create_dir_all(&dir).expect("temp bundle");
    std::fs::write(
        dir.join("pipeline.json"),
        br#"{"name":"mp-lookalike","version":1,"category":"Multipass","passes":[]}"#,
    )
    .expect("write manifest");

    let folder = compositor_pipeline_bundle_load_base::category_for(&dir.to_string_lossy());
    let shipped = compositor_pipeline_bundle_load_base::category_for(&embed::id_of("mp-crt"));
    assert_eq!(folder, format!("{USER_MARK}Multipass"), "a declared heading is marked, not replaced");
    assert_ne!(folder, shipped, "the two would group together in the picker");
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// `mp-crt` must never sample outside the picture, at any curvature, on any
/// aspect ratio from 16:9 to 32:9.
///
/// This is the "no black surround" property, stated as arithmetic. A barrel warp
/// pushes samples outward, so without overscan the screen corner asks for a
/// source point past the edge and gets nothing — which is a black border that
/// grows with `curve` and with the panel's width. `lib/warp.wgsl` normalises by
/// the corner's own displacement to prevent it, and this is what would fail if
/// that normalisation were removed or applied in the pass instead of in the
/// function (where the POINTER correction also gets it).
///
/// Evaluated through the same interpreter the pointer uses, so it is testing the
/// shipped module rather than a Rust restatement of it.
#[test]
fn the_crt_warp_never_leaves_the_picture() {
    use compositor_pipeline_bundle_embed_base::embed;
    let src = embed::read_file(&embed::path_of("mp-crt"), "lib/warp.wgsl")
        .expect("mp-crt ships compiled in, so its warp module is always readable");
    let warp = compositor_pipeline_host_hit_base::hit::parse(&src).expect("mp-crt warp parses");
    // 16:9, 21:9, 32:9 (a 49" ultrawide) and a tall portrait panel.
    for &res in &[[1920.0f32, 1080.0], [3440.0, 1440.0], [5120.0, 1440.0], [1080.0, 1920.0]] {
        // Past the shipped maximum, so the property is not merely true at the
        // default: the knob goes to 0.5 and this checks beyond it.
        for &k in &[0.0f32, 0.14, 0.5, 0.9] {
            let args = compositor_pipeline_host_hit_base::hit::Args {
                res,
                params: [k, 0.0, 0.0, 0.0],
                time: 0.0,
            };
            for i in 0..=16 {
                for j in 0..=16 {
                    let (u, v) = (i as f64 / 16.0, j as f64 / 16.0);
                    let (wu, wv) = warp.eval((u, v), args).expect("evaluates");
                    // A hair of tolerance for f32 rounding at the exact corner.
                    assert!(
                        (-1e-4..=1.0 + 1e-4).contains(&wu)
                            && (-1e-4..=1.0 + 1e-4).contains(&wv),
                        "res {res:?} curve {k}: screen ({u:.3},{v:.3}) samples \
                         ({wu:.4},{wv:.4}) — outside the picture, which draws as black",
                    );
                }
            }
        }
    }
}
