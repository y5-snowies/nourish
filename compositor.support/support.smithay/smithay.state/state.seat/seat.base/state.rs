use smithay::backend::session::libseat::LibSeatSession;
use smithay::reexports::input::Device as InputDevice;
use smithay::input::pointer::{CursorImageStatus, CursorIcon, PointerHandle};
use smithay::input::{SeatHandler, SeatState};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::pointer_constraints::{PointerConstraintsState, with_pointer_constraint};
use smithay::wayland::pointer_gestures::PointerGesturesState;
use smithay::wayland::relative_pointer::RelativePointerManagerState;
use smithay::wayland::seat::WaylandFocus;

pub struct Seat<Handler: SeatHandler> {
    pub state: SeatState<Handler>,
    pub seat: smithay::input::Seat<Handler>,
    pub pointer_status: CursorImageStatus,
    pub relative_pointer_manager_state: RelativePointerManagerState,
    pub pointer_constraints_state: PointerConstraintsState,
    /// `zwp_pointer_gestures_v1` global — lets the focused window client receive
    /// native touchpad pinch/swipe gestures (the canvas zoom/pan repurposes them
    /// otherwise). Held to keep the global alive for the session.
    pub pointer_gestures_state: PointerGesturesState,
    /// Compositor-forced cursor icon that overrides client `set_cursor` while set.
    /// Used by the canvas hand tool to keep the grab cursor showing even when the
    /// pointer is over a window that sets its own cursor. `None` = clients win.
    pub force_cursor: Option<CursorIcon>,
    pub unlock_restoration_location: Option<(WlSurface, Point<f64, Logical>)>,
    pub previous_focus: Option<WlSurface>,
    pub libseat: Option<LibSeatSession>,
    /// Physical (libinput) keyboard devices, tracked on DeviceAdded/Removed so
    /// `led_state_changed` can mirror the xkb NumLock/CapsLock/ScrollLock state onto
    /// their hardware LEDs. Only populated on the udev backend (winit has no LEDs).
    pub keyboards: Vec<InputDevice>,
    /// Connected touch devices for the settings Display tab's claim list (udev only).
    pub touch_devices: Vec<InputDevice>,
}

impl<I> Seat<I>
where
    I: SeatHandler + 'static,
    I::KeyboardFocus: WaylandFocus + 'static,
    I: SeatHandler<KeyboardFocus = WlSurface, PointerFocus = WlSurface, TouchFocus = WlSurface>,
{
    pub fn is_keyboard_focused(&self, surface: &WlSurface) -> bool {
        let Some(kb) = self.seat.get_keyboard() else { return false; };
        let Some(kb) = kb.current_focus() else { return false; };
        let Some(kb_surface) = kb.wl_surface() else { return false; };
        kb_surface.as_ref() == surface
    }

    // The three pointer-constraint helpers that used to live here moved onto
    // `Dispatch` (state.base). Upstream smithay's `PointerConstraintRef::deactivate`
    // now takes `&mut D` — it calls `PointerConstraintsHandler::remove_constraint` —
    // and a `Seat<I>` held INSIDE `D` cannot hand out a borrow of its own container.
    // `unlock_restoration_location` below is still the state they read and write.

    pub fn is_pointer_over(&self, pointer: &PointerHandle<I>, surface: &WlSurface) -> bool {
        let Some(pointer) = pointer.current_focus() else { return false; };
        let Some(pointer) = pointer.wl_surface() else { return false; };
        pointer.as_ref() == surface
    }
}
