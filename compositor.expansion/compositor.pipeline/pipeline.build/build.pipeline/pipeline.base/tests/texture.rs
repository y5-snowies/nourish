//! Bundle textures: that a declared image resolves into a pass's inputs like any
//! other, and that the ways it can go wrong are refused at LOAD.
//!
//! The one that has no other guard is the id. A texture is a bundle source that
//! is not text, so it does not travel through the source hash by default — and
//! left out, editing a sprite sheet and reloading yields an identical pipeline
//! id, `GraphExec::prepare` sees no change, nothing reallocates, and the new art
//! never reaches the GPU. Exactly the failure `tests/reload.rs` exists for, one
//! input later, and just as silent: the file on disk is new, the desktop is not.
//!
//! The refusal cases use `.err().unwrap_or_else(panic!)` rather than
//! `expect_err`: that wants `Debug` on the Ok side, and a `CompiledPipeline`
//! carries SPIR-V and decoded images — dumping one on failure would bury the
//! message the assertion exists to show.

use compositor_pipeline_build_pipeline_base::pipeline::{load_pipeline, TEXTURE};
use std::path::Path;

const WORKER: compositor_pipeline_build_place_base::place::Env =
    compositor_pipeline_build_place_base::place::Env { worker: true };

/// A solid RGBA PNG of the given size and colour.
fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let buf = image::RgbaImage::from_pixel(w, h, image::Rgba(rgba));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(buf)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("encode png");
    out.into_inner()
}

/// A bundle whose one pass samples `sheet`, plus whatever extra manifest text the
/// caller wants folded in (targets, a second texture, a bad path…).
fn write_bundle(dir: &Path, extra: &str, image_bytes: &[u8]) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("passes")).expect("mkdir");
    std::fs::write(
        dir.join("passes/p.wgsl"),
        "@group(0) @binding(0) var samp: sampler;\n\
         @group(0) @binding(1) var sheet: texture_2d<f32>;\n\
         @fragment\n\
         fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {\n\
         \x20   return textureSampleLevel(sheet, samp, vec2<f32>(0.5), 0.0);\n\
         }\n",
    )
    .expect("write pass");
    std::fs::write(dir.join("art.png"), image_bytes).expect("write png");
    std::fs::write(
        dir.join("pipeline.json"),
        format!(
            r#"{{"name":"t","version":1,
                 "textures":{{"sheet":{{"file":"art.png"}}}},{extra}
                 "passes":[{{"name":"p","shader":"passes/p.wgsl",
                             "inputs":{{"sheet":"sheet"}},"output":"output"}}]}}"#
        ),
    )
    .expect("write manifest");
}

/// A scratch bundle directory, per case, so the cases can run in parallel.
fn dir(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("y5-test-texture-{name}"))
}

/// The whole feature in one assertion: a name in `textures` is usable in `inputs`
/// exactly where a target name would be, and lands as a `TEXTURE` sentinel so the
/// executor binds the uploaded image rather than target #0.
#[test]
fn a_declared_texture_resolves_into_the_pass_inputs() {
    let d = dir("resolve");
    write_bundle(&d, "", &png(4, 4, [255, 0, 0, 255]));
    let cp = load_pipeline(&d, WORKER).expect("loads");

    assert_eq!(cp.textures.len(), 1, "the declared texture was not decoded");
    assert_eq!((cp.textures[0].width, cp.textures[0].height), (4, 4));
    assert!(cp.textures[0].srgb, "srgb defaults to true — see manifest::Texture");
    assert_eq!(
        cp.before[0].inputs,
        vec![TEXTURE],
        "the input resolved to something other than texture #0 — as a plain index it \
         would bind whatever target happens to sit at that slot",
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// THE trap. Same manifest, same WGSL, different PNG: the pipeline id must move,
/// or a reload after editing the art is a no-op with nothing to show for it.
#[test]
fn editing_a_texture_changes_the_pipeline_id() {
    let d = dir("reload");
    write_bundle(&d, "", &png(4, 4, [255, 0, 0, 255]));
    let before = load_pipeline(&d, WORKER).expect("loads");

    // Only the image file differs — the case `Shader::reload` serves after an
    // author repaints a sprite sheet.
    std::fs::write(d.join("art.png"), png(4, 4, [0, 255, 0, 255])).expect("repaint");
    let after = load_pipeline(&d, WORKER).expect("reloads");
    assert_ne!(
        before.id, after.id,
        "the pipeline id survived a texture edit — `GraphExec::prepare` would see no change, \
         keep the image it uploaded from the OLD bytes, and the repaint would never appear",
    );

    // …and identical bytes are still the same pipeline, so an export that rewrites
    // a file unchanged does not re-upload every texture and recompile every pass.
    std::fs::write(d.join("art.png"), png(4, 4, [0, 255, 0, 255])).expect("rewrite");
    let again = load_pipeline(&d, WORKER).expect("loads");
    assert_eq!(after.id, again.id, "identical bytes must be the same pipeline");
    let _ = std::fs::remove_dir_all(&d);
}

/// `srgb: false` reaches the loader, because it is the one attribute whose wrong
/// value is silently wrong output rather than an error: a LUT linearised on
/// sample is corrupt at every entry and still renders.
#[test]
fn srgb_false_survives_to_the_compiled_pipeline() {
    let d = dir("linear");
    write_bundle(&d, "", &png(2, 2, [128, 128, 128, 255]));
    std::fs::write(
        d.join("pipeline.json"),
        r#"{"name":"t","version":1,
            "textures":{"sheet":{"file":"art.png","srgb":false}},
            "passes":[{"name":"p","shader":"passes/p.wgsl",
                       "inputs":{"sheet":"sheet"},"output":"output"}]}"#,
    )
    .expect("write manifest");
    let cp = load_pipeline(&d, WORKER).expect("loads");
    assert!(!cp.textures[0].srgb, "`srgb: false` was dropped between manifest and pipeline");
    let _ = std::fs::remove_dir_all(&d);
}

/// A path out of the bundle is a bundle that can read any file the compositor
/// can and paint it on the desktop.
#[test]
fn a_texture_path_may_not_escape_the_bundle() {
    let d = dir("escape");
    write_bundle(&d, "", &png(2, 2, [1, 2, 3, 4]));
    std::fs::write(
        d.join("pipeline.json"),
        r#"{"name":"t","version":1,
            "textures":{"sheet":{"file":"../../../etc/hostname"}},
            "passes":[{"name":"p","shader":"passes/p.wgsl",
                       "inputs":{"sheet":"sheet"},"output":"output"}]}"#,
    )
    .expect("write manifest");
    let err = load_pipeline(&d, WORKER).err().unwrap_or_else(|| panic!("an escaping texture path must not load"));
    assert!(err.contains("inside the bundle"), "say what the rule is: {err}");
    let _ = std::fs::remove_dir_all(&d);
}

/// Targets and textures are named from ONE namespace in `inputs`, so a name in
/// both is ambiguous — and resolved by precedence it would be silently ambiguous.
#[test]
fn a_name_that_is_both_a_target_and_a_texture_is_refused() {
    let d = dir("collide");
    write_bundle(
        &d,
        r#""targets":{"sheet":{"format":"rgba16f"}},"#,
        &png(2, 2, [1, 2, 3, 4]),
    );
    let err = load_pipeline(&d, WORKER).err().unwrap_or_else(|| panic!("a colliding name must not load"));
    assert!(err.contains("sheet"), "name the offender: {err}");
    let _ = std::fs::remove_dir_all(&d);
}

/// Rendering into a texture is not a thing that can happen, and the author is
/// told THAT rather than being told a name they clearly declared is unknown.
#[test]
fn a_texture_cannot_be_a_pass_output() {
    let d = dir("output");
    write_bundle(&d, "", &png(2, 2, [1, 2, 3, 4]));
    std::fs::write(
        d.join("pipeline.json"),
        r#"{"name":"t","version":1,
            "textures":{"sheet":{"file":"art.png"}},
            "passes":[{"name":"p","shader":"passes/p.wgsl","output":"sheet"},
                      {"name":"o","shader":"passes/p.wgsl",
                       "inputs":{"sheet":"sheet"},"output":"output"}]}"#,
    )
    .expect("write manifest");
    let err = load_pipeline(&d, WORKER).err().unwrap_or_else(|| panic!("writing a texture must not load"));
    assert!(err.contains("read-only"), "say why, not just no: {err}");
    let _ = std::fs::remove_dir_all(&d);
}

/// A file that is not an image at all fails the bundle, rather than being bound
/// as a blank that shows up as a black patch several frames later.
#[test]
fn a_file_that_is_not_an_image_fails_the_bundle() {
    let d = dir("garbage");
    write_bundle(&d, "", b"this is not a png");
    let err = load_pipeline(&d, WORKER).err().unwrap_or_else(|| panic!("a non-image must not load"));
    assert!(err.contains("sheet"), "name the texture, not the path: {err}");
    let _ = std::fs::remove_dir_all(&d);
}

/// The per-pass image cap is the real ceiling, and it is about descriptor
/// bindings rather than about textures — so it counts targets too.
#[test]
fn a_pass_may_not_bind_more_images_than_the_descriptor_floor_allows() {
    use compositor_pipeline_bundle_graph_base::graph::MAX_PASS_IMAGES;
    let d = dir("cap");
    write_bundle(&d, "", &png(2, 2, [1, 2, 3, 4]));
    let n = MAX_PASS_IMAGES + 1;
    let targets: Vec<String> =
        (0..n).map(|i| format!(r#""t{i}":{{"format":"rgba16f"}}"#)).collect();
    let feeders: Vec<String> = (0..n)
        .map(|i| format!(r#"{{"name":"f{i}","shader":"passes/p.wgsl","output":"t{i}"}}"#))
        .collect();
    let inputs: Vec<String> = (0..n).map(|i| format!(r#""i{i}":"t{i}""#)).collect();
    std::fs::write(
        d.join("pipeline.json"),
        format!(
            r#"{{"name":"t","version":1,
                 "targets":{{{}}},
                 "passes":[{},
                   {{"name":"o","shader":"passes/p.wgsl",
                     "inputs":{{{}}},"output":"output"}}]}}"#,
            targets.join(","),
            feeders.join(","),
            inputs.join(","),
        ),
    )
    .expect("write manifest");
    let err = load_pipeline(&d, WORKER).err().unwrap_or_else(|| panic!("past the cap must not load"));
    assert!(err.contains("at most"), "say the limit: {err}");
    let _ = std::fs::remove_dir_all(&d);
}

/// Refused, not trimmed — the storage budget's rule, for the same reason: a
/// bundle handed a smaller image than it declared samples the wrong pixels
/// everywhere and has no way to find out.
#[test]
fn an_oversized_image_is_refused_by_its_header() {
    use compositor_pipeline_compile_decode_base::decode::{decode, MAX_EDGE};
    // Not written to disk: the point is that the refusal reads the HEADER, so it
    // never reaches the allocation that would be the actual problem.
    let big = png(MAX_EDGE + 1, 1, [0, 0, 0, 255]);
    let err = decode("sheet", &big, true).err().unwrap_or_else(|| panic!("past the edge limit must not decode"));
    assert!(err.contains("before decoding"), "say when it was caught: {err}");
}
