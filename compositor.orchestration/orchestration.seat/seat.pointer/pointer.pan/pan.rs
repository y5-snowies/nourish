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

    // A press-driven pan (mouse drag / emulated touch press on empty canvas) advances
    // the camera ONLY on pointer motion, and every motion event computes its world
    // point BEFORE the CAM_BUF pan step flushes — so at frame end the seat's world
    // location is one pan step stale against the just-panned camera. Reconciling
    // against that stale projection re-seats both accumulators shifted by the frame's
    // pan delta, compounding each frame into runaway cursor/camera speed on a plain
    // mouse pan. The motion path maintains both accumulators itself during a press
    // pan; the glide paths (no press, no motion events) still reconcile below.
    if state.inner.canvas().position_updating {
        return;
    }

    // Where the cursor is actually drawn: its world location projected through the
    // live (possibly panned) camera — the same pane context the cursor renders in.
    //
    // That identity holds only while the pointer's world location is a pure
    // projection of the hardware position. A background bundle with a pointer
    // warp breaks it: `current_location()` is CORRECTED, so projecting it back
    // gives the displaced screen point, not the hardware one. Writing that into
    // the accumulator makes the next event's input the previous event's output —
    // the correction is then applied to its own result, compounding every event
    // until the cursor pins itself in a corner. That is not a warp being too
    // strong; it is a feedback loop, and it runs away in about three events.
    //
    // Under a warp the hardware position is authoritative and a camera glide does
    // not move it, so the accumulator is left alone and only the camera's pan
    // accumulator is re-seated below.
    let warped = compositor_orchestration_seat_pointer_warp::warp::applies(state);
    let ctx = state.focus_pane_context();
    let phys: Point<f64, Physical> = if warped {
        let m = state.inner.pointer_mut().motion;
        Point::from((m.x, m.y))
    } else {
        let world = state.state.seat.seat.get_pointer().unwrap().current_location();
        let t: Transform = (world, ctx).into();
        t.into()
    };

    {
        let motion = &mut state.inner.pointer_mut().motion;
        // Already consistent (no glide has pulled them apart) → nothing to do.
        // Skipped under a warp: `phys` IS the accumulator there, so this would
        // always return early and leave `position_previous` stale.
        if !warped && (motion.x - phys.x).abs() < 1e-6 && (motion.y - phys.y).abs() < 1e-6 {
            return;
        }
        motion.x = phys.x;
        motion.y = phys.y;
    }

    // The canvas-PAN accumulator is the SAME screen point, held separately on the
    // camera and advanced ONLY by `CamCmd::Pan` (i.e. only on pointer motion). A
    // glide therefore leaves it stale by exactly the pan, and the first press-drag
    // afterwards computes `screen - position_previous` and flings the camera by the
    // whole glide. Re-seat it together with the pointer accumulator — `wire.rs`
    // pairs the same two writes when it warps the pointer.
    state.inner.camera_mut().position_previous = Point::from((phys.x, phys.y));
}
