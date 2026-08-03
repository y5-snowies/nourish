//! Late-phase scheduling, called once startup threading is complete (right
//! before the IME launch): arms `SCHED_RESET_ON_FORK` so tasks created from
//! here on — forked children AND late threads — start at default scheduling
//! instead of inheriting the `priority` boost. Deferred so the initial
//! compositor threads DO inherit it.
//!
//! `SCHED_RESET_ON_FORK` is per-THREAD (`sched_setscheduler(0, …)` acts on the
//! caller, not the process), so this covers only what the calling thread goes
//! on to create. Threads that already exist — the `y5-launch` worker among
//! them — keep the raw boost and hand it to anything THEY fork; a fork site on
//! such a thread must reset scheduling itself between fork and exec (see
//! `launch.execute`).

/// Arm the inheritance stop for the boosted modes; no-op when disabled.
pub fn arm(priority: &str) {
    if priority == "auto" || priority == "realtime" {
        arm_reset_on_fork();
    }
}

/// Re-assert the CURRENT policy/priority plus `SCHED_RESET_ON_FORK`, so
/// whatever `apply` achieved is kept (a realtime→auto fallback arms as
/// SCHED_OTHER, not RR) and new tasks start at default scheduling.
fn arm_reset_on_fork() {
    let mut param = libc::sched_param { sched_priority: 0 };
    let policy = unsafe {
        libc::sched_getparam(0, &mut param);
        libc::sched_getscheduler(0)
    };
    if policy < 0 {
        return warn!("priority: sched_getscheduler failed; SCHED_RESET_ON_FORK not armed");
    }
    if unsafe { libc::sched_setscheduler(0, policy | libc::SCHED_RESET_ON_FORK, &param) } == 0 {
        info!("priority: SCHED_RESET_ON_FORK armed — new threads/children start at default scheduling");
    } else {
        let err = std::io::Error::last_os_error();
        warn!("priority: could not arm SCHED_RESET_ON_FORK ({err}); children will inherit the boost");
    }
}
