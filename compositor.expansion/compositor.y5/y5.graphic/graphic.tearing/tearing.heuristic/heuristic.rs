//! Deciding, from process metadata alone, that a client wants to tear.
//!
//! Games mostly do not speak `wp_tearing_control_v1`, so the protocol tag covers
//! almost none of the windows it was meant for. Two sources fill the gap, both
//! resolved against the window's process AND its ancestors: `Y5_TEARING=1` (the
//! user saying so, always honoured) and Steam attribution (`steam_app_*`, the
//! `SteamAppId`/compat-tool environment, or an executable inside a library).
//!
//! X11 titles resolve like any other, which is a property of running XWayland
//! natively rather than behind a proxy: an X11 window's pid is its own
//! `_NET_WM_PID` (see `window.ident`), not the surface credentials that would name
//! the single X server for every X11 title on screen.

use compositor_introspection_extraction_window_meta_types::types::{Meta, MetaNode};
use std::path::Path;

pub const TEARING_ENV: &str = "Y5_TEARING";

/// Exact path components that exist only inside a Steam library. Matched anywhere
/// in the path, so libraries on other drives are covered too.
const LIBRARY: [&str; 1] = ["steamapps"];

/// Prefix matches for the same: the compatibility tools live in several
/// differently-named siblings — `compatdata` for a per-title Proton prefix,
/// `compatibilitytools.d` for user-installed Proton/Wine builds — and a title
/// running under either is executing out of that directory.
const LIBRARY_PREFIX: [&str; 1] = ["compat"];

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

fn in_library(part: &str) -> bool {
    LIBRARY.contains(&part) || LIBRARY_PREFIX.iter().any(|pre| part.starts_with(pre))
}

fn library(exe: &Path) -> bool {
    let parts: Vec<String> = exe
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    if parts.iter().any(|p| in_library(p)) {
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

/// Steam's own windows. They come from the same root, with the same environment
/// as the games, so they have to be named out.
fn client(m: &Meta) -> bool {
    m.app_id.as_deref().is_some_and(|id| {
        let id = id.to_lowercase();
        id == "steam" || id.starts_with("steamwebhelper")
    })
}

/// Why a window is a target — and how firmly. The two rank on opposite sides of the
/// client's own hint; `pacer::Verdict::is_target` holds the ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `Y5_TEARING=1` on the process or an ancestor — the user, about this exact process.
    Forced,
    /// Steam attribution — a guess the client may contradict.
    Steam,
}

/// Does this window want to tear, and on whose authority? `steam_enabled` gates only the
/// Steam guess — `Y5_TEARING` is honoured even on a window Steam attribution excludes,
/// since there it is the user overriding. `Forced` wins anywhere in the chain, so the
/// walk cannot return on the first match: a Steam-attributed parent must not mask a
/// `Y5_TEARING` set further up.
pub fn classify(node: &MetaNode, steam_enabled: bool) -> Option<Source> {
    let by_steam = steam_enabled && !client(&node.meta);
    let mut by_guess = false;
    let mut at = Some(node);
    while let Some(n) = at {
        if explicit(&n.meta) {
            return Some(Source::Forced);
        }
        by_guess |= by_steam && steam(&n.meta);
        at = n.parent.as_deref();
    }
    by_guess.then_some(Source::Steam)
}
