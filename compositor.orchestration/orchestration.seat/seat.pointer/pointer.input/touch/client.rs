//! `wl_touch` forwarding. When the first finger of a sequence lands on a client
//! surface (window or layer), the whole sequence is delivered to that client as
//! native multi-touch via smithay's `TouchHandle`. Compositor UI (iced) and empty
//! desktop are handled by the pointer-emulation / gesture paths instead.
use smithay::input::touch::{DownEvent, MotionEvent, UpEvent};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_surface_interface_base::hit::{SurfaceHit, surface_under_filtered};

/// The client focus (surface + its origin in world space) under a world point, or
/// `None` if the *topmost* thing there is compositor UI (iced) or empty desktop.
/// Decided on the topmost hit — a client below an iced overlay does not count, so
/// touching e.g. the menu bar stays pointer-emulated rather than hitting a window.
pub fn focus_at(_loop: &mut Loop, world: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
    match surface_under_filtered(_loop, world, &|_| true) {
        Some(SurfaceHit::Window { surface, position, .. }) => Some((surface, position)),
        Some(SurfaceHit::Layer { surface, position_space, .. }) => Some((surface, position_space)),
        _ => None,
    }
}

/// Is there a client surface (window/layer) under this world point?
pub fn is_client(_loop: &mut Loop, world: Point<f64, Logical>) -> bool {
    focus_at(_loop, world).is_some()
}

fn slot(id: i32) -> smithay::backend::input::TouchSlot {
    if id >= 0 { Some(id as u32) } else { None }.into()
}

pub fn down(_loop: &mut Loop, id: i32, world: Point<f64, Logical>, time: u32) {
    let focus = focus_at(_loop, world);
    let Some(touch) = _loop.state.seat.seat.get_touch() else { return };
    touch.down(
        &mut _loop.state,
        focus,
        &DownEvent { slot: slot(id), location: world, serial: SERIAL_COUNTER.next_serial(), time },
    );
}

pub fn motion(_loop: &mut Loop, id: i32, world: Point<f64, Logical>, time: u32) {
    // Focus is only used for DnD hit-testing during motion; keep it current.
    let focus = focus_at(_loop, world);
    let Some(touch) = _loop.state.seat.seat.get_touch() else { return };
    touch.motion(&mut _loop.state, focus, &MotionEvent { slot: slot(id), location: world, time });
}

pub fn up(_loop: &mut Loop, id: i32, time: u32) {
    let Some(touch) = _loop.state.seat.seat.get_touch() else { return };
    touch.up(&mut _loop.state, &UpEvent { slot: slot(id), serial: SERIAL_COUNTER.next_serial(), time });
}

/// End of an input batch — flush queued down/motion/up to the client.
pub fn frame(_loop: &mut Loop) {
    if let Some(touch) = _loop.state.seat.seat.get_touch() {
        touch.frame(&mut _loop.state);
    }
}

/// Abort the whole touch stream (the compositor claimed it as a gesture).
pub fn cancel(_loop: &mut Loop) {
    if let Some(touch) = _loop.state.seat.seat.get_touch() {
        touch.cancel(&mut _loop.state);
    }
}
