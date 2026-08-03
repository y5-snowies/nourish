// Every spawn goes through the hygiene wrapper — see `process.child`.
use compositor_support_library_process_child_hygiene::hygiene::command;

/// Name advertised to portals — must match `DesktopNames=` in the .desktop
/// and `XDG_CURRENT_DESKTOP` in the wrapper.
// const DESKTOP_NAME: &str = "Y5Compositor";

/// Propagate the session environment to systemd --user and the D-Bus
/// activation environment. Call ONCE, after the Wayland socket is listening
/// and all backends are initialized.
///
/// `wayland_socket` is the name Smithay gave you, e.g. from
/// `ListeningSocketSource::socket_name()` — typically "wayland-1".
///
/// Only call this when running as the actual session compositor. Do NOT call
/// it when running nested/embedded for development: it mutates the user-wide
/// systemd and D-Bus environment and would clobber the host session's vars.
pub fn announce_session(wayland_socket: &str, desktop_name: &str) {
    // WAYLAND_DISPLAY is passed as NAME=VALUE so we don't depend on it being
    // present in our own process env. The desktop name and session type are
    // passed explicitly too, so this works regardless of what the wrapper set.
    let result = command("dbus-update-activation-environment")
        .arg("--systemd")
        .arg(format!("WAYLAND_DISPLAY={wayland_socket}"))
        .arg(format!("XDG_CURRENT_DESKTOP={desktop_name}"))
        .arg("XDG_SESSION_TYPE=wayland")
        .status();

    match result {
        Ok(s) if s.success() => {
            info!("session environment propagated to systemd and D-Bus");
        }
        Ok(s) => {
            warn!("dbus-update-activation-environment exited with status {s}");
        }
        Err(e) => {
            warn!(
                "could not run dbus-update-activation-environment: {e} \
                        (is it on PATH? it ships with dbus)"
            );
        }
    }
}
