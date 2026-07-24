//! CPU-priority boost for the compositor (settings.json `priority`): under
//! all-core load (e.g. Steam's shader compile) wakeup latency makes commits
//! miss vblank. `"auto"` = nice (direct, then rtkit over D-Bus); `"realtime"`
//! = SCHED_RR. Best-effort; every outcome logged. [`apply`] runs early so ALL
//! compositor threads inherit the boost; the `priority.arm` crate stops child
//! inheritance once startup completes.
const NICE: i32 = -10;

/// SCHED_RR priority for `"realtime"`: preempts every CFS task already, stays
/// under pipewire's audio. Kernel RT throttling (95%/s) bounds a runaway.
const RR_PRIORITY: i32 = 2;

/// Apply the policy: `""` off, `"auto"` nice, `"realtime"` SCHED_RR.
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
/// setcap it); on failure degrades to `auto`.
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
