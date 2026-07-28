//! Composite liveness per output — what the exclusive-pacing watchdog reads.
//!
//! Split out of `tearing.pacer` because it is a different kind of state: that is
//! per-surface protocol data a client writes, this is per-output timing the
//! render loop writes and a timer reads. Per PIPE: with one output compositing
//! at 300fps and another showing a paced desktop, a single shared "last
//! composite" lets the fast one answer for the slow one, which is never rescued.

use compositor_support_smithay_state_tearing_floor::floor::{self, CEILING};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// One output's composite cadence.
struct Pipe {
    last: Instant,
    /// Smoothed interval between PACER-driven composites. Rescue frames are
    /// excluded, so this stays the target's rate instead of drifting onto the
    /// watchdog's own.
    interval: Option<Duration>,
    /// The watchdog, not the pacer, produced this pipe's last composite.
    rescuing: bool,
}

impl Pipe {
    /// How long this pipe may go without a composite before it is rescued.
    ///
    /// Twice the pacer's cadence while the pacer is alive. A target running
    /// slower than the floor is slow, not stalled; firing at the floor against it
    /// interleaves a watchdog frame with nearly every client frame, handing back
    /// exactly the exclusivity the gate exists to enforce.
    ///
    /// The floor FLAT once rescuing: that grace protects a live pacer and there
    /// is none left to interleave with, so the floor becomes what it is
    /// documented to be — the slowest the compositor may run — not half it.
    fn due(&self, floor: Duration) -> Duration {
        match self.rescuing {
            true => floor,
            false => self.interval.map_or(floor, |i| (i * 2).clamp(floor, CEILING)),
        }
    }
}

static PIPES: Mutex<Vec<(String, Pipe)>> = Mutex::new(Vec::new());

/// Whether the frames now arriving are the watchdog's own. Load-bearing: a
/// rescue lands at `delta ~= 2 x interval`, so folding it in gives
/// `interval' = 1.25 x interval` — a compounding loop that reaches `CEILING`
/// (4fps) about nine rescues, a second, after any stall, and pins there.
static RESCUE: AtomicBool = AtomicBool::new(false);

/// Record that `key` composited. Called on every frame, paced or not.
pub fn note_composite(key: &str) {
    let now = Instant::now();
    let rescue = RESCUE.load(Ordering::Relaxed);
    let Ok(mut pipes) = PIPES.lock() else { return };
    match pipes.iter_mut().find(|(k, _)| k == key) {
        Some((_, p)) => {
            let delta = now.duration_since(p.last);
            // Measure the CADENCE only from on-time pacer frames: a rescue is the
            // watchdog's rate, and the frame that ends a stall measures the stall.
            // EWMA over ~4 frames, so one long composite (a shader hitch, a mode
            // set) cannot push the threshold out for the frames behind it.
            if !rescue && delta < p.due(floor::get()) {
                p.interval = Some(p.interval.map_or(delta, |prev| (prev * 3 + delta) / 4));
            }
            (p.rescuing, p.last) = (rescue, now);
        }
        None => pipes.push((key.to_string(), Pipe { last: now, interval: None, rescuing: rescue })),
    }
}

/// Has any pipe gone longer than its OWN due time without compositing? Empty
/// (nothing recorded yet, or just reset) counts as stalled: engagement has just
/// begun and one frame is what establishes the cadence.
pub fn stalled() -> bool {
    let (f, now) = (floor::get(), Instant::now());
    let over = |(_, p): &(String, Pipe)| now.duration_since(p.last) >= p.due(f);
    PIPES.lock().map(|v| v.is_empty() || v.iter().any(over)).unwrap_or(true)
}

/// Attribute the frames about to arrive — called at the top of every watchdog
/// tick with that tick's verdict, so the flag is re-decided as often as it is
/// read and a resumed pacer is misread for at most one tick.
pub fn set_rescue(on: bool) {
    RESCUE.store(on, Ordering::Relaxed);
}

/// Forget every measured cadence. Called when the gate disengages, so the next
/// engagement measures its own pacer instead of inheriting the previous one's.
pub fn reset() {
    RESCUE.store(false, Ordering::Relaxed);
    if let Ok(mut pipes) = PIPES.lock() {
        pipes.clear();
    }
}
