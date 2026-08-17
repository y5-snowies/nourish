use smithay::input::Seat;
use smithay::input::pointer::{Focus, PointerHandle};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_seat;
use smithay::utils::{Rectangle, Serial};
use smithay::wayland::shell::xdg::ToplevelSurface;
use compositor_support_smithay_dispatch_state_base::state::{Dispatch, DispatchWire};
use compositor_support_smithay_state_grab_base::movement::state::GrabMovement;
use compositor_support_smithay_state_grab_base::resize::state::GrabResize;
use compositor_support_smithay_state_grab_dispatch::dispatch::check_grab;

pub fn move_request_prepare<WireObject: DispatchWire>(
    space: &compositor_support_smithay_state_space_base::state::SpaceState,
    surface: ToplevelSurface,
    seat: wl_seat::WlSeat,
    serial: Serial,
) -> Option<(PointerHandle<WireObject>, GrabMovement<WireObject>, Serial, Focus)> {
    let seat = Seat::from_resource(&seat).unwrap();
    let wl_surface = surface.wl_surface();
    if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
        let pointer = seat.get_pointer().unwrap();
        let window = space
            .state
            .elements()
            .find(|w| w.toplevel().unwrap().wl_surface() == wl_surface)
            .unwrap()
            .clone();
        let initial_window_location = space.state.element_location(&window).unwrap();
        let grab = GrabMovement { start_data, window, initial_window_location };
        return Some((pointer, grab, serial, Focus::Clear));
    }
    return None;
}

pub fn move_request_bind<WireObject: DispatchWire>(
    wire: &mut WireObject,
    prepare: (PointerHandle<WireObject>, GrabMovement<WireObject>, Serial, Focus),
) {
    let (pointer, grab, serial, focus) = prepare;
    pointer.set_grab(wire, grab, serial, focus);
}

pub fn resize_request_prepare<WireObject: DispatchWire>(
    space: &compositor_support_smithay_state_space_base::state::SpaceState,
    surface: ToplevelSurface,
    seat: wl_seat::WlSeat,
    serial: Serial,
    edges: xdg_toplevel::ResizeEdge,
) -> Option<(PointerHandle<WireObject>, GrabResize<WireObject>, Serial, Focus)> {
    let seat = Seat::from_resource(&seat).unwrap();
    let wl_surface = surface.wl_surface();
    if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
        let pointer = seat.get_pointer().unwrap();
        let window = space
            .state
            .elements()
            .find(|w| w.toplevel().unwrap().wl_surface() == wl_surface)
            .unwrap()
            .clone();
        let initial_window_location = space.state.element_location(&window).unwrap();
        let initial_window_size = window.geometry().size;
        surface.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Resizing);
        });
        surface.send_pending_configure();
        let grab = GrabResize::start(
            start_data,
            window,
            edges.into(),
            Rectangle::new(initial_window_location, initial_window_size),
        );
        return Some((pointer, grab, serial, Focus::Clear));
    }
    return None;
}

pub fn resize_request_bind<WireObject: DispatchWire>(
    wire: &mut WireObject,
    result: (PointerHandle<WireObject>, GrabResize<WireObject>, Serial, Focus),
) {
    let (pointer, grab, serial, focus) = result;
    pointer.set_grab(wire, grab, serial, focus);
}
