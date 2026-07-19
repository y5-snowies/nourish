//! Absorb the "jump" the first pointer motion makes after a two-finger glide.
//!
//! Relative motion accumulates onto the physical cursor accumulator
//! (`pointer().motion`) — the on-screen start point the next delta builds on. A
//! two-finger glide pans the camera WITHOUT routing through that accumulator, so
//! it is left pointing at the pre-pan position while the cursor is actually drawn
//! at its (unchanged) world location projected through the now-panned camera. The
//! first post-glide motion then accumulates from the stale point and snaps the
//! cursor across the whole pan.
//!
//! The world location is intentionally left untouched (the cursor stays pinned to
//! its world point during the glide — the "pan indefinitely" behaviour). We only
//! re-seat the accumulator onto the cursor's current on-screen position — its
//! world location projected through the live camera, i.e. exactly where the cursor
//! is drawn — so the next motion continues seamlessly. A no-op in steady state,
//! where the accumulator and the projected world location already agree; a
//! finger-glide is the only thing that pulls them apart.

use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_camera_transform_translate::transform::Transform;
use smithay::utils::{Physical, Point};

pub fn reconcile_finger_pan(state: &mut Loop) {
    // `hooks()` runs once PER OUTPUT inside the render loop, each pass with
    // `render_output` set to the output being drawn — and `focus_pane_context()`
    // resolves against `render_output`. Only reconcile during the pass for the
    // output the cursor is actually on; otherwise (multi-monitor) we would project
    // the cursor through another monitor's camera and clobber the accumulator with
    // a foreign-space point every frame, freezing pointer motion. When
    // `render_output` is unset (single-output paths / off the render loop) it
    // already resolves the cursor's output, so proceed.
    if state.inner.render_output.is_some()
        && state.inner.render_output != state.inner.cursor_output
    {
        return;
    }

    // Where the cursor is actually drawn: its world location projected through the
    // live (possibly panned) camera — the same pane context the cursor renders in.
    let world = state.state.seat.seat.get_pointer().unwrap().current_location();
    let ctx = state.focus_pane_context();
    let phys: Point<f64, Physical> = {
        let t: Transform = (world, ctx).into();
        t.into()
    };

    let motion = &mut state.inner.pointer_mut().motion;
    // Already consistent (no glide has pulled them apart) → nothing to do.
    if (motion.x - phys.x).abs() < 1e-6 && (motion.y - phys.y).abs() < 1e-6 {
        return;
    }
    motion.x = phys.x;
    motion.y = phys.y;
}
