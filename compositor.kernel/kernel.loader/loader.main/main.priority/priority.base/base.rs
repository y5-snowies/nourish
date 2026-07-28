//! CPU-priority boost for the compositor (settings.json `priority`): under
//! all-core load (e.g. Steam's shader compile) wakeup latency makes commits
//! miss vblank. `"realtime"` (the DEFAULT) = SCHED_RR; `"auto"` = nice (direct,
//! then rtkit over D-Bus). Best-effort; every outcome logged. [`apply`] runs
//! early so ALL compositor threads inherit the boost; the `priority.arm` crate
//! stops child inheritance once startup completes.
//!
//! Both of the weaker settings are hand-edit escape hatches — nothing prompts
//! for this — so the shipped path has to degrade on its own. It does: without
//! CAP_SYS_NICE, `realtime` runs [`auto`], which is the same function `"auto"`
//! itself runs, rtkit fallback included. A machine that cannot grant SCHED_RR
//! ends up exactly where it would have with the previous default.
const NICE: i32 = -10;

/// SCHED_RR priority for `"realtime"`: preempts every CFS task already, stays
/// under pipewire's audio. Kernel RT throttling (95%/s) bounds a runaway.
const RR_PRIORITY: i32 = 2;

/// Apply the policy: `"realtime"` (the default) SCHED_RR, `"auto"` nice, `""`
/// off.
pub fn apply(priority: &str) {
    match priority {
        "" => trace!("priority: disabled (settings.json priority=\"\")"),
        "auto" => auto(),
        "realtime" => realtime(),
        other => warn!(
            "priority: unknown value {other:?} (expected \"\", \"auto\" or \"realtime\"); \
             leaving default scheduling"
        ),
    }
}

/// SCHED_RR; threads spawned afterwards inherit it (intended) until
/// [`arm_reset_on_fork`]. Direct-only; needs CAP_SYS_NICE (the build scripts
/// and the installer setcap it). On failure it calls [`auto`] — the same
/// function, not an approximation of it — so an un-setcap'd machine gets the
/// full nice ladder, rtkit over D-Bus included. Deliberately NOT rtkit's
/// `MakeThreadRealtime`: that would hand back the SCHED_RR this failure just
/// established the machine will not grant directly.
fn realtime() {
    let param = libc::sched_param { sched_priority: RR_PRIORITY };
    if unsafe { libc::sched_setscheduler(0, libc::SCHED_RR, &param) } == 0 {
        info!("priority: SCHED_RR {RR_PRIORITY} applied via sched_setscheduler (direct)");
        return;
    }
    let err = std::io::Error::last_os_error();
    warn!(
        "priority=realtime: sched_setscheduler(SCHED_RR) failed ({err} — is the binary \
         setcap'd with cap_sys_nice?); falling back to the nice ladder"
    );
    auto();
}

fn auto() {
    if unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, NICE) } == 0 {
        info!("priority: nice {NICE} applied via setpriority (direct)");
        return;
    }
    let direct_err = std::io::Error::last_os_error();
    match rtkit_high_priority(NICE) {
        Ok(()) => info!("priority: nice {NICE} applied via rtkit (D-Bus)"),
        Err(rtkit_err) => warn!(
            "priority=auto: could not raise priority (setpriority: {direct_err}; \
             rtkit: {rtkit_err}); running at default scheduling"
        ),
    }
}

fn rtkit_high_priority(nice: i32) -> Result<(), String> {
    let tid = unsafe { libc::gettid() } as u64;
    let conn = zbus::blocking::Connection::system().map_err(|e| format!("system bus: {e}"))?;
    conn.call_method(
        Some("org.freedesktop.RealtimeKit1"),
        "/org/freedesktop/RealtimeKit1",
        Some("org.freedesktop.RealtimeKit1"),
        "MakeThreadHighPriority",
        &(tid, nice),
    )
    .map(|_| ())
    .map_err(|e| format!("MakeThreadHighPriority: {e}"))
}
