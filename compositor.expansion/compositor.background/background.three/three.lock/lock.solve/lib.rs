//! Spring-solver helpers for the morph animation phases.

use compositor_background_three_lock_spring::{Solver, solve};
use std::time::Duration;

pub const SPRING_STIFFNESS: f64 = 154.0;
pub const SPRING_DAMPING: f64 = 9.54;
pub const SPRING_MASS: f64 = 0.25;

pub const BAND_DURATION_MS: u64 = 1500;
pub const BAND_STAGGER_MS: u64 = 1250;

pub fn band_solver(value_start: f64, value_target: f64, duration_ms: u64) -> Solver {
    Solver {
        stiffness: SPRING_STIFFNESS,
        damping: SPRING_DAMPING,
        mass: SPRING_MASS,
        value_start,
        value_target,
        duration: Duration::from_millis(duration_ms),
    }
}

/// Solve a band's flatness given the parent-phase time. `from→to` is the
/// band's value trajectory.
pub fn solve_band(t_phase: Duration, band_start_ms: u64, from: f64, to: f64) -> f32 {
    let start = Duration::from_millis(band_start_ms);
    if t_phase < start {
        return from as f32;
    }
    let t_band = t_phase - start;
    let solver = band_solver(from, to, BAND_DURATION_MS);
    solve(&solver, t_band).unwrap_or(to) as f32
}

pub fn solve_phase_progress(t_phase: Duration, duration_ms: u64, forward: bool) -> f32 {
    // EXPERIMENT: was a spring mapped into a fixed 250ms window. The spring
    // settled by ~60% of the phase, so all visible motion front-loaded into
    // the first third and read as an instant snap regardless of duration.
    // Now: linear phase progress eased with a symmetric smoothstep, so the
    // travel is spread evenly across the ENTIRE phase.
    let phase_progress = (t_phase.as_secs_f64() / (duration_ms as f64 / 1000.0)).clamp(0.0, 1.0);
    // smoothstep (3t²-2t³): gentle ease in/out, no early settle, no overshoot.
    let eased = phase_progress * phase_progress * (3.0 - 2.0 * phase_progress);
    let (from, to) = if forward { (0.0, 1.0) } else { (1.0, 0.0) };
    (from + (to - from) * eased) as f32
}
