//! The multipass bundles that ship compiled INTO the binary.
//!
//! `shader.builtin` already embeds the single-file WGSL worlds, but a `Builtin` is
//! one `&'static str` and a multipass bundle is a manifest plus a tree of pass and
//! module sources. Those could only live on disk, which meant the `mp-*` set was
//! not shipped at all: it existed in `document/shader-examples`, and a user saw it
//! only after running `document/shader-examples/install-shaders.sh` by hand.
//!
//! So this is the same table, widened to a FILE per row instead of a bundle per
//! row, over the bundles in `bundles/` next door. They live HERE and not in
//! `document/shader-examples` because `include_str!` makes them a build input:
//! shipped source belongs beside the code that ships it, not in a documentation
//! tree a packager is entitled to drop. The examples tree keeps the bundles that
//! demonstrate the mechanism; these are the ones the compositor offers.
//!
//! # How it reaches the loader
//!
//! The loader is written against `bundle.join(rel)` and `std::fs::read_to_string`
//! throughout. Rather than fork every one of those, an embedded selection resolves
//! to a path under [`ROOT`] — a directory that does not exist — and [`read_file`]
//! intercepts the read before anything opens it. Disk bundles take the same call
//! and are unaffected, so there is one code path and the embedded set cannot
//! develop its own loader behaviour.
//!
//! Paths are normalised, which is not incidental: `mp-invert` and friends share
//! `mp-parallax`'s backdrop through `../mp-parallax/passes/backdrop.wgsl`, so the
//! keys only line up once `..` is resolved.

use std::path::{Path, PathBuf};

/// The virtual directory embedded bundles resolve under. Never opened — every
/// read goes through [`read_file`] — but it must BE a path, because `resolve_ref`
/// returns one. Angle brackets keep it clear of any real bundle folder.
pub const ROOT: &str = "<embedded>";

/// Selection ids carry `shader.builtin`'s prefix, so an embedded bundle can never
/// collide with a disk folder of the same name and is recognised without a readdir.
pub const PREFIX: &str = "builtin:";

macro_rules! embedded {
    ($($bundle:literal / $file:literal),* $(,)?) => {
        &[$((
            concat!($bundle, "/", $file),
            include_str!(concat!("bundles/", $bundle, "/", $file)),
        )),*]
    };
}

/// Every embedded file, keyed by `<bundle>/<path within it>`.
const FILES: &[(&str, &str)] = embedded![
    "mp-parallax"/"pipeline.json",
    "mp-parallax"/"lib/parallax.wgsl",
    "mp-parallax"/"passes/backdrop.wgsl",
    "mp-grayscale"/"pipeline.json",
    "mp-grayscale"/"passes/gray.wgsl",
    "mp-invert"/"pipeline.json",
    "mp-invert"/"passes/invert.wgsl",
    "mp-levels"/"pipeline.json",
    "mp-levels"/"passes/levels.wgsl",
    "mp-colorblind"/"pipeline.json",
    "mp-colorblind"/"passes/vision.wgsl",
    "mp-effects"/"pipeline.json",
    "mp-effects"/"passes/effects.wgsl",
    "mp-effects"/"passes/finish.wgsl",
    "mp-crt"/"pipeline.json",
    "mp-crt"/"lib/warp.wgsl",
    "mp-crt"/"passes/crt.wgsl",
];

/// The embedded bundle names, in picker order: the plain backdrop first, then the
/// filters that build on it, then the two that do their own thing.
pub const BUNDLES: &[&str] = &[
    "mp-parallax",
    "mp-grayscale",
    "mp-invert",
    "mp-levels",
    "mp-colorblind",
    "mp-effects",
    "mp-crt",
];

/// The bundle a selection value names, if it names an embedded one.
pub fn bundle_of(value: &str) -> Option<&'static str> {
    let name = value.strip_prefix(PREFIX)?;
    BUNDLES.iter().copied().find(|b| *b == name)
}

/// The selection id for an embedded bundle name.
pub fn id_of(bundle: &str) -> String {
    format!("{PREFIX}{bundle}")
}

/// Where `bundle_of` sends a selection: a path under [`ROOT`].
pub fn path_of(bundle: &str) -> PathBuf {
    Path::new(ROOT).join(bundle)
}

/// The embedded bundle this path refers to, if it is one of ours.
pub fn name_of(bundle: &Path) -> Option<&str> {
    bundle.strip_prefix(ROOT).ok()?.to_str()
}

/// Where the bundle sources live in the tree. For tests and tooling that want the
/// files themselves; the compositor never reads them from here.
pub fn source_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("bundles")
}

/// Read `rel` out of `bundle`, from the embedded table or from disk.
///
/// THE seam. Every bundle-relative read in the loader goes through here, which is
/// what keeps an embedded bundle loading by exactly the same code as a disk one.
pub fn read_file(bundle: &Path, rel: &str) -> Result<String, String> {
    let Some(name) = name_of(bundle) else {
        return std::fs::read_to_string(bundle.join(rel))
            .map_err(|e| format!("read {rel}: {e}"));
    };
    read(name, rel)
        .map(str::to_string)
        .ok_or_else(|| format!("embedded {name}: no such file '{rel}'"))
}

/// Read `rel` out of `bundle` as raw bytes, for the files that are not text.
///
/// The embedded table is `&'static str` — every entry is a source file — so an
/// embedded bundle has no bytes to hand back and says so plainly rather than
/// reporting the file as missing. None of the shipped bundles declares a texture,
/// so this is a description of the table's shape, not a restriction anything is
/// currently hitting: shipping one means adding an `include_bytes!` table beside
/// [`FILES`] and reading it here.
pub fn read_bytes(bundle: &Path, rel: &str) -> Result<Vec<u8>, String> {
    if let Some(name) = name_of(bundle) {
        return Err(format!(
            "embedded {name}: '{rel}' — compiled-in bundles carry source files only, so they \
             cannot declare textures"
        ));
    }
    std::fs::read(bundle.join(rel)).map_err(|e| format!("read {rel}: {e}"))
}

/// Whether `bundle` carries a multipass manifest — the test that decides between
/// the multipass and single-pass loaders.
pub fn has_manifest(bundle: &Path) -> bool {
    match name_of(bundle) {
        Some(name) => read(name, "pipeline.json").is_some(),
        None => bundle.join("pipeline.json").is_file(),
    }
}

/// Look up one embedded source, resolving `.`/`..` so cross-bundle `shader` paths
/// land on the same key their target is stored under.
fn read(bundle: &str, rel: &str) -> Option<&'static str> {
    let mut parts: Vec<&str> = Vec::new();
    for seg in bundle.split('/').chain(rel.split('/')) {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let key = parts.join("/");
    FILES.iter().find(|(p, _)| *p == key).map(|(_, s)| *s)
}
