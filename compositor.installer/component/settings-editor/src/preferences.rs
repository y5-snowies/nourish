//! The "Set preferences" flow: help the user pick a per-monitor preferred mode and
//! persist it to `preferences.json`. Reuses the compositor's own preferences schema +
//! atomic I/O (`preference.base`) so what we write is exactly what the compositor
//! reads, and the DRM probe (`drm_probe`) for the connected monitors and their modes.
//!
//! Reached only from the top-level menu (never the installer flow), so Escape/back is
//! always available: Esc at the monitor list returns to the menu; Esc at the mode
//! list returns to the monitor list.

use crate::drm_probe::{self, ProbedMonitor};
use crate::prompt::ask;
use crate::select::{select_list, Item};
use crate::term::Nav;
use compositor_model_environment_preference_base::base as pref;
use compositor_model_environment_preference_base::base::{ModeRequest, Preference};

/// Run the preferences editor until the user backs out of the monitor list. The
/// monitor marked `*` is the **default output**: the compositor uses the first entry
/// in the `outputs` array (see `display.base`'s `profiles.first()`), so "set as
/// default" simply moves a monitor to the front.
pub fn run() {
    let mut prefs = pref::load();
    let monitors = drm_probe::probe();

    if monitors.is_empty() {
        println!(
            "\nNo monitors could be probed (need read access to /dev/dri/card*, and a\n\
             connected display). Falling back to manual entry."
        );
        manual_entry(&mut prefs);
        return;
    }

    loop {
        let items: Vec<Item> = monitors
            .iter()
            .map(|m| Item::new(m.label.clone(), saved_hint(&prefs, &m.identity_key)))
            .collect();
        // The `*` marks the current default: the monitor matching the first profile.
        let default = default_index(&prefs, &monitors);
        match select_list("Change monitor preferences", &items, default, true) {
            Nav::Selected(i) => monitor_menu(&mut prefs, &monitors[i]),
            Nav::Back => return,
        }
    }
}

/// Per-monitor actions: set its preferred mode, or make it the default output. Esc
/// returns to the monitor list.
fn monitor_menu(prefs: &mut Preference, mon: &ProbedMonitor) {
    loop {
        let is_default = prefs.outputs.first().and_then(|p| p.identity.as_deref()) == Some(&mon.identity_key);
        let actions = [
            Item::new("Set preferred mode", saved_hint(prefs, &mon.identity_key)),
            Item::new("Set as default output", if is_default { "already default" } else { "" }),
        ];
        match select_list(&format!("{} — preferences", mon.label), &actions, None, true) {
            Nav::Selected(0) => set_mode(prefs, mon),
            Nav::Selected(1) => {
                pref::set_default(&mut prefs.outputs, &mon.identity_key);
                match pref::save(prefs) {
                    Ok(()) => println!("\n{} is now the default output ✓", mon.label),
                    Err(e) => eprintln!("\nfailed to save preferences: {e}"),
                }
            }
            _ => return,
        }
    }
}

/// Show one monitor's advertised modes and persist the chosen one. Esc returns to the
/// per-monitor menu without changing anything.
fn set_mode(prefs: &mut Preference, mon: &ProbedMonitor) {
    let items: Vec<Item> = mon
        .modes
        .iter()
        .map(|m| {
            let pref = if m.preferred { "preferred" } else { "" };
            Item::new(format!("{}x{} @ {}Hz", m.width, m.height, m.refresh_hz()), pref)
        })
        .collect();
    let current = current_mode_index(prefs, mon);
    let title = format!("{} — choose a mode", mon.label);
    match select_list(&title, &items, current, true) {
        Nav::Selected(i) => {
            let m = &mon.modes[i];
            pref::upsert_output(
                &mut prefs.outputs,
                &mon.identity_key,
                ModeRequest::Advertised { width: m.width, height: m.height, refresh_mhz: m.refresh_mhz },
            );
            match pref::save(prefs) {
                Ok(()) => println!(
                    "\nSet {} to {}x{} @ {}Hz ✓",
                    mon.label, m.width, m.height, m.refresh_hz()
                ),
                Err(e) => eprintln!("\nfailed to save preferences: {e}"),
            }
        }
        Nav::Back => {}
    }
}

/// Index in `monitors` of the current default output (the monitor matching the FIRST
/// profile in the array), or `None` when no profile is saved yet.
fn default_index(prefs: &Preference, monitors: &[ProbedMonitor]) -> Option<usize> {
    let key = prefs.outputs.first()?.identity.as_deref()?;
    monitors.iter().position(|m| m.identity_key == key)
}

/// Index into `mon.modes` of the currently-saved Advertised mode for this monitor.
fn current_mode_index(prefs: &Preference, mon: &ProbedMonitor) -> Option<usize> {
    let profile = prefs.outputs.iter().find(|p| p.identity.as_deref() == Some(&mon.identity_key))?;
    let ModeRequest::Advertised { width, height, refresh_mhz } = profile.mode.as_ref()? else {
        return None;
    };
    mon.modes
        .iter()
        .position(|m| m.width == *width && m.height == *height && m.refresh_mhz == *refresh_mhz)
}

/// A short "current: WxH@RHz" hint for the monitor list, or "" when nothing is saved.
fn saved_hint(prefs: &Preference, key: &str) -> String {
    let Some(profile) = prefs.outputs.iter().find(|p| p.identity.as_deref() == Some(key)) else {
        return String::new();
    };
    match &profile.mode {
        Some(ModeRequest::Advertised { width, height, refresh_mhz }) => {
            format!("current: {width}x{height} @ {}Hz", refresh_mhz / 1000)
        }
        Some(_) => "current: custom".to_string(),
        None => String::new(),
    }
}

/// Fallback when DRM can't be probed: ask for an identity key and a width/height/
/// refresh, and save an Advertised request. The identity key must match the
/// compositor's "make model serial" — we tell the user as much.
fn manual_entry(prefs: &mut Preference) {
    println!(
        "\nEnter the monitor identity exactly as the compositor reports it\n\
         (\"make model serial\", e.g. \"DEL U2720Q 12345\"); leave blank to cancel."
    );
    let key = ask("identity", "Monitor identity key.", "");
    if key.trim().is_empty() {
        return;
    }
    let width = ask("width", "Mode width in pixels.", "1920").parse::<u16>().unwrap_or(1920);
    let height = ask("height", "Mode height in pixels.", "1080").parse::<u16>().unwrap_or(1080);
    let refresh_hz = ask("refresh", "Refresh rate in Hz.", "60").parse::<u32>().unwrap_or(60);
    pref::upsert_output(
        &mut prefs.outputs,
        key.trim(),
        ModeRequest::Advertised { width, height, refresh_mhz: refresh_hz * 1000 },
    );
    match pref::save(prefs) {
        Ok(()) => println!("\nSet {} to {width}x{height} @ {refresh_hz}Hz ✓", key.trim()),
        Err(e) => eprintln!("\nfailed to save preferences: {e}"),
    }
}
