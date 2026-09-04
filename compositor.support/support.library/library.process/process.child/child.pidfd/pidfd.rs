//! A file descriptor per child, so a child's exit is an ordinary event.
//!
//! The alternative it replaces is `waitpid(-1)`, which collects ANY child of this
//! process and therefore collides with everything else that waits. Three collisions
//! were live: `Command::spawn` waits on its own child after a failed `exec` under an
//! assertion, so losing that race PANICS the spawning thread; the `status` and `output`
//! wrappers wait on a child they just spawned, and a stolen one makes them report a
//! subprocess failure that never happened; and smithay's Xwayland handling waits on its
//! server the same way. A mutex held across every spawn narrowed the first, and left the
//! rest.
//!
//! None of that is a race here. A descriptor names ONE process, so the only children
//! ever waited on are the ones we opened a descriptor for, and nothing else in the
//! process, or in any library it links, can lose a child to us. There is no interlock,
//! no retry, and no signal handling.
//!
//! Two properties make it fit an event loop. The descriptor becomes readable when the
//! process exits, so it is a source like any other, and it can be opened for a process
//! that has ALREADY become a zombie, so there is no window between a spawn returning and
//! the watch being set up.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::OnceLock;

/// Where a descriptor goes to be watched. Installed once by whoever owns the event
/// loop; every spawn site reaches it through [`watch`] without knowing what it is.
static SINK: OnceLock<Box<dyn Fn(OwnedFd) + Send + Sync>> = OnceLock::new();

/// Name the destination for exit descriptors. Call once, during startup.
///
/// A single installed destination rather than a value threaded through every caller,
/// because the callers are scattered — a kill helper in the selection overlay, the input
/// method launcher, the debug terminal, the app executor — and they have nothing else in
/// common to carry it on. Write-once and read-only afterwards, so no lock is taken on the
/// spawn path.
pub fn install(sink: impl Fn(OwnedFd) + Send + Sync + 'static) {
    if SINK.set(Box::new(sink)).is_err() {
        warn!("exit-descriptor sink installed twice; keeping the first");
    }
}

/// Open a descriptor for `pid` and hand it to the installed destination.
///
/// The whole contract for a caller that spawns a child and then forgets it. Silent when
/// nothing is installed, which is the right answer for a test binary or a tool linking
/// this crate: the child is not watched, exactly as before there was a sink.
pub fn watch(pid: u32) {
    let Some(sink) = SINK.get() else { return };
    match open(pid) {
        Some(fd) => sink(fd),
        None => warn!("no exit descriptor for pid {pid}; it will not be reaped"),
    }
}

/// Open a descriptor for `pid`. `None` if the kernel refuses.
///
/// The refusals are both benign. `ESRCH` means the process is entirely gone, which can
/// only happen if something else already reaped it, so there is nothing left to wait
/// for. `ENOSYS` means a kernel older than 5.3, where the caller keeps whatever fallback
/// it has. A zombie is NOT a refusal: it is still a process until it is waited on, which
/// is exactly why this can be called after a spawn rather than racing it.
pub fn open(pid: u32) -> Option<OwnedFd> {
    // SAFETY: a syscall with no pointer arguments. The returned descriptor is fresh and
    // owned by us, so wrapping it transfers that ownership.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
    match fd {
        fd if fd < 0 => None,
        fd => Some(unsafe { OwnedFd::from_raw_fd(fd as i32) }),
    }
}

/// Reap the process this descriptor names, clearing its zombie. Returns whether it was
/// reaped.
///
/// Call when the descriptor reports readable. Non-blocking, so a spurious wake-up costs
/// a syscall and answers `false` rather than stalling whichever thread asked — which
/// matters, because this is meant to be answered on the event loop rather than on a
/// worker built to absorb a blocking wait.
///
/// Reaping is what makes the process table entry go away. It is worth doing for its own
/// sake and no more than that: a zombie has already exited, so it holds no memory, no
/// threads and no scheduling entity, and cannot compete with the compositor for
/// anything. What accumulating them costs is process ids.
pub fn reap(fd: &OwnedFd) -> bool {
    // SAFETY: `info` is a local out-parameter and the descriptor is ours. `P_PIDFD`
    // makes the id a descriptor rather than a pid, which is what keeps this from
    // naming a process some other pid holder is waiting on.
    unsafe {
        let mut info: libc::siginfo_t = std::mem::zeroed();
        let rc = libc::waitid(
            libc::P_PIDFD,
            fd.as_raw_fd() as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG,
        );
        rc == 0 && info.si_pid() != 0
    }
}
