//! Phase state machine: advances `MorphAnim` every frame.

use bevy::prelude::*;
use compositor_background_three_lock_solve::solve_phase_progress;
use compositor_background_three_lock_state::{MorphAnim, MorphPhase, SNAPSHOT_LABEL};
use compositor_support_bevy_core_bridge_base::BridgeRegistry;
use std::time::Duration;

// PRE_MORPH_DELAY_MS is a safety TIMEOUT only: the fold starts the instant the cast
// is bridged in (see PreMorphDelay), so it no longer gates the perceived idle.
pub const PRE_MORPH_DELAY_MS: u64 = 500;
pub const MORPH_DURATION_MS: u64 = 1400; // primary speed lever (was 250)
pub const SPHERE_FULL_HOLD_MS: u64 = 800;
pub const SHRINK_DURATION_MS: u64 = 700;
pub const HERO_HOLD_MS: u64 = 3000; // hero hold (resume is command-driven)
pub const GROW_DURATION_MS: u64 = 700;
pub const UNMORPH_DURATION_MS: u64 = MORPH_DURATION_MS;

/// True once the captured screen ("cast") is bridged into the Bevy `GpuImage`.
/// Until then the plane samples the blank placeholder, so we must not fold yet.
fn snapshot_ready(bridge: &Option<Res<BridgeRegistry>>) -> bool {
    let Some(b) = bridge else { return false };
    let Ok(entries) = b.entries.lock() else { return false };
    entries.iter().any(|e| e.label == SNAPSHOT_LABEL && e.installed.lock().map(|g| *g).unwrap_or(false))
}

pub fn tick_animation(mut anim: ResMut<MorphAnim>, time: Res<Time>, bridge: Option<Res<BridgeRegistry>>) {
    let now = time.elapsed_secs_f64();
    let t_phase = Duration::from_secs_f64((now - anim.phase_started_at).max(0.0));
    // Plane may draw only once the cast is bridged in (gates the first-frame flash).
    let ready = snapshot_ready(&bridge);
    anim.render_ready = if ready { 1.0 } else { 0.0 };

    match anim.phase {
        MorphPhase::Idle => { anim.t = 1.0; anim.going_to_sphere = 0.0; anim.hero = 0.0; }
        MorphPhase::PreMorphDelay => {
            anim.t = 0.0; anim.going_to_sphere = 1.0; anim.hero = 0.0;
            // Fold the moment the cast is on screen (~1 frame); timeout is a fallback.
            if ready || t_phase >= Duration::from_millis(PRE_MORPH_DELAY_MS) {
                anim.phase = MorphPhase::Morphing; anim.phase_started_at = now;
            }
        }
        MorphPhase::Morphing => {
            anim.going_to_sphere = 1.0;
            anim.t = solve_phase_progress(t_phase, MORPH_DURATION_MS, true);
            if t_phase >= Duration::from_millis(MORPH_DURATION_MS) {
                anim.t = 1.0; anim.phase = MorphPhase::SphereFull; anim.phase_started_at = now;
            }
        }
        MorphPhase::SphereFull => {
            anim.t = 1.0; anim.going_to_sphere = 1.0;
            if t_phase >= Duration::from_millis(SPHERE_FULL_HOLD_MS) {
                anim.phase = MorphPhase::ShrinkingToHero; anim.phase_started_at = now;
            }
        }
        MorphPhase::ShrinkingToHero => {
            anim.t = 1.0; anim.going_to_sphere = 1.0;
            anim.hero = solve_phase_progress(t_phase, SHRINK_DURATION_MS, true);
            if t_phase >= Duration::from_millis(SHRINK_DURATION_MS) {
                anim.hero = 1.0; anim.phase = MorphPhase::Hero; anim.phase_started_at = now;
            }
        }
        MorphPhase::Hero => { anim.t = 1.0; anim.going_to_sphere = 1.0; anim.hero = 1.0; }
        MorphPhase::GrowingFromHero => {
            anim.t = 1.0; anim.going_to_sphere = 1.0;
            anim.hero = solve_phase_progress(t_phase, GROW_DURATION_MS, false);
            if t_phase >= Duration::from_millis(GROW_DURATION_MS) {
                anim.hero = 0.0; anim.phase = MorphPhase::SphereFullReverse; anim.phase_started_at = now;
            }
        }
        MorphPhase::SphereFullReverse => {
            anim.t = 1.0; anim.going_to_sphere = 1.0; anim.hero = 0.0;
            if t_phase >= Duration::from_millis(SPHERE_FULL_HOLD_MS / 4) {
                anim.t = 0.0; anim.going_to_sphere = 0.0;
                anim.phase = MorphPhase::Unmorphing; anim.phase_started_at = now;
            }
        }
        MorphPhase::Unmorphing => {
            anim.going_to_sphere = 0.0;
            anim.t = solve_phase_progress(t_phase, UNMORPH_DURATION_MS, true);
            if t_phase >= Duration::from_millis(UNMORPH_DURATION_MS) {
                anim.t = 1.0; anim.phase = MorphPhase::Idle; anim.phase_started_at = now;
            }
        }
    }
}
