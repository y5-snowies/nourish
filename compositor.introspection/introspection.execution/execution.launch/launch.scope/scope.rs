//! `StartTransientUnit` against `org.freedesktop.systemd1` (user bus), and the
//! per-session slice that makes those scopes collectable again.

use zbus::blocking::Connection;
use zbus::zvariant::Value;

// Every spawn goes through the hygiene wrapper — see `process.child`.
use compositor_support_library_process_child_hygiene::hygiene::command;
use compositor_support_library_process_child_spawn::spawn as child_spawn;

/// The slice every app this compositor launches is placed under.
///
/// **The single source of truth for the name**, because two callers must agree on it and
/// there is nowhere else to record it: [`adopt_into_scope`] writes it at launch, and
/// [`stop_session_slice`] stops it at shutdown, with the process exiting in between.
///
/// UNDER `app.slice`, which is what the leading `app-` means: a dash is systemd's slice
/// hierarchy separator, so this resolves to `app.slice` -> `app-y5.slice` ->
/// `app-y5-<pid>.slice`, and the intermediate is created along with it. Staying inside
/// `app.slice` is the point of the prefix rather than decoration. That slice is where a
/// desktop's applications are expected to live — systemd ships it for the user manager
/// with `CPUWeight=100`, against `background.slice` at 30 — and it is the subtree that
/// resource control and `systemd-oomd` policies are written against. A top-level slice of
/// our own would be stoppable just the same and would quietly opt every app y5 launches
/// out of all of it.
///
/// Discriminated by our own pid, and that is the other half. The systemd USER manager is
/// shared by every session the same user runs, so `app.slice` ITSELF — the systemd
/// default, and what this used to pass — mixes our apps in with every other session's.
/// Stopping that, or anything matched by a shared name prefix, would kill the apps of a
/// session running on another VT. A pid is unique, already available, and needs no
/// plumbing or registry.
///
/// The slice is created implicitly by naming it: a slice unit needs no unit file, which is
/// exactly how `systemd-run --slice=` accepts an arbitrary name. Nothing creates it up
/// front, so there is no startup step that can fail and no state to keep in sync.
pub fn session_slice() -> String {
    format!("app-y5-{}.slice", std::process::id())
}

/// Stop this session's slice, and with it every app the session launched.
///
/// The BACKSTOP, not the normal path. A well-behaved client exits when the compositor's
/// socket closes — an X11 app loses Xwayland and quits, a wayland app sees its display go
/// — and needs nothing from here. What this exists for is the client that ignores the
/// disconnection entirely, which games routinely do, and which would otherwise keep
/// running with no display and no owner: its scope lives under the user manager, which
/// outlives the session and has no reason to collect a scope nobody asked it to.
///
/// Stopping the SLICE rather than the scopes one by one is what makes this complete. It
/// takes the whole cgroup, so a process that double-forked away from the pid we adopted
/// is still inside it and still goes.
///
/// Fire-and-forget: the call queues a job with the user manager, which carries it out
/// whether or not we are still here to watch, and waiting could only delay the exit.
/// Best-effort like the adoption it mirrors — a failure means apps outlive the session,
/// which is what happened before this existed.
///
/// TWO TRANSPORTS, and the fallback is not redundancy. The bus call needs a session bus
/// (`DBUS_SESSION_BUS_ADDRESS`, or `$XDG_RUNTIME_DIR/bus`); `systemctl --user` does not —
/// it falls back to the user manager's private socket at
/// `$XDG_RUNTIME_DIR/systemd/private`, which is how systemd manages user units in an
/// environment with no `dbus-daemon` at all. The two are not interchangeable and systemd
/// is the harder dependency of the pair: y5 already requires the user manager, while the
/// session bus is a separate daemon that can be absent, and — the case this is really
/// for — can be torn down BEFORE us at logout. That is the moment the slice most needs
/// stopping and the moment the bus is least likely to answer.
pub fn stop_session_slice() -> Result<(), String> {
    let slice = session_slice();
    let bus = stop_over_bus(&slice);
    if bus.is_ok() {
        return Ok(());
    }
    match systemctl(&["--user", "stop", "--no-block", &slice]) {
        Ok(()) => {
            info!("session slice stopped through systemctl; the session bus did not answer");
            Ok(())
        }
        // Both, because which one failed is the whole diagnosis: no bus is ordinary at
        // logout, no systemd is a different machine entirely.
        Err(cli) => Err(format!("{}; {cli}", bus.unwrap_err())),
    }
}

fn stop_over_bus(slice: &str) -> Result<(), String> {
    let conn = Connection::session().map_err(|e| format!("session bus: {e}"))?;
    conn.call_method(
        Some("org.freedesktop.systemd1"),
        "/org/freedesktop/systemd1",
        Some("org.freedesktop.systemd1.Manager"),
        "StopUnit",
        &(slice, "replace"),
    )
    .map(|_| ())
    .map_err(|e| format!("StopUnit({slice}): {e}"))
}

/// Run `systemctl` and treat a non-zero exit as an error, same as a spawn failure —
/// both mean the unit was not acted on.
fn systemctl(args: &[&str]) -> Result<(), String> {
    let mut cmd = command("systemctl");
    cmd.args(args);
    match child_spawn::status(&mut cmd) {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("systemctl {} exited with {status}", args.join(" "))),
        Err(err) => Err(format!("could not run systemctl: {err}")),
    }
}

/// Adopt `pid` into a transient scope named after `unit` (`.scope` appended if
/// absent). Best-effort: the process is already running, so a failure here only
/// means it misses cgroup isolation, not that the launch failed.
///
/// BUS ONLY, unlike [`stop_session_slice`], and not for want of trying: no systemd CLI
/// adopts an existing pid. `systemd-run --user --scope` STARTS a process in a new scope;
/// there is no verb for "take this one", because `StartTransientUnit` with a `PIDs=`
/// property is the only interface systemd exposes for it. `busctl` would carry the same
/// call but needs the bus just as much, so it buys nothing.
///
/// Degrading consistently is what makes that acceptable. With no session bus nothing is
/// adopted, so the slice has no members and the stop at shutdown has nothing to collect —
/// apps simply keep the placement they were launched with. The fallback in
/// `stop_session_slice` still earns its keep for the ordinary case of a bus that was
/// present at launch and gone by logout.
pub fn adopt_into_scope(pid: u32, unit: &str) -> Result<(), String> {
    let conn = Connection::session().map_err(|e| format!("session bus: {e}"))?;

    let name = if unit.ends_with(".scope") {
        unit.to_string()
    } else {
        format!("{unit}.scope")
    };

    // Manager.StartTransientUnit(name: s, mode: s, properties: a(sv), aux: a(sa(sv)))
    let properties: Vec<(&str, Value)> = vec![
        ("PIDs", Value::from(vec![pid])),
        // Ours, not `app.slice`: see `session_slice` for why the default cannot be
        // cleaned up and this can.
        ("Slice", Value::from(session_slice())),
        ("CollectMode", Value::from("inactive-or-failed")),
    ];
    let aux: Vec<(&str, Vec<(&str, Value)>)> = Vec::new();

    conn.call_method(
        Some("org.freedesktop.systemd1"),
        "/org/freedesktop/systemd1",
        Some("org.freedesktop.systemd1.Manager"),
        "StartTransientUnit",
        &(name.as_str(), "fail", properties, aux),
    )
    .map(|_| ())
    .map_err(|e| format!("StartTransientUnit({name}): {e}"))
}
