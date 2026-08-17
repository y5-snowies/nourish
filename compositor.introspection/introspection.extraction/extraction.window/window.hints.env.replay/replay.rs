//! Which captured environment variables must NOT be re-applied at launch.
//!
//! `ENV_ALLOWLIST` (capture) and this list (replay) are two different
//! policies. Capture decides what we are willing to read at all; replay
//! decides what may be pushed back into a relaunched process. Replay is the
//! sharper one, because a replayed variable OVERRIDES the executor's
//! `base_env`: the launch env is assembled `base_env ++ request.env` and
//! applied last-wins, so anything in the plan's `EnvOverlay` beats the live
//! session value.
//!
//! That is how a placeholder persisted under `WAYLAND_DISPLAY=wayland-1` came
//! back after a reboot, pinned the app to a socket that no longer existed —
//! or, worse, to whatever else had since claimed that name.
//!
//! A DENYLIST, not an allowlist. We cannot know which variables matter to a
//! user's session: a toolkit override, a proxy setting, a locale, a licence
//! path, some in-house variable an internal tool needs. Enumerating what is
//! allowed would silently drop all of them, and the failure would be invisible
//! — the app just behaves differently and nothing says why. So the default is
//! to replay what we captured, and only the variables we can NAME A REASON to
//! withhold are withheld.
//!
//! This filter applies only to the INFERRED overlay. Env pairs the user typed
//! into the placeholder's Env overlay field are an explicit override and are
//! always replayed, whatever their name.

/// Captured variables that must never be re-applied to a relaunched process.
///
/// Every entry needs a reason that survives scrutiny, in one of three kinds:
///
/// 1. **Session-owned** — the compositor is the authority and publishes the
///    live value through `base_env`; a captured copy can only be staler.
///    `DISPLAY` is here for a second reason: `base_env` sets it EMPTY on
///    purpose to keep apps off the X11 fallback, and a captured `DISPLAY=:0`
///    silently undid that.
/// 2. **One-shot** — minted per launch, so a stale one correlates to nothing.
/// 3. **Provenance** — describes where the ORIGINAL process came from.
///    Replaying `container=podman` onto a host launch is a plain lie, and a
///    sandbox sets its own on the way in.
pub const ENV_NO_REPLAY: &[&str] = &[
    // 1. Session-owned.
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "XDG_SESSION_TYPE",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_DESKTOP",
    "XDG_SESSION_ID",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "DESKTOP_SESSION",
    // 2. One-shot.
    "XDG_ACTIVATION_TOKEN",
    "DESKTOP_STARTUP_ID",
    "GIO_LAUNCHED_DESKTOP_FILE_PID",
    "BAMF_DESKTOP_FILE_HINT",
    // 3. Provenance of the original process.
    "container",
    "FLATPAK_ID",
    "SNAP",
    "SNAP_INSTANCE_NAME",
    "APPIMAGE",
    "SSH_CONNECTION",
    "SSH_CLIENT",
    "SSH_TTY",
];

/// Whether a captured variable may be re-applied at launch.
pub fn is_replayable(key: &str) -> bool {
    !ENV_NO_REPLAY.contains(&key)
}
