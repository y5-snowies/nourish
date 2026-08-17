//! Why a keyring password prompt appears in one session and not another.
//!
//! `org.freedesktop.secrets` owning its name proves nothing about whether the
//! next app to want a secret will interrupt the user. The login collection is
//! normally unlocked at login by `pam_gnome_keyring.so`, which captures the
//! password from the PAM stack that started the session. A session begun
//! outside that stack — a nested compositor, a systemd user unit, a container —
//! reaches the desktop with the collection still locked, and the first secret
//! request is what surfaces it, arbitrarily far from the actual cause.
//!
//! So there are two questions, and the lock state is the one that matters:
//! is the collection locked, and is anything in this system's PAM configuration
//! positioned to have unlocked it.

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;

pub const NAME: &str = "org.freedesktop.secrets";
const PATH: &str = "/org/freedesktop/secrets";
const SERVICE_IFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_IFACE: &str = "org.freedesktop.Secret.Collection";

pub struct Collection {
    pub path: String,
    pub label: String,
    pub locked: bool,
    /// This is the collection the `default` alias resolves to — the one an app
    /// that just asks for "a secret" will touch.
    pub is_default: bool,
}

fn service(connection: &Connection) -> zbus::Result<Proxy<'_>> {
    Proxy::new(connection, NAME, PATH, SERVICE_IFACE)
}

/// Every collection the service exposes, with the default one flagged.
pub fn collections(connection: &Connection) -> zbus::Result<Vec<Collection>> {
    let service = service(connection)?;
    let paths: Vec<OwnedObjectPath> = service.get_property("Collections")?;

    // `ReadAlias` answers with "/" when the alias is unset, which is itself
    // worth knowing: no default collection means every app is prompted to make
    // one rather than to unlock one.
    let default: Option<String> = service
        .call::<_, _, OwnedObjectPath>("ReadAlias", &("default",))
        .ok()
        .map(|p| p.as_str().to_string())
        .filter(|p| p != "/");

    let mut found = Vec::new();
    for path in paths {
        let path = path.as_str().to_string();
        let proxy = Proxy::new(connection, NAME, path.clone(), COLLECTION_IFACE)?;
        // Unreadable `Locked` is treated as locked: the failure this reports on
        // is a prompt the user did not expect, so guessing "unlocked" would hide
        // exactly the case worth surfacing.
        let collection = Collection {
            label: proxy
                .get_property::<String>("Label")
                .ok()
                .filter(|l| !l.trim().is_empty())
                .unwrap_or_else(|| String::from("<unnamed>")),
            locked: proxy.get_property("Locked").unwrap_or(true),
            is_default: default.as_deref() == Some(path.as_str()),
            path,
        };
        drop(proxy);
        found.push(collection);
    }
    Ok(found)
}

/// Where `pam_gnome_keyring.so` lives, if it is installed at all.
pub fn keyring_module() -> Option<String> {
    [
        "/usr/lib64/security/pam_gnome_keyring.so",
        "/usr/lib/x86_64-linux-gnu/security/pam_gnome_keyring.so",
        "/usr/lib/security/pam_gnome_keyring.so",
        "/lib/security/pam_gnome_keyring.so",
    ]
    .into_iter()
    .find(|p| std::path::Path::new(p).exists())
    .map(str::to_string)
}

/// PAM services wired for keyring auto-unlock, split by which half they do.
///
/// `auth` is where the password is captured and the collection unlocked;
/// `session` starts the daemon. A service with only `session` still leaves the
/// collection locked — the distinction is the difference between "prompted
/// once at login" and "prompted later, seemingly at random".
pub fn keyring_pam_services() -> Vec<(String, bool, bool)> {
    let Ok(entries) = std::fs::read_dir("/etc/pam.d") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let lines: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with('#') && l.contains("pam_gnome_keyring.so"))
            .collect();
        if lines.is_empty() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let auth = lines.iter().any(|l| l.starts_with("auth"));
        let session = lines.iter().any(|l| l.starts_with("session"));
        found.push((name, auth, session));
    }
    found.sort();
    found
}
