//! The portal frontend, its interfaces, and whichever backend is meant to serve
//! them.
//!
//! Reading a `version` property here is not a passive check — it D-Bus-activates
//! `xdg-desktop-portal` if it is not already running, which is precisely the
//! evidence wanted: a version that comes back proves the service file, the
//! binary and the bus policy all line up.

use std::path::PathBuf;

use zbus::blocking::Connection;
use zbus::names::InterfaceName;

pub const DESTINATION: &str = "org.freedesktop.portal.Desktop";
pub const OBJECT: &str = "/org/freedesktop/portal/desktop";

/// `(interface suffix, required)`. Required means a user will visibly try to do
/// this and be met with nothing at all when it is missing.
pub const INTERFACES: &[(&str, bool)] = &[
    ("FileChooser", true),
    ("OpenURI", true),
    ("Settings", true),
    ("Screenshot", false),
    ("ScreenCast", false),
    ("Secret", false),
    ("Inhibit", false),
    ("Notification", false),
];

/// The `version` property of one portal interface.
pub fn version(connection: &Connection, interface: &str) -> zbus::Result<u32> {
    let proxy = zbus::blocking::fdo::PropertiesProxy::builder(connection)
        .destination(DESTINATION)?
        .path(OBJECT)?
        .build()?;
    let name = InterfaceName::try_from(format!("org.freedesktop.portal.{interface}"))
        .map_err(|e| zbus::Error::Failure(e.to_string()))?;
    let value = proxy.get(name, "version")?;
    u32::try_from(&value).map_err(|e| zbus::Error::Failure(e.to_string()))
}

/// ScreenCast's `AvailableSourceTypes` bitmask, rendered.
pub fn source_types(connection: &Connection) -> zbus::Result<String> {
    let proxy = zbus::blocking::fdo::PropertiesProxy::builder(connection)
        .destination(DESTINATION)?
        .path(OBJECT)?
        .build()?;
    let name = InterfaceName::try_from("org.freedesktop.portal.ScreenCast")
        .map_err(|e| zbus::Error::Failure(e.to_string()))?;
    let bits = u32::try_from(&proxy.get(name, "AvailableSourceTypes")?)
        .map_err(|e| zbus::Error::Failure(e.to_string()))?;
    let mut kinds = Vec::new();
    if bits & 1 != 0 {
        kinds.push("monitor");
    }
    if bits & 2 != 0 {
        kinds.push("window");
    }
    if bits & 4 != 0 {
        kinds.push("virtual");
    }
    Ok(match kinds.is_empty() {
        true => format!("none (bits={bits})"),
        false => kinds.join(", "),
    })
}

/// Config files xdg-desktop-portal would consult, most specific first, and the
/// `default=` line each one sets.
///
/// This reports what is on disk rather than re-deriving the backend the portal
/// will pick: the real algorithm also considers each backend's `UseIn`/portal
/// files, and a probe that guesses wrong is worse than one that shows its
/// working.
pub fn configs(desktop: &str) -> Vec<(PathBuf, Option<String>)> {
    let home = std::env::var("HOME").unwrap_or_default();
    let config_home = std::env::var("XDG_CONFIG_HOME").unwrap_or(format!("{home}/.config"));
    let mut names = Vec::new();
    // XDG_CURRENT_DESKTOP is colon-separated and tried left to right.
    for entry in desktop.split(':').filter(|d| !d.is_empty()) {
        names.push(format!("{}-portals.conf", entry.to_lowercase()));
    }
    names.push(String::from("portals.conf"));

    let mut found = Vec::new();
    for dir in [
        format!("{config_home}/xdg-desktop-portal"),
        String::from("/etc/xdg-desktop-portal"),
        String::from("/usr/share/xdg-desktop-portal"),
    ] {
        for name in &names {
            let path = PathBuf::from(&dir).join(name);
            if path.exists() {
                let default = std::fs::read_to_string(&path).ok().and_then(|text| {
                    text.lines()
                        .map(str::trim)
                        .find(|line| line.starts_with("default"))
                        .map(str::to_string)
                });
                found.push((path, default));
            }
        }
    }
    found
}
