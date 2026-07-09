use smithay::input::pointer::CursorImageStatus;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_protocols::wp::pointer_constraints::zv1::server::zwp_confined_pointer_v1::ZwpConfinedPointerV1;
use smithay::reexports::wayland_protocols::wp::pointer_constraints::zv1::server::zwp_locked_pointer_v1::ZwpLockedPointerV1;
use smithay::reexports::wayland_protocols::wp::pointer_constraints::zv1::server::zwp_pointer_constraints_v1::ZwpPointerConstraintsV1;
use smithay::reexports::wayland_protocols::wp::pointer_gestures::zv1::server::zwp_pointer_gesture_hold_v1::ZwpPointerGestureHoldV1;
use smithay::reexports::wayland_protocols::wp::pointer_gestures::zv1::server::zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1;
use smithay::reexports::wayland_protocols::wp::pointer_gestures::zv1::server::zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1;
use smithay::reexports::wayland_protocols::wp::pointer_gestures::zv1::server::zwp_pointer_gestures_v1::ZwpPointerGesturesV1;
use smithay::reexports::wayland_protocols::wp::relative_pointer::zv1::server::zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1;
use smithay::reexports::wayland_protocols::wp::relative_pointer::zv1::server::zwp_relative_pointer_v1::ZwpRelativePointerV1;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::{Dispatch, DisplayHandle, GlobalDispatch};
use smithay::wayland::GlobalData;
use smithay::wayland::pointer_constraints::{PointerConstraintsState, PointerConstraintUserData};
use smithay::wayland::pointer_gestures::{PointerGesturesState, PointerGestureUserData};
use smithay::wayland::relative_pointer::{RelativePointerManagerState, RelativePointerUserData};
use smithay::wayland::seat::{SeatGlobalData, WaylandFocus};
use compositor_support_smithay_dispatch_state_base::state::DispatchWire;

pub fn new<I: DispatchWire>(
    display_handle: &DisplayHandle,
) -> compositor_support_smithay_state_seat_base::state::Seat<I>
where
    I: GlobalDispatch<ZwpRelativePointerManagerV1, GlobalData>,
    I: Dispatch<ZwpRelativePointerManagerV1, GlobalData>,
    I: Dispatch<ZwpRelativePointerV1, RelativePointerUserData<I>>,
    I: GlobalDispatch<WlSeat, SeatGlobalData<I>> + SeatHandler + 'static,
    I: GlobalDispatch<ZwpPointerConstraintsV1, GlobalData>,
    I: Dispatch<ZwpPointerConstraintsV1, GlobalData>,
    I: Dispatch<ZwpConfinedPointerV1, PointerConstraintUserData<I>>,
    I: Dispatch<ZwpLockedPointerV1, PointerConstraintUserData<I>>,
    I: GlobalDispatch<ZwpPointerGesturesV1, GlobalData>,
    I: Dispatch<ZwpPointerGesturesV1, GlobalData>,
    I: Dispatch<ZwpPointerGestureSwipeV1, PointerGestureUserData<I>>,
    I: Dispatch<ZwpPointerGesturePinchV1, PointerGestureUserData<I>>,
    I: Dispatch<ZwpPointerGestureHoldV1, PointerGestureUserData<I>>,
    <I as SeatHandler>::PointerFocus: WaylandFocus,
    <I as SeatHandler>::KeyboardFocus: WaylandFocus,
{
    // A seat is a group of keyboards, pointer, and touch devices.
    // It maintains keyboard focus (who gets keystrokes) and pointer focus (who gets mouse events).
    let mut seat_state = SeatState::new();

    // Creates the actual `wl_seat` global named "winit" (often renamed to "seat0" in real setups).
    let mut seat: Seat<I> = seat_state.new_wl_seat(&display_handle, "winit");

    // Notify clients that we have a keyboard.
    // **Calloop Interaction:** Later in your code, you will read raw key events from your backend
    // (like libinput or Winit) during a `calloop` tick. You will feed those events into
    // `seat.get_keyboard().unwrap().input(...)`. That method translates the raw scancodes into
    // Wayland events and flushes them to the currently focused client.
    //
    // Repeat rate: 200ms delay, 25/sec. The keymap comes from the saved layout
    // preference (see `seat.xkb`); the settings window hot-reloads it via `apply`.
    let layout = compositor_support_smithay_state_seat_xkb::xkb::load();
    let cfg = compositor_support_smithay_state_seat_xkb::xkb::config(&layout);
    let keyboard = seat.add_keyboard(cfg, 200, 25).unwrap();

    // Enable NumLock at startup. In Wayland the compositor owns the xkb state,
    // so this is our responsibility (no external `numlockx` equivalent). Set it
    // as a *locked* modifier so it persists until the user toggles NumLock; this
    // also recomputes the LED state, which is mirrored to physical keyboards via
    // `SeatHandler::led_state_changed` + the per-device update on DeviceAdded.
    let mut mods = keyboard.modifier_state();
    mods.num_lock = true;
    keyboard.set_modifier_state(mods);

    // Notify clients that we have a pointer (mouse).
    // Side-effect: Clients use this to render their own cursor icons. When you dispatch mouse
    // motion events through the seat, it triggers `enter`, `leave`, and `motion` events on the
    // client, causing them to draw hover states (like highlighting a button).
    seat.add_pointer();

    seat.add_touch(); // wl_touch: native multi-touch for client surfaces.

    let relative_pointer_manager_state = RelativePointerManagerState::new::<I>(&display_handle);

    let pointer_constraints_state = PointerConstraintsState::new::<I>(&display_handle);

    // Touchpad gesture protocol: clients that bind it receive native pinch/swipe
    // gestures when focused (the canvas otherwise repurposes them for zoom/pan).
    let pointer_gestures_state = PointerGesturesState::new::<I>(&display_handle);

    return compositor_support_smithay_state_seat_base::state::Seat {
        state: seat_state,
        seat: seat,
        pointer_status: CursorImageStatus::default_named(),
        relative_pointer_manager_state,
        pointer_constraints_state,
        pointer_gestures_state,
        force_cursor: None,
        unlock_restoration_location: None,
        previous_focus: None,
        libseat: None,
        keyboards: Vec::new(),
    };
}
