use smithay::reexports::wayland_protocols::wp::fractional_scale::v1::server::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};

use smithay::reexports::wayland_server::{Dispatch, DisplayHandle, GlobalDispatch};
use smithay::wayland::GlobalData;
use smithay::wayland::fractional_scale::{
    FractionalScaleData, FractionalScaleHandler, FractionalScaleManagerState, FractionalScaleState,
};
use smithay::wayland::presentation::{PresentationData, PresentationState};
use compositor_support_smithay_dispatch_state_base::state::DispatchWire;
use compositor_support_smithay_state_fractional_base::state::{Fractional, FractionalScaleConfig};

pub fn new<I: DispatchWire>(display_handle: &DisplayHandle) -> Fractional
where
    I: GlobalDispatch<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, GlobalData>
        + Dispatch<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, GlobalData>
        + Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, FractionalScaleData>
        + 'static,
    I: FractionalScaleHandler + 'static {
    let mut fractional_manager_state = FractionalScaleManagerState::new::<I>(&display_handle);
    return Fractional {
        state: fractional_manager_state,
        cfg: FractionalScaleConfig::default(),
        last_observed: None,
        cycle: None,
        armed: false,
        last_emit_at: None,
        last_emitted_scale: None,
    };
}
