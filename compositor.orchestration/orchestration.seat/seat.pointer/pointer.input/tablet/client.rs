//! Pen hit-testing — the SAME `surface_under_filtered` the pointer/touch paths use,
//! so canvas-vs-client classification stays consistent across all input types.

use smithay::input::keyboard::KeyboardHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use compositor_orchestration_core_state_base::Loop;
use compositor_support_smithay_dispatch_state_base::state::Dispatch;
use compositor_y5_surface_interface_base::hit::{surface_under_filtered, SurfaceHit};

/// Topmost hit of any kind under a world point.
pub fn topmost(_loop: &mut Loop, world: Point<f64, Logical>) -> Option<SurfaceHit> {
    surface_under_filtered(_loop, world, &|_| true)
}

/// Surface + its world origin for a window/layer hit (iced/empty → `None`, handled
/// by the canvas path, exactly like touch's `focus_of`).
pub fn focus_of(hit: &SurfaceHit) -> Option<(WlSurface, Point<f64, Logical>)> {
    match hit {
        SurfaceHit::Window { surface, position, .. } => Some((surface.clone(), *position)),
        SurfaceHit::Layer { surface, position_space, .. } => Some((surface.clone(), *position_space)),
        _ => None,
    }
}

/// The window under the pen IF its client bound the tablet protocol — the tablet
/// analog of touch's `is_client` (`client_has_touch`). `None` ⇒ forward nothing.
pub fn tablet_focus(_loop: &mut Loop, world: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
    let focus = topmost(_loop, world).as_ref().and_then(focus_of)?;
    if _loop.state.tablet.client_has_tablet(&focus.0) {
        Some(focus)
    } else {
        None
    }
}

/// Raise + activate + keyboard-focus the surface under the pen on tip-down, exactly
/// like a click / touch-tap (reuses the pointer path's `apply_focus`).
pub fn apply_focus(_loop: &mut Loop, world: Point<f64, Logical>) {
    let Some(hit) = topmost(_loop, world) else { return };
    let serial = SERIAL_COUNTER.next_serial();
    let keyboard: KeyboardHandle<Dispatch> = _loop.state.seat.seat.get_keyboard().unwrap();
    crate::native_press::press::apply_focus(_loop, &hit, &keyboard, serial);
}
