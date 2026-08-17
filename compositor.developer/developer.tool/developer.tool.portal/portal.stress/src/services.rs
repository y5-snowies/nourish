//! "Owns a bus name" is not "works".
//!
//! A `.service` file makes a name activatable whether or not the binary behind
//! it can start; a name can also be held by a process that is wedged. Every
//! service here is therefore round-tripped rather than merely looked up.

use zbus::blocking::{Connection, Proxy};

/// `(well-known name, object path, required, what breaks without it)`.
///
/// The path is the service's own root — `org.freedesktop.DBus.Peer` is exported
/// on it by every binding, so one call form covers all of them.
pub const SERVICES: &[(&str, &str, bool, &str)] = &[
    ("ca.desrt.dconf", "/ca/desrt/dconf/Writer/user", true, "GSettings writes fall back to memory"),
    ("org.gtk.vfs.Daemon", "/org/gtk/vfs/Daemon", false, "Files loses trash, recent, mounts"),
    ("org.gtk.vfs.UDisks2VolumeMonitor", "/org/gtk/Private/RemoteVolumeMonitor", false, "no removable volumes in Files"),
    ("org.freedesktop.Notifications", "/org/freedesktop/Notifications", false, "no desktop notifications"),
    ("org.a11y.Bus", "/org/a11y/bus", false, "accessibility bus absent"),
    ("org.freedesktop.systemd1", "/org/freedesktop/systemd1", false, "no transient scopes for launched apps"),
    ("org.freedesktop.FileManager1", "/org/freedesktop/FileManager1", false, "\"show in file manager\" does nothing"),
];

/// Round-trip a service. Activates it if it is not yet running, which is the
/// point — starting is the half of "installed" that a name listing cannot show.
pub fn ping(connection: &Connection, name: &str, path: &str) -> zbus::Result<()> {
    let proxy = Proxy::new(connection, name, path, "org.freedesktop.DBus.Peer")?;
    proxy.call::<_, _, ()>("Ping", &())
}

/// The command behind a bus name, for telling implementations apart —
/// gnome-keyring-daemon, kwalletd6 and keepassxc all answer to
/// `org.freedesktop.secrets` and behave differently.
pub fn owner_command(connection: &Connection, name: &str) -> Option<String> {
    let dbus = zbus::blocking::fdo::DBusProxy::new(connection).ok()?;
    let pid = dbus.get_connection_unix_process_id(name.try_into().ok()?).ok()?;
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(format!("{} (pid {pid})", comm.trim()))
}
