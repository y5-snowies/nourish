//! Manifest parsing: defaults, enum spellings, and rejection of bad shapes.

use compositor_pipeline_bundle_manifest_base::manifest::{parse, Format, Requirement, When};

const BLOOM: &str = r#"{
  "name": "bloom",
  "targets": {
    "scene":  { "format": "rgba16f", "scale": 1.0 },
    "bright": { "format": "rgba16f", "scale": 0.5 }
  },
  "passes": [
    { "name": "base",   "shader": "passes/base.wgsl",   "output": "scene", "when": "before-content" },
    { "name": "bright", "shader": "passes/bright.wgsl", "inputs": {"src":"scene"}, "output": "bright" },
    { "name": "blurH",  "shader": "passes/blur.wgsl",   "inputs": {"src":"bright"}, "output": "blur_a", "defines": {"DIR":"H"} },
    {
      "name": "vignette", "shader": "passes/vignette.wgsl", "when": "after-content",
      "inputs": {"scene":"content","bloom":"blur_a"}, "output": "output",
      "requires": ["window_geometry", "composited_scene"]
    }
  ]
}"#;

#[test]
fn parses_bloom_bundle() {
    let m = parse(BLOOM).expect("valid manifest");
    assert_eq!(m.name, "bloom");
    assert_eq!(m.version, 1); // defaulted
    assert_eq!(m.passes.len(), 4);
    assert_eq!(m.targets["bright"].scale, 0.5);
    assert_eq!(m.targets["scene"].format, Format::Rgba16f);
}

#[test]
fn defaults_apply() {
    let m = parse(r#"{ "name": "min", "passes": [ { "name": "p", "shader": "p.wgsl" } ] }"#)
        .expect("minimal manifest");
    let p = &m.passes[0];
    assert_eq!(p.output, "output"); // default target
    assert_eq!(p.when, When::BeforeContent); // default band
    assert!(p.inputs.is_empty() && p.requires.is_empty());
}

#[test]
fn enum_spellings_are_kebab_and_snake() {
    let m = parse(BLOOM).unwrap();
    let vig = m.passes.iter().find(|p| p.name == "vignette").unwrap();
    assert_eq!(vig.when, When::AfterContent);
    assert_eq!(
        vig.requires,
        vec![Requirement::WindowGeometry, Requirement::CompositedScene]
    );
}

#[test]
fn unknown_requirement_is_rejected() {
    // A typo'd requirement must not parse as "requires nothing": the whole point
    // of the list is that every engine cost has an entry behind it, so a silently
    // dropped entry is a cost with no author.
    let bad = r#"{ "name": "x", "passes": [
        { "name": "p", "shader": "p.wgsl", "requires": ["world_rects"] } ] }"#;
    assert!(parse(bad).is_err());
}

#[test]
fn unknown_field_is_rejected() {
    // deny_unknown_fields guards against typos in the manifest.
    let bad = r#"{ "name": "x", "passes": [], "pases": [] }"#;
    assert!(parse(bad).is_err());
}

#[test]
fn missing_passes_is_rejected() {
    assert!(parse(r#"{ "name": "x" }"#).is_err());
}

#[test]
fn category_is_optional_and_carried() {
    // A bundle may name its picker heading; one that does not leaves it to the
    // reader, which resolves the fallback ("User"). Absent must be `None` rather
    // than an empty string, so "did not say" and "said nothing" stay different.
    let named = r#"{ "name": "x", "category": "Multipass", "passes": [] }"#;
    assert_eq!(parse(named).expect("parses").category.as_deref(), Some("Multipass"));
    assert!(parse(r#"{ "name": "x", "passes": [] }"#).expect("parses").category.is_none());
}

#[test]
fn a_texture_defaults_to_srgb_and_requires_a_file() {
    // `srgb` defaults to TRUE because the common case is artwork, and artwork
    // sampled without linearising composites too bright. The declaration exists
    // for the other case — a LUT, a mask, a field — whose bytes are numbers and
    // must NOT be linearised, and where the wrong value renders something
    // plausible rather than failing.
    let m = parse(
        r#"{ "name": "x", "textures": { "art": { "file": "a.png" } }, "passes": [] }"#,
    )
    .expect("parses");
    let t = m.textures.get("art").expect("declared");
    assert_eq!(t.file, "a.png");
    assert!(t.srgb, "the default must be sRGB — see manifest::Texture");

    let linear = parse(
        r#"{ "name": "x", "textures": { "lut": { "file": "l.png", "srgb": false } },
             "passes": [] }"#,
    )
    .expect("parses");
    assert!(!linear.textures["lut"].srgb);

    // `file` is the one thing a texture cannot be without, so it is required
    // rather than defaulted to the texture's name — a name is an identifier the
    // shader uses and a path is where bytes live, and conflating them means
    // renaming a binding moves the file it reads.
    assert!(parse(r#"{ "name": "x", "textures": { "art": {} }, "passes": [] }"#).is_err());
}
