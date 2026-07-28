//! `execute()` — the one place a child is actually spawned.

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use compositor_introspection_execution_launch_scope::scope::adopt_into_scope;
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

    let mut cmd = Command::new(program);
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

    // Child hygiene applied between fork and exec. SAFETY: the closure runs in
    // the forked child before exec; every call below is async-signal-safe (a
    // single syscall or a sigset op), allocates nothing, and touches no shared
    // state.
    unsafe {
        cmd.pre_exec(|| {
            // (1) Reset the signal mask. We block SIGCHLD process-wide for the
            // reaper's signalfd and `execve` preserves the mask — so without this
            // apps inherit a blocked SIGCHLD and ones that rely on it (e.g.
            // alacritty detecting its shell exiting) never close.
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::pthread_sigmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());

            // (2) Don't leak inherited fds into the app: the compositor's DRM
            // master / GPU nodes / wayland sockets / dmabuf / syncobj / event-loop
            // epoll+eventfds, AND whatever the launching harness leaked into us.
            // Mark every fd >= 3 close-on-exec so they all close at the imminent
            // exec; stdio (0/1/2) is kept. CLOSE_RANGE_CLOEXEC defers the close to
            // exec (rather than closing now), which leaves Rust's CLOEXEC
            // error-report pipe usable so spawn-failure detection still works.
            // Best-effort — ignore the result (e.g. pre-5.11 kernels).
            // (Scheduling needs no reset here: the compositor arms
            // SCHED_RESET_ON_FORK after startup, so forked children already
            // start at default policy/nice despite the `priority` boost.)
            libc::close_range(3, libc::c_uint::MAX, libc::CLOSE_RANGE_CLOEXEC as libc::c_int);

            Ok(())
        });
    }

    let pid = match cmd.spawn() {
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
    warn!("launch failed: {reason}");
    LaunchOutcome { correlation: req.correlation, token: req.token.clone(), pid: None, result: Err(reason) }
}
