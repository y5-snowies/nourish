//! When a pane may render: the cadence, and the ceiling over it.
//!
//! Two gates, both of which must pass. `rate`/`cadence` express the INTENT —
//! follow the panel, or a wall-clock interval. `ceiling` is the LAST WORD, so a
//! cadence driven by a high-refresh or tearing host cannot outrun what the
//! machine was told to hold.

use compositor_background_two_worker_key::key::PaneKey;
use compositor_background_two_worker_pane::pane::Pane;
use compositor_model_environment_background_base::base::{Cadence, Rate, TripleBufferBackground};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Retraces between renders for `rate` on a `refresh` panel; at least one.
fn divisor(rate: Rate, refresh: Duration) -> u64 {
    let Some(i) = rate.min_interval(refresh) else { return 1 };
    ((i.as_secs_f64() / refresh.as_secs_f64()).round() as u64).max(1)
}
/// Multiple of the interval the wall-clock fallback waits before overriding the
/// retrace count. It must NEVER participate in the steady state: at 1x it fires
/// when the divisor does, the two drift, and spacing alternates between "N
/// retraces" and "whenever the clock said so" — uneven, and it reads as stutter.
/// At 3x it is unreachable unless the serial has genuinely stopped.
const STALL: u32 = 3;

/// One gate: is `rate`, counted against `cadence`, satisfied for this pane?
///
/// `due_at` is this gate's next scheduled slot, used only by the wall-clock
/// branch. There is deliberately no tolerance term: the schedule already absorbs
/// what a tolerance used to paper over — a render stamped at FINISH rather than
/// start (a systematic undershoot proportional to render time) and the worker's
/// own poll granularity. Half an interval of slack over-corrected both, and on a
/// host where the serial gate is wide open it let a "1x refresh" cap admit 2x.
fn gate(rate: Rate, cadence: Cadence, p: &Pane, serial: u64, due_at: Option<Instant>) -> bool {
    match cadence {
        Cadence::Vblank => {
            // Exact in serial space: counting cannot drift, so a frame is never
            // dropped for sub-frame jitter.
            let need = divisor(rate, p.refresh);
            let by_serial = p.last_serial.is_none_or(|s| serial.wrapping_sub(s) >= need);
            // A STALLED serial must not wedge us: it only advances on a compositor
            // scene build, and while the background is the only thing animating the
            // compositor only builds one because we published — so waiting on a
            // divisor we can no longer reach is a mutual deadlock. Deliberately NOT
            // also gated on the serial having moved: when the loop has genuinely
            // stopped the serial is FROZEN, which would make the rescue unreachable
            // in the one case it exists for. Uncapped has no interval of its own,
            // so the retrace stands in.
            let base = rate.min_interval(p.refresh).unwrap_or(p.refresh);
            by_serial || p.last.is_some_and(|t| t.elapsed() >= base * STALL)
        }
        // Uncapped has no schedule; a pane that has never rendered is due now.
        Cadence::Timer => match (rate.min_interval(p.refresh), due_at) {
            (Some(_), Some(d)) => Instant::now() >= d,
            (Some(_), None) => true,
            (None, _) => true,
        },
    }
}

/// May this pane render now? Both gates must pass: `rate`/`cadence` express the
/// INTENT, `ceiling`/`ceiling_cadence` are the LAST WORD.
///
/// The two carry their own cadence because the useful pairing is a rate counted
/// in vblanks — phase-locked, no timer drifting against the retrace — under a cap
/// expressed against the panel. A single cadence could not say that: it forced
/// the cap into the rate's space, so under `Vblank` the ceiling was counted in
/// COMPOSITES and stopped bounding anything to the display on a tearing host.
pub fn due(p: &Pane, tb: &TripleBufferBackground, serial: u64) -> bool {
    gate(tb.rate, tb.cadence, p, serial, p.due_rate)
        && gate(tb.ceiling, tb.ceiling_cadence, p, serial, p.due_cap)
}

/// Time until this pane is due again, or `None` if it is due now.
///
/// A known deadline WINS over the poll interval, in both directions. Sleeping
/// `max(poll, deadline)` rounded a sub-millisecond remainder up to the poll and
/// landed the render that much off its slot; sleeping `min` would burn wakeups
/// re-asking a question that cannot change yet. Nothing can pass before the
/// latest deadline, so wait exactly that long — and only fall back to polling
/// once no deadline is outstanding and the serial is all that is left to watch.
pub fn until_due(p: &Pane, tb: &TripleBufferBackground) -> Option<Duration> {
    let now = Instant::now();
    let remain = |slot: Option<Instant>| {
        slot.and_then(|d| d.checked_duration_since(now)).filter(|d| !d.is_zero())
    };
    let deadline = [(tb.cadence, p.due_rate), (tb.ceiling_cadence, p.due_cap)]
        .into_iter()
        .filter(|(c, _)| matches!(c, Cadence::Timer))
        .filter_map(|(_, slot)| remain(slot))
        .max();
    // The serial only moves on a retrace, so waking a few times per refresh
    // notices it just as well as a 1ms poll and costs a Pi far less CPU.
    let polls = matches!(tb.cadence, Cadence::Vblank)
        || matches!(tb.ceiling_cadence, Cadence::Vblank);
    let wait = deadline.or_else(|| polls.then(|| p.refresh / 4));
    // A PIPELINED frame still owes a fence poll and a publish, and that is work
    // the render deadline knows nothing about — it is retired at the top of the
    // next pass (`serve_pane`), not on a schedule. Sleeping to the next render
    // slot would hold a finished frame back a whole interval, which is precisely
    // the wait pipelining exists to remove. Cap the nap while one is outstanding.
    match (wait, p.pending.is_some()) {
        (Some(w), true) => Some(w.min(p.refresh / 4)),
        (w, _) => w,
    }
}

/// Time until the earliest pane comes due, or `None` if any is due now.
pub fn until_any_due(panes: &HashMap<PaneKey, Pane>, tb: &TripleBufferBackground) -> Option<Duration> {
    panes.values().filter_map(|p| until_due(p, tb)).min()
}
