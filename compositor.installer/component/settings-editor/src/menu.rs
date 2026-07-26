//! The top-level menu shown on a normal interactive run: choose between editing
//! `settings.json` (Settings) and editing per-monitor preferences (Set preferences).
//! Escape at the menu exits. The installer flow does NOT use this — it goes straight
//! into Settings (see `main::run_installer`).

use crate::select::{select_list, Item};
use crate::term::Nav;
use crate::{edit, preferences, persist};
use compositor_model_environment_config_base::base::Environment;
use std::path::Path;

/// Loop the menu until the user escapes. `base` is the starting settings (existing
/// file or template); `path` is where Settings writes.
pub fn run(path: &Path, base: Environment) {
    // Keep edits across re-entry to Settings within one session.
    let mut settings = base;
    let items = [
        Item::new("Settings", "renderer, GPU, capture, logging…  (settings.json)"),
        Item::new("Set preferences", "per-monitor preferred mode  (preferences.json)"),
    ];
    loop {
        match select_list("y5.compositor.settings", &items, None, true) {
            Nav::Selected(0) => {
                settings = edit::interactive(settings);
                persist::write_settings(path, &settings);
            }
            Nav::Selected(1) => preferences::run(),
            _ => return, // Selected(>=2) is unreachable; Back/Eof exits.
        }
    }
}
