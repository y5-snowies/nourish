use smithay::utils::{Logical, Point};
use compositor_orchestration_core_state_base::Loop;

/// Fractional-scale (camera zoom) bounds — a pragmatic guard, not the root fix.
/// The zoom multiplies every element's destination geometry and damage
/// (`size * zoom`); left unbounded it grows roughly exponentially (the scroll
/// step is `0.05 * zoom`) and the scene can wedge at extreme zoom (the present
/// loop stops re-rendering — likely a damage-tracking interaction, since it
/// reproduces over a static window but not over the always-damaging background).
/// Until that's root-caused, clamp to a usable, overflow-safe range.
const MIN_ZOOM: f64 = 0.02;
const MAX_ZOOM: f64 = 50.0;

pub fn position(_loop: &mut Loop, position: Point<f64, Logical>) {
    _loop.inner.camera_mut().transform.position = position;
    // Focused world's parallax (spawn_target == the spatial world in view).
    let target = _loop.inner.worlds.spawn_target();
    if let Some(ref mut background) = _loop.inner.worlds.get_mut(target).storage_mut().get_mut(&compositor_background_two_storage_base::base::BG_TWO_MUT).instance {
        background.pan = (position.x as f32, position.y as f32);
    }
}
pub fn zoom(_loop: &mut Loop, zoom: f64) {
    // Guard against the extreme-zoom wedge (see MIN_ZOOM/MAX_ZOOM).
    let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    _loop.inner.camera_mut().transform.zoom = zoom;

    let target = _loop.inner.worlds.spawn_target();
    if let Some(ref mut background) = _loop.inner.worlds.get_mut(target).storage_mut().get_mut(&compositor_background_two_storage_base::base::BG_TWO_MUT).instance {
        background.zoom = zoom as f32;
    }
}
