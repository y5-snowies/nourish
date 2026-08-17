//! Which bus are we on, what can it start, and does the environment agree.

use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use zbus::blocking::Connection;

/// Well-known names currently owned, plus those the bus knows how to start.
///
/// Activatable is the more useful of the two: a `.service` file plus an
/// installed binary is what "the portal works here" actually means. Nothing has
/// to be running yet.
pub struct Names {
    pub running: HashSet<String>,
    pub activatable: HashSet<String>,
}

impl Names {
    pub fn has(&self, name: &str) -> bool {
        self.running.contains(name) || self.activatable.contains(name)
    }

    /// How the name is present, for the detail column.
    pub fn how(&self, name: &str) -> &'static str {
        match (self.running.contains(name), self.activatable.contains(name)) {
            (true, true) => "running, activatable",
            (true, false) => "running",
            (false, true) => "activatable",
            (false, false) => "absent",
        }
    }

    /// Every present name under `prefix`, sorted.
    pub fn with_prefix(&self, prefix: &str) -> Vec<String> {
        let mut found: Vec<String> = self
            .running
            .union(&self.activatable)
            .filter(|n| n.starts_with(prefix))
            .cloned()
            .collect();
        found.sort();
        found
    }
}

pub fn names(connection: &Connection) -> zbus::Result<Names> {
    let proxy = zbus::blocking::fdo::DBusProxy::new(connection)?;
    let running = proxy.list_names()?.into_iter().map(|n| n.to_string()).collect();
    let activatable = proxy
        .list_activatable_names()?
        .into_iter()
        .map(|n| n.to_string())
        .collect();
    Ok(Names { running, activatable })
}

/// The address libdbus would resolve, and how it was arrived at.
///
/// The fallback branch is the one that matters for isolation: with no
/// `DBUS_SESSION_BUS_ADDRESS`, a client silently uses `$XDG_RUNTIME_DIR/bus`.
/// That is why a private session wants a private runtime dir and not merely a
/// private env var — lose the variable and you land back on the host's bus
/// without any error to notice.
pub fn address() -> (String, &'static str) {
    if let Ok(addr) = std::env::var("DBUS_SESSION_BUS_ADDRESS") {
        return (addr, "DBUS_SESSION_BUS_ADDRESS");
    }
    match std::env::var("XDG_RUNTIME_DIR") {
        Ok(dir) => (format!("unix:path={dir}/bus"), "XDG_RUNTIME_DIR fallback"),
        Err(_) => (String::from("<unresolvable>"), "no env"),
    }
}

/// `/run/user/<uid>` — the runtime dir a login session gets by default.
pub fn default_runtime_dir() -> Option<PathBuf> {
    let uid = std::fs::metadata("/proc/self").ok()?.uid();
    Some(PathBuf::from(format!("/run/user/{uid}")))
}

/// Whether `address` points at the default per-user bus socket.
///
/// A private session must answer `false` here. It is the single check that
/// distinguishes "I isolated the session" from "I thought I did".
pub fn is_default_bus(address: &str) -> Option<bool> {
    let path = address.strip_prefix("unix:path=")?.split(',').next()?;
    let default = default_runtime_dir()?.join("bus");
    Some(Path::new(path) == default)
}
