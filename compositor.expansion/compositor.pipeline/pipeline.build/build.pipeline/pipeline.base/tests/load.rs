//! Load the shipped `bloom` example end-to-end and check its compiled wiring.

use compositor_pipeline_bundle_manifest_base::manifest::Requirement;
/// These tests are about COMPILING a bundle, not about where it runs, so they
/// all load against a session that has a worker. Placement itself is covered
/// by `shader.place`.
const WORKER: compositor_pipeline_build_place_base::place::Env =
    compositor_pipeline_build_place_base::place::Env { worker: true };

use compositor_pipeline_build_pipeline_base::pipeline::{load_pipeline, Output, CONTENT, HISTORY};
use std::path::PathBuf;

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../../document/shader-examples")
}
fn bloom_dir() -> PathBuf {
    examples().join("bloom")
}

#[test]
fn loads_bloom_pipeline() {
    let p = load_pipeline(&bloom_dir(), WORKER).expect("bloom compiles");

    // 4 declared intermediate targets, 5 passes, all before-content.
    assert_eq!(p.targets.len(), 4);
    assert_eq!(p.before.len(), 5);

    let names: Vec<&str> = p.before.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["base", "bright", "blurH", "blurV", "combine"]);

    // base: no inputs; combine: two inputs, writes the swapchain.
    let base = &p.before[0];
    assert!(base.inputs.is_empty());
    assert_ne!(base.output, Output::Swapchain); // writes an intermediate ("scene")

    let combine = p.before.iter().find(|c| c.name == "combine").unwrap();
    assert_eq!(combine.inputs.len(), 2);
    assert_eq!(combine.output, Output::Swapchain);

    // Every pass carries compiled SPIR-V.
    assert!(p.before.iter().all(|c| !c.module.spv.is_empty()));

    // Props merged across passes (threshold, intensity, exposure, speed, density…).
    assert!(p.properties.iter().any(|q| q.name == "threshold"));
    assert!(p.properties.iter().any(|q| q.name == "intensity"));

    // bloom is background-only: no after-content passes.
    assert!(p.after.is_empty());
}

#[test]
fn loads_vignette_after_content() {
    let p = load_pipeline(&examples().join("vignette"), WORKER).expect("vignette compiles");
    // A before-content backdrop + one after-content pass; no intermediate targets.
    assert_eq!(p.before.len(), 1);
    assert_eq!(p.after.len(), 1);
    assert!(p.targets.is_empty());

    assert_eq!(p.before[0].name, "backdrop");
    assert_eq!(p.before[0].output, Output::Swapchain);

    let vig = &p.after[0];
    assert_eq!(vig.name, "vignette");
    assert_eq!(vig.output, Output::Swapchain);
    // Its one input samples the composited scene (the CONTENT sentinel).
    assert_eq!(vig.inputs, vec![CONTENT]);
    assert!(!vig.module.spv.is_empty());
}

#[test]
fn loads_window_glow_requiring_window_geometry() {
    let p = load_pipeline(&examples().join("window-glow"), WORKER).expect("window-glow compiles");
    assert_eq!(p.before.len(), 1); // backdrop
    assert_eq!(p.after.len(), 1); // glow

    let glow = &p.after[0];
    assert_eq!(glow.name, "glow");
    assert_eq!(glow.inputs, vec![CONTENT]);
    assert!(glow.requires.world_set(), "glow requires window_geometry");
    // The backdrop requires nothing, so it costs nothing.
    assert!(p.before[0].requires.is_empty());
}

#[test]
fn loads_motion_blur_requiring_previous_frame() {
    let p = load_pipeline(&examples().join("motion-blur"), WORKER).expect("motion-blur compiles");
    // A pan-linked backdrop (before) + one after-content blur sampling content+history.
    assert_eq!(p.before.len(), 1);
    assert_eq!(p.before[0].name, "backdrop");
    assert_eq!(p.after.len(), 1);
    assert!(p.requires.has(Requirement::PreviousFrame), "motion-blur declares previous_frame");

    let blur = &p.after[0];
    assert_eq!(blur.name, "blur");
    assert_eq!(blur.output, Output::Swapchain);
    // Sorted binding names: cur -> content(1st), prev -> history(2nd).
    assert_eq!(blur.inputs, vec![CONTENT, HISTORY]);
    assert!(!blur.module.spv.is_empty());

    // A bundle that does not ask for history does not get the image.
    let vig = load_pipeline(&examples().join("vignette"), WORKER).unwrap();
    assert!(!vig.requires.has(Requirement::PreviousFrame));
    assert!(!vig.requires.textures());
}

#[test]
fn loads_glass_requiring_world_textures() {
    // Compiles the glass WGSL — including its `binding_array<texture_2d>` — through
    // the real naga compose → SPIR-V path. (load_pipeline itself does not gate on
    // the device feature; the producer `load_multipass` does.)
    let p = load_pipeline(&examples().join("glass"), WORKER).expect("glass compiles");
    assert_eq!(p.before.len(), 1); // backdrop
    assert_eq!(p.after.len(), 1); // glass
    assert!(p.requires.textures(), "glass requires world_textures");
    assert!(p.requires.whole_band(), "glass takes the whole band, not just windows");
    assert!(p.requires.has(Requirement::PreviousFrame), "glass samples the history backdrop");

    let glass = &p.after[0];
    assert_eq!(glass.name, "glass");
    assert!(glass.requires.world_set(), "glass requires world_geometry");
    assert!(glass.requires.textures());
    // Sorted binding names: content -> content(1st), history -> history(2nd).
    assert_eq!(glass.inputs, vec![CONTENT, HISTORY]);
    assert!(!glass.module.spv.is_empty());
}
