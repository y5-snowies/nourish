//! `y5.compositor.settings` — interactive setup tool for the compositor.
//!
//! A normal run shows a menu:
//!   1. Settings        — edit ~/.config/y5.compositor/settings.json (every field;
//!                        the compositor has no defaults and panics on a partial file)
//!   2. Set preferences — pick a per-monitor preferred mode (preferences.json)
//! Escape navigates back; Escape at the menu exits.
//!
//!   y5.compositor.settings                  # interactive menu
//!   y5.compositor.settings --config-file=P  # interactive, settings path P
//!   y5.compositor.settings --installer      # installer setup: Settings only, no menu,
//!                                           #   no preferences
//!   y5.compositor.settings --write-default  # non-interactive, write the template

mod drm_probe;
mod edit;
mod menu;
mod persist;
mod preferences;
mod prompt;
mod select;
mod template;
mod term;

use compositor_model_environment_config_base::base::{resolve_path, Environment};
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{}", USAGE);
        return;
    }
    let write_default = args.iter().any(|a| a == "--write-default");
    let installer = args.iter().any(|a| a == "--installer");

    // `resolve_path` is the same logic the compositor uses (honors --config-file=,
    // else $XDG_CONFIG_HOME/.config), so the tool and compositor never disagree.
    let path = resolve_path();

    // Pre-fill from an existing file when present; fall back to the template.
    let base = load_existing(&path).unwrap_or_else(template::default_settings);

    if write_default {
        // Non-interactive: write the canonical template and exit.
        persist::write_settings(&path, &template::default_settings());
        return;
    }

    if installer {
        // Initial setup driven by the installer: straight into Settings, no menu, no
        // preferences.
        let settings = edit::interactive(base);
        persist::write_settings(&path, &settings);
        return;
    }

    // Normal interactive run: the menu owns the Settings + preferences loop.
    menu::run(&path, base);
}

/// Load and parse an existing settings file, or `None` if absent/unreadable/invalid.
///
/// Migrated first, with the SAME `config.base::migrate` the compositor runs on
/// load. Without it this tool would show an older file's authored values while
/// the compositor is already acting on the migrated ones — and a save would then
/// write the stale value back, undoing the migration permanently.
fn load_existing(path: &Path) -> Option<Environment> {
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed = serde_json::from_str::<serde_json::Value>(&raw).map(|mut v| {
        compositor_model_environment_config_base::base::migrate(&mut v);
        v
    });
    match parsed.and_then(serde_json::from_value) {
        Ok(env) => Some(env),
        Err(e) => {
            eprintln!("note: existing {} is invalid ({e}); starting from defaults.", path.display());
            None
        }
    }
}

const USAGE: &str = "\
y5.compositor.settings — set up the compositor

USAGE:
    y5.compositor.settings [OPTIONS]

A normal run shows a menu: Settings (author settings.json) and Set preferences
(per-monitor preferred mode -> preferences.json). Escape navigates back.

OPTIONS:
    --config-file=<PATH>   Use PATH for settings.json instead of the default location.
    --installer            Installer setup: go straight to Settings (no menu, no
                           preferences).
    --write-default        Non-interactive: write the canonical default settings.
    -h, --help             Show this help.
";
