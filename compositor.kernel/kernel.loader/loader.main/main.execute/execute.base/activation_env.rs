//! Push session environment variables to the D-Bus activation env
//! and the systemd user manager.
//!
//! This is what tells `systemd-run --user` (and D-Bus-activated user
//! services) which Wayland socket and session type to use. Without
//! this step, launches inherit whatever the user manager had at
//! login — under GDM that's GNOME's `WAYLAND_DISPLAY=wayland-0`, so
//! apps land on GNOME instead of y5.
//!
//! GNOME, KDE, sway, etc. all do this at session start. We do the
//! same.

use std::io;

// Every spawn goes through the hygiene wrapper — see `process.child`.
use compositor_support_library_process_child_hygiene::hygiene::command;
use compositor_support_library_process_child_spawn::spawn as child_spawn;

/// Update the session and user-manager activation environments with
/// `KEY=VALUE` pairs, so subsequent `systemd-run --user` and
/// D-Bus-activated launches inherit them.
///
/// Calls `dbus-update-activation-environment --systemd` under the
/// hood. The `--systemd` flag means both the session D-Bus and the
/// systemd user manager get updated. Without `--systemd` only D-Bus
/// would be updated, and `systemd-run --user` would still see the
/// old (GDM-time) env.
///
/// An empty value (`KEY=`) clears the variable for spawned apps,
/// which is what we want for `DISPLAY` so apps don't fall back to
/// XWayland under another compositor.
pub fn push_session_env(pairs: &[(&str, &str)]) -> io::Result<()> {
    if pairs.is_empty() {
        return Ok(());
    }

    let mut cmd = command("dbus-update-activation-environment");
    cmd.arg("--systemd");
    for (k, v) in pairs {
        cmd.arg(format!("{k}={v}"));
    }

    let status = child_spawn::status(&mut cmd)?;
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("dbus-update-activation-environment exited with {status}"),
        ));
    }
    Ok(())
}

/// Convenience: push the standard Wayland-session env. Call this
/// right after creating the Wayland socket, alongside your existing
/// `std::env::set_var("WAYLAND_DISPLAY", …)`.
///
/// - `WAYLAND_DISPLAY` → the socket name your compositor created.
/// - `DISPLAY=` (empty) → forces apps off X11 fallback. Set this to
///   `":1"` or whatever later if you run XWayland under y5.
/// - `XDG_SESSION_TYPE=wayland` → declares we're a Wayland session;
///   some apps check this directly.
/// - `XDG_CURRENT_DESKTOP=y5` → identifies the desktop for apps that
///   use it to pick portal backends, theming, etc.
///
/// Toolkit-specific hints (`MOZ_ENABLE_WAYLAND`, `GDK_BACKEND`,
/// `QT_QPA_PLATFORM`) are intentionally NOT set here — they can
/// break apps that don't want them (Electron apps with old Chromium,
/// some legacy GTK apps), and individual apps that need them can
/// set them in their own desktop files via `Exec=env GDK_BACKEND=…`.
pub fn push_wayland_session_env(wayland_socket: &str) -> io::Result<()> {
    push_session_env(&[
        ("WAYLAND_DISPLAY", wayland_socket),
        ("DISPLAY", ""),
        ("XDG_SESSION_TYPE", "wayland"),
        // ("XDG_CURRENT_DESKTOP", "y5"),
    ])
}