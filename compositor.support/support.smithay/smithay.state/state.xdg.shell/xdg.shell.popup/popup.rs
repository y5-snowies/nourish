use smithay::desktop::PopupKind;
use smithay::reexports::wayland_server::protocol::wl_seat;
use smithay::utils::Serial;
use smithay::wayland::shell::xdg::{PopupSurface, PositionerState};
use compositor_support_smithay_dispatch_state_base::state::{Dispatch, DispatchWire};

pub fn new_popup(
    dispatch: &mut Dispatch,
    surface: PopupSurface,
    _positioner: PositionerState,
) {
    compositor_support_smithay_state_popup_dispatch::dispatch::unconstrain_popup(dispatch, &surface);
    let _ = dispatch.popup.state.track_popup(PopupKind::Xdg(surface));
}

pub fn reposition_request(
    dispatch: &mut Dispatch,
    surface: PopupSurface,
    positioner: PositionerState,
    token: u32,
) {
    surface.with_pending_state(|state| {
        let geometry = positioner.get_geometry();
        state.geometry = geometry;
        state.positioner = positioner;
    });
    compositor_support_smithay_state_popup_dispatch::dispatch::unconstrain_popup(dispatch, &surface);
    surface.send_repositioned(token);
}

pub fn grab(
    _dispatch: &mut Dispatch,
    _surface: PopupSurface,
    _seat: wl_seat::WlSeat,
    _serial: Serial,
) {
    // TODO: Handle popup grabs.
}
