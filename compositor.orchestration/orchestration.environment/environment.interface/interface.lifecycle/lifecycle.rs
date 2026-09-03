//! Publishing the session environment. [`announce_session`] is the once-at-startup set,
//! [`push_session_env`] the incremental one for values only known later (`DISPLAY`, once
//! y5's own Xwayland reports its number); both go through [`publish`].
//!
//! TWO publishers, run in sequence, reaching different consumers.
//! `systemctl --user set-environment` sets the systemd user manager's environment — what
//! user units and `systemd-run --user` inherit, and on a modern desktop what a
//! D-Bus-activated service inherits too, since activation is delegated to systemd
//! wherever a `.service` file carries `SystemdService=` (xdg-desktop-portal does).
//! `dbus-update-activation-environment --systemd` sets the session bus's own activation
//! environment as well, for services activated by dbus-daemon directly.
//!
//! systemd goes FIRST: it is the harder dependency (y5 already requires user units,
//! logind and `sd_booted`, while the dbus TOOLS are a separate package from the daemon),
//! it is the half that carries the portals, and it is what `systemctl --user
//! show-environment` reports — so if the second call is missing entirely, the half that
//! matters has already landed.

use std::io;

// Every spawn goes through the hygiene wrapper — see `process.child`.
use compositor_support_library_process_child_hygiene::hygiene::command;
use compositor_support_library_process_child_spawn::spawn as child_spawn;

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
    match publish(&[
        ("WAYLAND_DISPLAY", wayland_socket),
        ("XDG_CURRENT_DESKTOP", desktop_name),
        ("XDG_SESSION_TYPE", "wayland"),
    ]) {
        Ok(()) => info!("session environment propagated to systemd and D-Bus"),
        Err(err) => warn!("could not propagate the session environment: {err}"),
    }
}

/// systemd as the init system? `/run/systemd/system` exists iff systemd is PID 1 —
/// libsystemd's `sd_booted()`. Without it `systemctl --user` only answers "has not been
/// booted with systemd as init system", so the call is skipped rather than spawned to
/// fail. (`executor.install` carries the same one-liner; sharing a `Path::exists` across
/// crates would cost more than it saves.)
fn systemd_booted() -> bool {
    std::path::Path::new("/run/systemd/system").exists()
}

/// One publisher. A non-zero exit is an error like a spawn failure — both mean the
/// environment did not land.
fn run(program: &str, leading: &[&str], pairs: &[(&str, &str)]) -> Result<(), String> {
    let mut cmd = command(program);
    cmd.args(leading);
    for (k, v) in pairs {
        cmd.arg(format!("{k}={v}"));
    }
    match child_spawn::status(&mut cmd) {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("{program} exited with {status}")),
        Err(err) => Err(format!("could not run {program}: {err}")),
    }
}

/// Publish `pairs` to both activation environments — see the module docs for which
/// consumer each reaches.
///
/// `Ok` if EITHER landed, because they are not equivalent halves of one write: systemd
/// alone still serves the portals and `systemd-run --user`, and D-Bus alone still serves
/// directly-activated services. A partial success says which half is missing rather than
/// discarding the half that worked.
fn publish(pairs: &[(&str, &str)]) -> io::Result<()> {
    if pairs.is_empty() {
        return Ok(());
    }
    let systemd = systemd_booted()
        .then(|| run("systemctl", &["--user", "set-environment"], pairs))
        .unwrap_or_else(|| Err("systemd is not PID 1".to_string()));
    let dbus = run("dbus-update-activation-environment", &["--systemd"], pairs);
    match (&systemd, &dbus) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(err)) => {
            warn!("D-Bus activation environment not updated ({err}); systemd user manager is set");
            Ok(())
        }
        (Err(err), Ok(())) => {
            warn!("systemd user manager not updated ({err}); D-Bus activation env is set");
            Ok(())
        }
        (Err(a), Err(b)) => Err(io::Error::other(format!("{a}; {b}"))),
    }
}

/// Update the activation environments with `KEY=VALUE` pairs, so later
/// `systemd-run --user` and D-Bus-activated launches inherit them.
///
/// `--systemd` updates the systemd user manager as well as the session bus; without
/// it `systemd-run --user` would keep the login-time (under GDM: GNOME's) values. An
/// empty value clears the variable, which is what `DISPLAY` wants before y5's own
/// Xwayland is up — otherwise apps fall back to X under ANOTHER compositor.
pub fn push_session_env(pairs: &[(&str, &str)]) -> io::Result<()> {
    publish(pairs)
}

/// [`push_session_env`], but only while this session is the ACTIVE one.
///
/// The environment written here is per-USER, not per-session: every y5 the same user
/// runs on every VT shares it. A push from a session nobody is looking at overwrites
/// the environment of the session they ARE looking at — and the worst case is not a
/// stale value but a destructive one, a background session's Xwayland dying and
/// clearing `DISPLAY` out from under a foreground session whose X server is healthy.
///
/// Skipping loses nothing: the pushing session's own process state is updated
/// regardless (that is what its launches read, via `executor.install::base_env`), and
/// the session lifecycle re-publishes the whole set the moment it becomes active
/// again. `active` is passed rather than read so this crate stays what it is — a
/// wrapper around one subprocess — and so the condition is visible at the call site.
pub fn push_session_env_if_active(active: bool, pairs: &[(&str, &str)]) {
    if !active {
        info!("session not active; deferring env publish to the next activation: {pairs:?}");
        return;
    }
    if let Err(err) = push_session_env(pairs) {
        warn!("could not publish the session environment: {err:?}");
    }
}

/// Unpublish what this session published — the exit counterpart of [`announce_session`],
/// and only for the values this session still OWNS.
///
/// Leaving them behind is not cosmetic. The environment is per-USER and outlives the
/// process, so the next login inherits a `WAYLAND_DISPLAY` naming a socket nobody is
/// listening on — and `main()` documents where that lands: Mesa's device-select layer
/// does a blocking roundtrip to `$WAYLAND_DISPLAY` inside `vkEnumeratePhysicalDevices`,
/// so the next compositor hangs during GPU init, before its event loop starts. First boot
/// works and every login after a session hangs.
///
/// OWNERSHIP IS DECIDED BY COMPARISON, not by whether this session is active. `active`
/// answers "may I publish", which is a different question with a different failure: by
/// the time a session is torn down it may already have been deactivated (logind can
/// deactivate before it signals the process), so an active-gated retraction skips exactly
/// the logout it exists for. Reading the value back and clearing only an exact match is
/// precise in both directions — a successor that has already announced its own socket
/// keeps it, and a predecessor still owning the value clears it however long it took to
/// exit.
///
/// This is NOT the read-back that `session.lifecycle` rejects. That one would take the
/// shared value as INPUT and republish it, which means following whatever last trampled
/// it. Here the value is only ever compared against what we ourselves wrote, and the
/// answer decides whether to clear.
///
/// `active` remains the fallback for the case where the shared environment cannot be
/// read at all — no systemd, or `show-environment` failing — where the old rule is still
/// the best available guess.
///
/// Clearing is an empty value, since neither publisher has an unset verb. Pairs with an
/// empty value are skipped: there is nothing to own and nothing to retract.
pub fn retract_session_env(active: bool, owned: &[(&str, &str)]) {
    let published: Vec<(&str, &str)> = owned.iter().copied().filter(|(_, v)| !v.is_empty()).collect();
    if published.is_empty() {
        return;
    }
    let clear: Vec<(&str, &str)> = match show_environment() {
        Some(shared) => published
            .iter()
            .filter(|(key, value)| still_ours(&shared, key, value))
            .map(|(key, _)| (*key, ""))
            .collect(),
        None => {
            if !active {
                info!("cannot read the shared environment and this session is not active; leaving {published:?}");
                return;
            }
            published.iter().map(|(key, _)| (*key, "")).collect()
        }
    };
    if clear.is_empty() {
        info!("shared environment no longer names this session; leaving it for its owner");
        return;
    }
    match push_session_env(&clear) {
        Ok(()) => info!("session environment retracted: {clear:?}"),
        Err(err) => warn!("could not retract the session environment: {err:?}"),
    }
}

/// The shared environment as the systemd user manager reports it, or `None` when it
/// cannot be read — no systemd, or a failed call.
///
/// systemd's copy is the one asked, not D-Bus's, for the reason the module docs give for
/// writing it first: it is the harder dependency and the half that carries the portals,
/// so it is the half a stale value does the damage through.
fn show_environment() -> Option<String> {
    if !systemd_booted() {
        return None;
    }
    let mut cmd = command("systemctl");
    cmd.args(["--user", "show-environment"]);
    let out = child_spawn::output(&mut cmd).ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Does `key` in the shared environment still hold exactly what we published?
///
/// A plain `KEY=VALUE` line match. systemd quotes values containing whitespace or control
/// characters, which none of ours do — a socket name and a display number — so a quoted
/// line simply fails to match and the value is left alone, which is the safe direction.
fn still_ours(shared: &str, key: &str, value: &str) -> bool {
    shared
        .lines()
        .any(|line| line.split_once('=').is_some_and(|(k, v)| k == key && v == value))
}
