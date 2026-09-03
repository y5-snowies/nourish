//! `command()` / `install()` — attach the between-fork-and-exec hook to a
//! `Command`. EVERY process the compositor spawns goes through one of them.

use std::ffi::OsStr;
use std::os::unix::process::CommandExt;
use std::process::Command;

/// `Command::new` with [`apply`] already installed — the form to use when the
/// command is built as a chain. Prefer this over bare `Command::new` at every
/// spawn site: a one-shot `ffprobe` inherits the same signal mask, fds and CPU
/// priority a long-lived app does.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    install(&mut cmd);
    cmd
}

/// Install [`apply`] as `cmd`'s `pre_exec` hook — for a `Command` built (or
/// received) elsewhere.
pub fn install(cmd: &mut Command) -> &mut Command {
    // SAFETY: `apply` is async-signal-safe — see its own contract.
    unsafe { cmd.pre_exec(apply) }
}

/// SAFETY: runs in the forked child before exec; every call is
/// async-signal-safe (a single syscall or a sigset op), allocates nothing and
/// touches no shared state. Best-effort throughout — a failure here degrades
/// the child's environment, it must never fail the spawn. Rust dup2's the
/// configured stdio onto 0/1/2 BEFORE pre_exec hooks run, so piped stdio
/// survives step (2).
pub fn apply() -> std::io::Result<()> {
    unsafe {
        // (1) Reset the signal mask. `execve` preserves it, and the compositor blocks
        // SIGTERM and SIGINT process-wide so its shutdown signalfd is their only
        // consumer (`loader::shutdown`) — so without this every launched app inherits a
        // blocked SIGTERM and ignores every request to quit, including the one systemd
        // sends at logout. Emptying the whole set rather than unblocking named signals
        // is deliberate: what matters is that the child starts from a clean mask,
        // whatever the compositor happens to block next.
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::pthread_sigmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());

        // (2) Don't leak inherited fds: the compositor's DRM master / GPU nodes
        // / wayland sockets / dmabuf / syncobj / event-loop epoll+eventfds, AND
        // whatever the launching harness leaked into us. Mark every fd >= 3
        // close-on-exec so they all close at the imminent exec; stdio (0/1/2)
        // is kept. CLOSE_RANGE_CLOEXEC defers the close to exec (rather than
        // closing now), which leaves Rust's CLOEXEC error-report pipe usable so
        // spawn-failure detection still works. Ignore the result (e.g. pre-5.11
        // kernels).
        libc::close_range(3, libc::c_uint::MAX, libc::CLOSE_RANGE_CLOEXEC as libc::c_int);

        // (3) Drop the compositor's `priority` boost (settings.json `priority`).
        // `priority.arm` arms SCHED_RESET_ON_FORK, but that attribute is
        // per-THREAD and covers only tasks the ARMING thread goes on to create:
        // a fork from any thread that predates the arm — the `y5-launch` worker
        // among them — still handed the raw boost down, so apps came up
        // SCHED_RR (top: PR -3, NI 0) under priority=realtime and nice -10
        // under priority=auto, competing with input dispatch and breaking
        // clients that manage their own thread priorities (firefox). Arming is
        // also only best-effort (it can fail, and does nothing when `priority`
        // is disabled), so the child side is the one reliable place. Do what
        // RESET_ON_FORK would: realtime class → SCHED_OTHER, negative nice → 0.
        // Both only LOWER our own priority, which is always permitted — no
        // capability needed, whatever granted the boost (setcap or rtkit).
        let policy = libc::sched_getscheduler(0);
        if policy == libc::SCHED_RR || policy == libc::SCHED_FIFO {
            let param = libc::sched_param { sched_priority: 0 };
            libc::sched_setscheduler(0, libc::SCHED_OTHER, &param);
        }
        if libc::getpriority(libc::PRIO_PROCESS, 0) < 0 {
            libc::setpriority(libc::PRIO_PROCESS, 0, 0);
        }
    }
    Ok(())
}
