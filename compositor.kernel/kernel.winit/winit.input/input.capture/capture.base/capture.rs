//! Host pointer capture for the nested winit window.
//!
//! Nested winit otherwise reports only ABSOLUTE positions, clamped to the window,
//! which leaves the compositor's entire RELATIVE path unreachable in the dev loop:
//! cursor speed, monitor teleport, client pointer locks/confines and the edge pan
//! all hang off `motion::relative`. `Locked` is the grab mode that produces relative
//! deltas — on Wayland the host wires `zwp_relative_pointer` and they arrive as the
//! `DeviceEvent::PointerMotion` the vendored winit backend forwards (a y5 patch;
//! upstream drops it and leaves `PointerMotionEvent = UnusedEvent`).
//!
//! Locking also stops the host painting its own cursor over the one y5 draws.

use smithay::reexports::winit::window::{CursorGrabMode, Window};

/// Take the host pointer (`on`) or hand it back.
///
/// X11 hosts support only `Confined`, which keeps the cursor inside the window but
/// still reports absolute motion; fall back to it rather than failing outright,
/// since the absolute edge-band pan covers that case.
///
/// The caller releases on focus loss, which is the escape hatch: whatever moves
/// focus away (a host keybinding, the workspace switcher) hands the pointer back.
/// `COMPOSITOR_WINIT_NO_CAPTURE=1` opts out of capture entirely.
pub fn capture(window: &dyn Window, on: bool) {
    if std::env::var("COMPOSITOR_WINIT_NO_CAPTURE").is_ok() {
        return;
    }
    if on {
        if window.set_cursor_grab(CursorGrabMode::Locked).is_ok() {
            info!("winit: pointer LOCKED — relative motion active");
        } else if window.set_cursor_grab(CursorGrabMode::Confined).is_ok() {
            warn!("winit: pointer only CONFINED (host has no Locked) — motion stays absolute");
        } else {
            warn!("winit: pointer capture refused by the host — motion stays absolute");
            return;
        }
    } else if window.set_cursor_grab(CursorGrabMode::None).is_err() {
        return;
    }
    window.set_cursor_visible(!on);
}
