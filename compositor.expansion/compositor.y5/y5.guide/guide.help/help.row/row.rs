//! The curated help list: which shortcuts a first-time user is shown, and the
//! keys they are bound to RIGHT NOW.
//!
//! Every combo is read back out of the same registries the live handlers are
//! built from (`canvas.input.keyboard`), keyed by the stable binding id — so a
//! rebind in the settings Keys tab moves this list with it, and a row whose
//! binding was disabled or renamed away simply disappears instead of lying.
//! The overview is the one hardcoded entry: its Super+Tab is not registry-backed
//! (see `overview.input.keyboard`), so there is nothing to look it up in.

use compositor_model_environment_keybinding_base::base::KeyBindings;
use compositor_y5_canvas_input_keyboard::navigator;
use compositor_y5_guide_help_view::HelpRow;

/// `(binding id, variant binding id, label)`, in the order shown. Launching an
/// app is first: it is the one thing a user on an empty canvas actually wants.
///
/// A variant id pairs two gestures that differ ONLY by a held modifier onto one
/// row — the panel then shows the base combo plus the extra modifier, and the
/// label names both halves in that order. Nine keys read as five ideas.
const SHOWN: &[(&str, &str, &str)] = &[
    ("launcher", "", "Launcher"),
    // Super+Alt alone selects ONE window; only the Shift variant drags a box
    // (`press.rs` starts a `SelectBox` grab on `Append`), so the box is the
    // variant here, not the base.
    ("grab_select", "grab_select_add", "Select / box"),
    ("grab_move", "grab_scale", "Move window / resize"),
    ("grab_hand", "", "Hand tool"),
    ("zoom_focus", "zoom_fit", "Zoom"),
];

/// Nested (winit) substitutes Right Ctrl for Super — the host owns Super and
/// never forwards it (`seat.keyboard/keyboard.input::shortcut_modifiers`). The
/// registries format the UNsubstituted combo, so a nested session would otherwise
/// be told to press a key that does nothing.
///
/// ONLY `Super` moves. A `Ctrl` in a combo still means Ctrl — the LEFT one, which
/// stays free for the clients inside the session and for genuinely Ctrl-based
/// compositor bindings. `Super+Ctrl+Alt` (the Hand tool) therefore reads
/// `RCtrl+Ctrl+Alt`: two different physical keys, which is exactly what it is.
fn remap(combo: &str, nested: bool) -> String {
    if !nested {
        return combo.to_string();
    }
    let mut parts: Vec<&str> = Vec::new();
    for token in combo.split('+') {
        let token = if token == "Super" { "RCtrl" } else { token };
        if !parts.contains(&token) {
            parts.push(token);
        }
    }
    parts.join("+")
}

/// What `variant` adds to `base`, as `+Shift` — the one modifier that switches
/// the gesture. Computed from the live combos rather than written down, so it
/// stays true if either side is rebound. `None` when they don't differ by pure
/// addition (a rebind made them unrelated), which just drops the second pill.
fn added(base: &str, variant: &str) -> Option<String> {
    let extra: Vec<&str> = variant.split('+').filter(|t| !base.split('+').any(|b| b == *t)).collect();
    if extra.is_empty() || base.split('+').any(|b| !variant.split('+').any(|v| v == b)) {
        return None;
    }
    Some(format!("+{}", extra.join("+")))
}

/// Build the panel's rows against the live overrides.
pub fn rows(nested: bool, overrides: &KeyBindings) -> Vec<HelpRow> {
    let mut known = navigator::registry(overrides);
    known.extend(navigator::fixed());
    let combo = |id: &str| known.iter().find(|r| r.id == id).map(|r| r.combo.clone());

    let mut out: Vec<HelpRow> = Vec::new();
    for (id, variant, label) in SHOWN {
        // Remapped BEFORE the diff: nested collapses Super onto Ctrl, which can
        // merge two tokens into one, and the difference has to be taken on what
        // the user will actually press.
        let Some(base) = combo(id).map(|c| remap(&c, nested)) else { continue };
        let extra = combo(variant).map(|v| remap(&v, nested)).and_then(|v| added(&base, &v));
        out.push(HelpRow { label: (*label).to_string(), combo: base, extra });
    }
    // Not registry-backed (hardcoded in the overview's own key handler), and the
    // one shortcut the install guide calls the rescue — so it is always shown.
    out.push(HelpRow {
        label: "Overview".to_string(),
        combo: remap("Super+Tab", nested),
        extra: None,
    });
    out
}

/// The tooltip under the Settings cog: the bare combo that gets you there.
/// Settings is a TAB of the overview rather than a binding of its own, so this
/// is the overview's key — the closest thing to a shortcut that exists.
pub fn settings_hint(nested: bool) -> String {
    remap("Super+Tab", nested)
}
