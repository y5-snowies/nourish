//! `wl_touch` forwarding. When the first finger of a sequence lands on a client
//! surface (window or layer), the whole sequence is delivered to that client as
//! native multi-touch via smithay's `TouchHandle`. Compositor UI (iced) and empty
//! desktop are handled by the pointer-emulation / gesture paths instead.
use smithay::input::touch::{DownEvent, MotionEvent, UpEvent};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_surface_interface_base::hit::{SurfaceHit, surface_under_filtered};

/// Topmost hit of any kind under a world point.
fn topmost(_loop: &mut Loop, world: Point<f64, Logical>) -> Option<SurfaceHit> {
    surface_under_filtered(_loop, world, &|_| true)
}

/// `wl_touch` focus (surface + its origin in world space) for a hit, or `None`
/// for iced/empty (those are handled by pointer emulation, not touch forwarding).
fn focus_of(hit: &SurfaceHit) -> Option<(WlSurface, Point<f64, Logical>)> {
    match hit {
        SurfaceHit::Window { surface, position, .. } => Some((surface.clone(), *position)),
        SurfaceHit::Layer { surface, position_space, .. } => Some((surface.clone(), *position_space)),
        _ => None,
    }
}

/// Should this touch sequence be delivered as native `wl_touch`? Only when the
/// topmost thing under the point is a client surface (window/layer, not an iced
/// overlay) AND that client actually bound `wl_touch` — e.g. Blender or a
/// touch-aware Chrome. Every other surface returns `false` and falls through to
/// pointer emulation, so ordinary apps behave exactly like they do under a
/// trackpad (and multi-finger gestures over them still reach the canvas).
pub fn is_client(_loop: &mut Loop, world: Point<f64, Logical>) -> bool {
    let Some((surface, _)) = topmost(_loop, world).as_ref().and_then(focus_of) else {
        return false;
    };
    _loop
        .state
        .seat
        .seat
        .get_touch()
        .map(|t| t.client_has_touch(&surface))
        .unwrap_or(false)
}

/// Is the topmost thing under this world point a window or layer surface? Used to
/// split single-finger touch: over a window/layer → click/drag; otherwise (empty
/// canvas / passthrough) → glide-pan.
pub fn over_window(_loop: &mut Loop, world: Point<f64, Logical>) -> bool {
    // `window()` rather than the `Window` variant, so a touch on the letterbox
    // bars counts as over the window and gets click/drag. It is the window's own
    // opaque pixels; glide-panning the canvas through them would be as wrong as
    // panning through its content. `focus_of` above still returns `None` there,
    // so the client is forwarded nothing either way.
    topmost(_loop, world).is_some_and(|h| h.window().is_some() || h.is_layer())
}

/// Is the topmost thing under this world point a compositor iced surface (the
/// touch pane, overview menu bar, selection toolbar, …)? Such UI must receive a
/// normal pointer tap in EVERY tool-mode — otherwise, e.g., the pane could not be
/// tapped to leave Hand mode (whose canvas taps are otherwise inert).
pub fn over_iced(_loop: &mut Loop, world: Point<f64, Logical>) -> bool {
    topmost(_loop, world).map(|h| h.is_iced()).unwrap_or(false)
}

/// Is the topmost thing under this point SCREEN-space compositor UI (the touch
/// pane, a docked toolbar)? Distinct from [`over_iced`], which also matches
/// WORLD-space iced (placeholders, group surfaces). Hand mode pans over world-space
/// content but must let a tap reach the SCREEN-space pane (to switch modes).
pub fn over_screen_iced(_loop: &mut Loop, world: Point<f64, Logical>) -> bool {
    matches!(
        topmost(_loop, world).and_then(|h| h.iced_space()),
        Some(compositor_monitor_compositor_iced_base::IcedSpace::Screen)
    )
}

fn slot(id: i32) -> smithay::backend::input::TouchSlot {
    if id >= 0 { Some(id as u32) } else { None }.into()
}

pub fn down(_loop: &mut Loop, id: i32, world: Point<f64, Logical>, time: u32) {
    let Some(hit) = topmost(_loop, world) else { return };
    let serial = SERIAL_COUNTER.next_serial();
    // A tap raises + activates + keyboard-focuses the surface, exactly like a
    // click — `TouchHandle::down` alone only sets *touch* focus.
    let keyboard = _loop.state.seat.seat.get_keyboard().unwrap();
    crate::native_press::press::apply_focus(_loop, &hit, &keyboard, serial);
    let focus = focus_of(&hit);
    let Some(touch) = _loop.state.seat.seat.get_touch() else { return };
    touch.down(&mut _loop.state, focus, &DownEvent { slot: slot(id), location: world, serial, time });
}

pub fn motion(_loop: &mut Loop, id: i32, world: Point<f64, Logical>, time: u32) {
    // Focus is only used for DnD hit-testing during motion; keep it current.
    let focus = topmost(_loop, world).as_ref().and_then(focus_of);
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
