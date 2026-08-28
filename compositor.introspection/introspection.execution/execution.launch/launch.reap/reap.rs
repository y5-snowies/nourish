//! Non-blocking drain of exited children.

use compositor_support_library_process_child_spawn::spawn::try_reap_guard;

/// Reap every child that has exited since the last call, returning their pids
/// (for logging / restoration cleanup). Never blocks: `WNOHANG` makes `waitpid`
/// return 0 once no more children have exited, and the interlock below is
/// ASKED for rather than waited on.
///
/// `None` means a spawn held the interlock and nothing was reaped. That is a
/// safe answer, not a failure: zombies stay collectable, and the next child to
/// exit raises SIGCHLD again. Whether to retry is the caller's to decide,
/// because only the caller knows what thread it is on — the launch worker can
/// afford to wait a few milliseconds; the calloop thread cannot.
///
/// We intentionally `waitpid(-1, …)` for ANY child rather than tracking a set,
/// so re-parented grandchildren are collected too. That is also why the
/// interlock exists at all: a spawn in flight owns a child std itself will wait
/// on, and collecting that one panics the spawning thread (see
/// `child.spawn::try_reap_guard`).
pub fn reap_zombies() -> Option<Vec<u32>> {
    let _spawning = try_reap_guard()?;
    let mut reaped = Vec::new();
    let mut status: libc::c_int = 0;
    loop {
        // SAFETY: waitpid with a local status out-pointer; no other invariants.
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if pid <= 0 {
            // 0 = children exist but none exited; -1 = no children / error.
            break;
        }
        reaped.push(pid as u32);
    }
    Some(reaped)
}
