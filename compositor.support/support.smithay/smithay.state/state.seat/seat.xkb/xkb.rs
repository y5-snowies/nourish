//! Keyboard layout (xkb) for the seat — the single home for the mapping from the
//! persisted `KeyboardLayout` preference to smithay's `XkbConfig`, plus loading the
//! preference and applying a config to a live keyboard.
//!
//! The seat factory calls [`load`] + [`checked_config`] so the keyboard comes up
//! with the user's layout at startup; the settings window calls [`apply`] to
//! hot-reload it. `Env` leaves the config empty so libxkbcommon reads the
//! `XKB_DEFAULT_*` environment variables (the historical default); `Manual` uses the
//! ordered `layouts` list joined with commas plus the preset switch `grp:` option
//! (variant/rules/model are left to the xkb default — set those via the environment).
//!
//! CRASH SAFETY: an unavailable layout (e.g. `he` instead of `il`) fails to compile
//! into a keymap. Every place this feeds smithay — startup `add_keyboard` and live
//! `set_xkb_config` — goes through [`checked_config`], which compiles the config in a
//! throwaway context first and falls back to the us default on failure, so a bad
//! (possibly persisted) layout can never panic the compositor.
use compositor_model_environment_preference_base::base::{self as pref, KeyboardLayout, LayoutSource};
use smithay::input::SeatHandler;
use smithay::input::keyboard::{KeyboardHandle, XkbConfig, xkb};

/// Load the keyboard-layout preference fresh from preferences.json. A missing key
/// yields the default (`Env`), so behaviour is unchanged for existing configs.
pub fn load() -> KeyboardLayout {
    pref::load().keyboard
}

/// The comma-joined layout list for `Manual` (owned, since it's a fresh join); empty
/// for `Env`. Callers hold this so [`config`] can borrow it into the `XkbConfig`.
pub fn layout_csv(k: &KeyboardLayout) -> String {
    match k.source {
        LayoutSource::Env => String::new(),
        LayoutSource::Manual => k.layouts.join(","),
    }
}

/// Map a layout preference to an [`XkbConfig`], borrowing the pre-joined `layout`
/// CSV from [`layout_csv`]. `Env` → empty (reads `XKB_DEFAULT_*`); `Manual` → the
/// joined layouts + the preset switch `grp:` option. NOT compile-checked — prefer
/// [`checked_config`] anywhere the result is handed to smithay.
pub fn config<'a>(k: &KeyboardLayout, layout: &'a str) -> XkbConfig<'a> {
    match k.source {
        LayoutSource::Env => XkbConfig::default(),
        LayoutSource::Manual => XkbConfig {
            layout,
            options: k.switch.grp_option().map(str::to_string),
            ..Default::default()
        },
    }
}

/// Does `cfg` compile into a keymap? Builds a throwaway xkb context/keymap (the same
/// thing smithay does inside `add_keyboard`/`set_xkb_config`) and reports success,
/// WITHOUT touching the live keyboard — so we can validate before wiring it in.
fn compiles(cfg: &XkbConfig) -> bool {
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    xkb::Keymap::new_from_names(
        &ctx,
        cfg.rules,
        cfg.model,
        cfg.layout,
        cfg.variant,
        cfg.options.clone(),
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .is_some()
}

/// Like [`config`], but compile-checked: if the requested layout can't be compiled
/// (unavailable code, bad option, …) it warns and returns `XkbConfig::default()`
/// (which reads `XKB_DEFAULT_*`, i.e. the us default) instead. This is the ONLY
/// config that should reach smithay, so an invalid layout degrades to a working
/// keymap rather than crashing.
pub fn checked_config<'a>(k: &KeyboardLayout, layout: &'a str) -> XkbConfig<'a> {
    let cfg = config(k, layout);
    if compiles(&cfg) {
        cfg
    } else {
        warn!("xkb: layout {:?} (switch {:?}) failed to compile — falling back to the default keymap", layout, k.switch);
        XkbConfig::default()
    }
}

/// Apply a layout preference to a live keyboard: recompiles the keymap and
/// rebroadcasts it to the focused client. Routed through [`checked_config`], so a
/// malformed `Manual` config falls back to the default keymap (a warn is logged)
/// instead of crashing or silently keeping a stale layout. `keyboard` must be an
/// OWNED handle (clone it via `get_keyboard()`) so `data` can be borrowed mutably
/// alongside it.
pub fn apply<D: SeatHandler + 'static>(keyboard: &KeyboardHandle<D>, data: &mut D, k: &KeyboardLayout) {
    let csv = layout_csv(k);
    let _ = keyboard.set_xkb_config(data, checked_config(k, &csv));
}
