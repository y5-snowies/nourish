//! Embedded default UI font for iced surfaces.
//!
//! Registers Inter (vendor/font/) into iced's global font system and maps the
//! generic sans-serif family — which `iced_core::Font::DEFAULT` resolves
//! through — to it, so text renders on systems with no fonts installed (no
//! ttf-dejavu & co. needed). System fonts still load alongside, so per-glyph
//! fallback to installed fonts keeps working for scripts Inter lacks.
//!
//! [`install`] must run before ANY iced text is created or measured: iced's
//! font system is a lazy global, and whoever touches it first freezes what
//! the early shaping caches see. `main()` calls this at startup, ahead of
//! every engine/surface construction.

/// Inter roman variable font. cosmic-text drives the `wght` axis, so every
/// requested weight renders true from this one file. It has no italic axis:
/// italic styles render upright unless an italic VF is added alongside.
pub const INTER_FONT: &[u8] =
    include_bytes!("../../../../../vendor/font/Inter-VariableFont_opsz,wght.ttf");

/// Family name in the font's `name` table (what fontdb indexes it under).
pub const FAMILY: &str = "Inter";

/// Register Inter and make it the default sans-serif family. Never panics:
/// on any failure the compositor keeps running and shaping falls back to
/// whatever fonts the system provides.
pub fn install() {
    use iced_graphics::text::cosmic_text::fontdb::{Family, Query, Source};

    let Ok(mut system) = iced_graphics::text::font_system().write() else {
        warn!("default UI font: iced font system lock poisoned; skipping {FAMILY} install");
        return;
    };
    let db = system.raw().db_mut();
    db.load_font_data(INTER_FONT.to_vec());
    db.set_sans_serif_family(FAMILY);

    // A host-installed Inter would shadow the embedded copy (system fonts are
    // loaded first and same-family faces resolve first-come), making the UI
    // render whatever Inter version the host ships. Prune file-backed faces of
    // this family so every machine renders the exact embedded version; hosts
    // without Inter are unaffected.
    let shadowing: Vec<_> = db
        .faces()
        .filter(|f| {
            !matches!(f.source, Source::Binary(_))
                && f.families.iter().any(|(name, _)| name == FAMILY)
        })
        .map(|f| f.id)
        .collect();
    if !shadowing.is_empty() {
        info!("default UI font: removing {} host-installed {FAMILY} face(s) shadowing the embedded copy", shadowing.len());
        for id in shadowing {
            db.remove_face(id);
        }
    }

    // Prove the mapping took: resolve the generic sans-serif family exactly
    // like shaping will, and log which face won and where it lives. "embedded"
    // = our Inter bytes; "system file" = a disk font won instead (embed NOT in
    // effect); no face at all = text would render as tofu on this system.
    let face = db
        .query(&Query { families: &[Family::SansSerif], ..Query::default() })
        .and_then(|id| db.face(id));
    match face {
        Some(face) => {
            let family = face.families.first().map(|(n, _)| n.as_str()).unwrap_or("?");
            let source = match face.source {
                Source::Binary(_) => "embedded",
                Source::File(_) | Source::SharedFile(..) => "system file",
            };
            info!("default UI font: sans-serif -> {family} ({source}, {} KiB)", INTER_FONT.len() / 1024);
        }
        None => warn!("default UI font: sans-serif resolves to NO face — embedded {FAMILY} failed to load"),
    }
}
