//! Enumerate the xkb keyboard layouts available on this system, for the Language
//! tab's "add a language" picker. Parses the `! layout` section of the xkb rules
//! list (`/usr/share/X11/xkb/rules/evdev.lst`) — the same catalogue libxkbcommon
//! reads — into `(code, human name)` pairs. Falls back to a small built-in list when
//! the file is absent (stripped image / non-standard install). Loaded ONCE into the
//! settings UI state, so the file read is not on any render path.

/// The rules list shipped by xkeyboard-config; its `! layout` section is the
/// authoritative list of layout codes for this machine.
const EVDEV_LST: &str = "/usr/share/X11/xkb/rules/evdev.lst";

/// All available layouts as `(code, human name)`, in the file's order (which groups
/// related layouts sensibly). Never empty — falls back to [`fallback`].
pub fn available() -> Vec<(String, String)> {
    read(EVDEV_LST).filter(|v| !v.is_empty()).unwrap_or_else(fallback)
}

/// Parse the `! layout` section of an evdev-style `.lst`. Sections are introduced by
/// `! <name>` lines; within `! layout`, each row is `code<whitespace>Human name`.
fn read(path: &str) -> Option<Vec<(String, String)>> {
    let raw = std::fs::read_to_string(path).ok()?;
    let mut out = Vec::new();
    let mut in_layout = false;
    for line in raw.lines() {
        let t = line.trim();
        if let Some(section) = t.strip_prefix('!') {
            in_layout = section.trim() == "layout";
            continue;
        }
        if !in_layout || t.is_empty() {
            continue;
        }
        if let Some((code, name)) = t.split_once(char::is_whitespace) {
            out.push((code.to_string(), name.trim().to_string()));
        }
    }
    Some(out)
}

/// A minimal built-in list so the picker still works if the rules file is missing.
fn fallback() -> Vec<(String, String)> {
    [
        ("us", "English (US)"),
        ("gb", "English (UK)"),
        ("il", "Hebrew"),
        ("ara", "Arabic"),
        ("de", "German"),
        ("fr", "French"),
        ("es", "Spanish"),
        ("it", "Italian"),
        ("ru", "Russian"),
        ("se", "Swedish"),
    ]
    .iter()
    .map(|(c, n)| (c.to_string(), n.to_string()))
    .collect()
}
