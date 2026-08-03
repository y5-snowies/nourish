//! Put an off-thread producer BELOW the compositor in the scheduler.
//!
//! This is not the small optimisation it looks like. `loader.main/main.priority`
//! puts the whole process on SCHED_RR by default, early, precisely so that every
//! compositor thread inherits it — and the off-thread workers are spawned after
//! that, so they inherit it too. Three background render threads then sit at the
//! SAME realtime priority as the thread that dispatches input, and under load
//! the kernel has no reason to prefer the one whose latency a user can feel.
//! Which is the exact failure the workers were built to remove.
//!
//! `nice` alone cannot fix that: SCHED_RR ignores it. The thread has to be moved
//! back to the normal class first, and only then niced.
//!
//! # Scope
//!
//! Both calls take a TID, and passing `0` means THIS THREAD — despite
//! `sched_setscheduler`'s name and `PRIO_PROCESS`'s, Linux scheduling attributes
//! are per-thread. So this is called as the first statement of a worker's own
//! thread body and can touch nothing else. It cannot leak to a thread that
//! already exists, and cannot leak to one created later from anywhere but this
//! thread. Threads a worker spawns for ITSELF do inherit it (bevy's task pools,
//! for instance), which is the intent: they are that worker's own work.

/// How far below normal. Enough that the compositor and any SCHED_OTHER
/// interactive task win a contended core, small enough that the worker still
/// gets scheduled promptly on an idle one — the common case, where it must keep
/// up with the panel.
const NICE: i32 = 5;

/// Demote the calling thread. Best-effort and never fatal: a worker that keeps
/// realtime priority still renders correctly, it just competes with the
/// compositor it was meant to get out of the way of. Logged once per thread,
/// which is three lines a session.
#[cfg(target_os = "linux")]
pub fn deprioritize(who: &str) {
    // Back to the normal class FIRST. While the thread is SCHED_RR its nice
    // value is not consulted at all, so niceing without this is a no-op that
    // reads like it worked.
    let param = libc::sched_param { sched_priority: 0 };
    if unsafe { libc::sched_setscheduler(0, libc::SCHED_OTHER, &param) } != 0 {
        let e = std::io::Error::last_os_error();
        // EPERM here means the process never got realtime in the first place,
        // which is the normal un-setcap'd case and not worth alarming about.
        trace!("{who}: sched_setscheduler(SCHED_OTHER) declined ({e}); already normal-class?");
    }
    if unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, NICE) } == 0 {
        info!("{who}: running at nice +{NICE}, below the compositor");
    } else {
        let e = std::io::Error::last_os_error();
        warn!("{who}: could not lower priority ({e}); it will compete with the compositor");
    }
}

#[cfg(not(target_os = "linux"))]
pub fn deprioritize(_who: &str) {}
