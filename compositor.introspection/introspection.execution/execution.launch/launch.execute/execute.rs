//! `execute()` — the one place a child is actually spawned.

use std::process::Stdio;

use compositor_introspection_execution_launch_scope::scope::adopt_into_scope;
use compositor_support_library_process_child_hygiene::hygiene::command;
use compositor_support_library_process_child_spawn::spawn::spawn;
use compositor_introspection_execution_launch_types::types::{LaunchOutcome, LaunchRequest};

/// Spawn `req` and return its outcome. The PID is always `Child::id()` — we
/// self-spawn, so it is available synchronously and never polled out of systemd.
/// When `scope` is set the live PID is additionally adopted into a transient
/// systemd `.scope` (best-effort); the caller passes `false` when systemd is
/// unavailable (e.g. a sandbox without systemd as PID 1). The reaper owns
/// reaping; we never `wait` on the child here.
pub fn execute(req: &LaunchRequest, scope: bool) -> LaunchOutcome {
    let mut argv = req.argv.iter();
    let Some(program) = argv.next() else {
        return fail(req, "launch request has no program".into());
    };

    // Signals, fds and CPU priority the app must not inherit — see
    // `process.child/child.hygiene` for what each step undoes and why.
    let mut cmd = command(program);
    cmd.args(argv);
    for (k, v) in &req.env {
        cmd.env(k, v);
    }
    if let Some(dir) = &req.working_dir {
        cmd.current_dir(dir);
    }
    // Matches the historical path: children inherit the compositor's stdio.
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());


    // `child.spawn`, never `Command::spawn` directly: it refuses a program that
    // could not be resolved (the ordinary failure — a plan naming a binary that
    // isn't installed) BEFORE forking, and holds the reaper off for the fork+exec
    // window so std's own wait on a failed exec cannot lose the race.
    let pid = match spawn(&mut cmd) {
        Ok(child) => {
            let pid = child.id();
            // std's Child does not wait on drop, so dropping here cannot race
            // the SIGCHLD reaper; it merely releases our handle.
            drop(child);
            pid
        }
        Err(e) => return fail(req, format!("spawn failed: {e}")),
    };

    if scope {
        if let Err(e) = adopt_into_scope(pid, &req.unit) {
            warn!("scope adoption failed for pid {pid} ({}): {e}", req.unit);
        }
    }

    LaunchOutcome { correlation: req.correlation, token: req.token.clone(), pid: Some(pid), result: Ok(()) }
}

fn fail(req: &LaunchRequest, reason: String) -> LaunchOutcome {
    // Name BOTH resolvable inputs. `spawn` reports a failed `chdir` and a failed
    // `exec` with the same bare ENOENT, so a message carrying neither path sends
    // you looking at the binary when the pinned working directory is what moved.
    warn!("launch failed: {reason} (argv={:?} cwd={:?})", req.argv, req.working_dir);
    LaunchOutcome { correlation: req.correlation, token: req.token.clone(), pid: None, result: Err(reason) }
}
