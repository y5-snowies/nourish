//! VBlank timestamp interpretation + refresh math. Pure value work over
//! caller-supplied inputs (Law 1: no loop types).

use smithay::output::Mode;
use smithay::wayland::presentation::Refresh;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct VblankStamp {
    pub time: Duration,
    pub sequence: u64,
}

/// Interpret the kernel-supplied (or absent) vblank timestamp, falling back
/// to the monotonic-now the caller measured (behavior carried verbatim).
pub fn interpret(time: Option<Duration>, sequence: u64, fallback_now: Duration) -> VblankStamp {
    VblankStamp {
        time: time.unwrap_or(fallback_now),
        sequence,
    }
}

/// Refresh descriptor for presentation feedback (mode.refresh is mHz).
pub fn refresh_interval(mode: &Mode) -> Refresh {
    Refresh::fixed(interval(mode))
}

/// The plain refresh interval as a Duration — what the Law-7 timing nets
/// (throttle / predict / estimate) compute against.
pub fn interval(mode: &Mode) -> Duration {
    Duration::from_secs_f64(1_000f64 / mode.refresh.max(1) as f64)
}

/// A dispatch delay this large means the event's timestamp is not what we think
/// it is (a different clock, a stale field). Fall back rather than anchor the
/// phase to nonsense.
const MAX_DISPATCH_DELAY: Duration = Duration::from_millis(200);

/// `CLOCK_MONOTONIC` now — the same clock DRM stamps `DrmEventTime::Monotonic`
/// with, and the same one `Instant` reads on Linux. `std::time::Instant` cannot
/// expose its raw value, so a direct read is the only way to relate the two.
pub fn monotonic_now() -> Option<Duration> {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `clock_gettime` only writes the `timespec` we hand it.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } != 0 {
        return None;
    }
    Some(Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32))
}

/// The `Instant` at which a retrace ACTUALLY occurred, given when we observed
/// its event and the kernel's own timestamp for it.
///
/// We see a completion event some dispatch delay after the retrace, and for an
/// async (tearing) flip the event fires mid-scanout, so `observed` is not a
/// vblank at all. Subtracting `kernel_now - vblank_kernel` recovers the true
/// retrace instant. Any true retrace anchors the phase equally well — they are
/// all congruent modulo the refresh interval — so a slightly stale timestamp is
/// still exact; only clock drift since then costs accuracy.
///
/// Falls back to `observed` when the driver reported a non-monotonic timestamp
/// (`vblank_kernel: None`), the clock read failed, or the delay is implausible.
pub fn anchor(observed: Instant, vblank_kernel: Option<Duration>, kernel_now: Option<Duration>) -> Instant {
    let delay = match (vblank_kernel, kernel_now) {
        (Some(vblank), Some(now)) => now.checked_sub(vblank),
        _ => None,
    };
    delay
        .filter(|d| *d <= MAX_DISPATCH_DELAY)
        .and_then(|d| observed.checked_sub(d))
        .unwrap_or(observed)
}

/// Time remaining until the next retrace, extrapolated from `anchor` by the
/// refresh interval. Correct across arbitrarily many elapsed intervals.
pub fn until_next(anchor: Instant, now: Instant, refresh: Duration) -> Duration {
    let period = refresh.as_nanos().max(1);
    let phase = (now.saturating_duration_since(anchor).as_nanos() % period) as u64;
    refresh.saturating_sub(Duration::from_nanos(phase))
}
