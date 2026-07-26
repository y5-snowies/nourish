//! Deciding, from process metadata alone, that a client wants to tear.
//!
//! Games mostly do not speak `wp_tearing_control_v1`, so the protocol tag covers
//! almost none of the windows it was meant for. These heuristics fill that gap
//! from what introspection already captured.
//!
//! Two sources, both resolved against the window's process AND its ancestors —
//! `Y5_TEARING=1` (the user saying so, e.g. as a Steam launch command; always
//! honoured), and Steam attribution (`steam_app_*`, the `SteamAppId`/compat-tool
//! environment, or an executable inside a library).
//!
//! Mind the asymmetry the X11 proxy creates: xwayland-satellite is a single
//! process for every X11 title, so such a window's `/proc` data is the
//! SATELLITE's and only its `app_id` says anything — `Y5_TEARING` on that game's
//! launch command is invisible here. Native (`PROTON_ENABLE_WAYLAND`) clients are
//! their own process and match on every rule.

use compositor_introspection_extraction_window_meta_types::types::{Meta, MetaNode};
use std::path::Path;

pub const TEARING_ENV: &str = "Y5_TEARING";

/// Path components that exist only inside a Steam library — `steamapps` for
/// installed titles, `compatdata` for a Proton prefix. Matched anywhere in the
/// path so libraries on other drives are covered, not just the default root.
const LIBRARY: [&str; 2] = ["steamapps", "compatdata"];

/// Subdirectories of the Steam install root holding Steam's OWN binaries. The
/// client lives under the same root as the games it installs, so the root alone
/// cannot separate them.
const CLIENT: [&str; 5] = ["ubuntu12_32", "ubuntu12_64", "linux32", "linux64", "bin"];

fn env<'a>(m: &'a Meta, key: &str) -> Option<&'a str> {
    m.selected_env.as_ref()?.get(key).map(String::as_str)
}

fn explicit(m: &Meta) -> bool {
    env(m, TEARING_ENV).is_some_and(|v| matches!(v, "1" | "true" | "on"))
}

/// A Steam install root, lowercased: the default, the legacy path, and Flatpak.
fn root(part: &str) -> bool {
    matches!(part, "steam" | ".steam" | "com.valvesoftware.steam")
}

fn library(exe: &Path) -> bool {
    let parts: Vec<String> = exe
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    if parts.iter().any(|p| LIBRARY.contains(&p.as_str())) {
        return true;
    }
    // Under a Steam root but outside a library — a non-Steam shortcut, or the
    // client itself. `< len - 1` rejects the launcher binary (`/usr/bin/steam`),
    // whose own name is the matching component.
    let Some(at) = parts.iter().position(|p| root(p)).filter(|at| at + 1 < parts.len()) else {
        return false;
    };
    !parts[at + 1..].iter().any(|p| CLIENT.contains(&p.as_str()))
}

fn steam(m: &Meta) -> bool {
    m.app_id.as_deref().is_some_and(|id| id.starts_with("steam_app_"))
        || env(m, "SteamAppId").is_some_and(|v| v != "0")
        || env(m, "SteamGameId").is_some_and(|v| v != "0")
        || env(m, "STEAM_COMPAT_DATA_PATH").is_some()
        || m.exe.as_deref().is_some_and(library)
}

/// Steam's own windows. They arrive through the same proxy, from the same root,
/// with the same environment as the games, so they have to be named out.
fn client(m: &Meta) -> bool {
    m.app_id.as_deref().is_some_and(|id| {
        let id = id.to_lowercase();
        id == "steam" || id.starts_with("steamwebhelper")
    })
}

/// Does this window want to tear? `steam_enabled` gates only the Steam
/// heuristic — `Y5_TEARING` is honoured even on a window Steam attribution
/// excludes, since there it is the user overriding, not a guess.
pub fn is_target(node: &MetaNode, steam_enabled: bool) -> bool {
    let by_steam = steam_enabled && !client(&node.meta);
    let mut at = Some(node);
    while let Some(n) = at {
        if explicit(&n.meta) || (by_steam && steam(&n.meta)) {
            return true;
        }
        at = n.parent.as_deref();
    }
    false
}
