//! Late-phase scheduling, called once startup threading is complete (right
//! before the IME launch): arms `SCHED_RESET_ON_FORK` so tasks created from
//! here on — forked children AND late threads — start at default scheduling
//! instead of inheriting the `priority` boost. Deferred so the initial
//! compositor threads DO inherit it.

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
