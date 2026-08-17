use smithay::wayland::shell::xdg::PopupSurface;
use compositor_support_smithay_dispatch_state_base::state::{Dispatch, DispatchWire};

pub fn unconstrain_popup(_dispatch: &Dispatch, popup: &PopupSurface) {
    let infinite_target = smithay::utils::Rectangle::from_loc_and_size(
        (-100_000, -100_000),
        (200_000, 200_000)
    );

    popup.with_pending_state(|state| {
        state.geometry = state.positioner.get_unconstrained_geometry(infinite_target);
    });
}
