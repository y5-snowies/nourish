use std::time::Instant;
use compositor_support_smithay_state_fractional_config::{DebounceCycle, FractionalScaleConfig};

/// Snap a raw zoom value into the configured scale lattice.
pub fn snap(cfg: &FractionalScaleConfig, zoom: f64) -> f64 {
    let clamped = zoom.clamp(cfg.min_scale, cfg.max_scale);
    let step = cfg.step.max(f64::EPSILON);
    (clamped / step).round() * step
}

/// Whether enough time has passed since the last emit.
pub fn rate_limit_clear(cfg: &FractionalScaleConfig, last_emit_at: Option<Instant>, now: Instant) -> bool {
    match last_emit_at {
        None => true,
        Some(t) => now.duration_since(t) >= cfg.min_interval,
    }
}

/// Result of a single debounce tick.
pub struct TickResult {
    pub last_observed: Option<u64>,
    pub cycle: Option<DebounceCycle>,
    pub armed: bool,
    pub last_emit_at: Option<Instant>,
    pub fire: bool,
}

/// One debounce tick over the PENDING EMIT SET, fingerprinted by `pending`
/// (`None` == nothing pending).
///
/// Observing the batch itself rather than a proxy signal is what makes ONE cycle
/// cover every source of churn: a zoom ease crossing lattice rungs, a drift pan
/// pushing windows off the pane, a window moving to a differently-zoomed pane.
/// Any change to the fingerprint restarts the quiet window; the tick fires once the
/// batch has held steady for `debounce_quiet`, or unconditionally at `debounce_max`,
/// subject to `min_interval`. Firing clears the cycle and the caller submits the
/// whole batch at once.
pub fn run_tick(
    cfg: &FractionalScaleConfig,
    last_observed: Option<u64>,
    cycle: Option<DebounceCycle>,
    armed: bool,
    last_emit_at: Option<Instant>,
    pending: Option<u64>,
) -> TickResult {
    let now = Instant::now();
    let Some(pending) = pending else {
        return TickResult { last_observed: None, cycle: None, armed: false, last_emit_at, fire: false };
    };

    let mut cycle = cycle;
    if last_observed != Some(pending) {
        // `started_at` survives the restart, so `debounce_max` still bounds a batch
        // that keeps churning.
        let started_at = cycle.map_or(now, |c| c.started_at);
        cycle = Some(DebounceCycle { started_at, quiet_after: now + cfg.debounce_quiet });
    }

    // `armed` is sticky once set: a batch that changes again after the cycle
    // elapsed still fires at the next clear interval rather than restarting.
    let mut armed = armed;
    if let Some(c) = cycle {
        if now >= c.quiet_after || now.duration_since(c.started_at) >= cfg.debounce_max {
            armed = true;
            cycle = None;
        }
    }

    let mut fire = false;
    let mut last_emit_at = last_emit_at;
    if armed && rate_limit_clear(cfg, last_emit_at, now) {
        armed = false;
        last_emit_at = Some(now);
        fire = true;
    }

    TickResult { last_observed: Some(pending), cycle, armed, last_emit_at, fire }
}
