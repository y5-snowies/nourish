use smithay::reexports::wayland_protocols::xwayland::shell::v1::server::{
    xwayland_shell_v1::XwaylandShellV1, xwayland_surface_v1::XwaylandSurfaceV1,
};
use smithay::reexports::wayland_server::{Dispatch, DisplayHandle, GlobalDispatch};
use smithay::wayland::GlobalData;
use smithay::wayland::xwayland_shell::{XWaylandShellState, XWaylandSurfaceUserData};
use compositor_support_smithay_dispatch_state_base::state::DispatchWire;
use compositor_support_smithay_state_xwayland_base::base::Xwayland;

/// Advertise `xwayland_shell_v1` and seed the empty XWayland state.
///
/// The global is created unconditionally, before the Xwayland process exists and
/// even if it never starts: smithay filters it to clients carrying
/// `XWaylandClientData`, so no ordinary client can see it, and Xwayland itself binds
/// it during its very first roundtrip — after which it is too late to create.
pub fn new<I: DispatchWire>(display_handle: &DisplayHandle) -> Xwayland
where
    I: GlobalDispatch<XwaylandShellV1, GlobalData>
        + Dispatch<XwaylandShellV1, GlobalData>
        + Dispatch<XwaylandSurfaceV1, XWaylandSurfaceUserData>
        + 'static,
{
    Xwayland {
        shell: XWaylandShellState::new::<I>(display_handle),
        xwm: None,
        client: None,
        map_position_parent_hover: None,
        map_position_parent_focus: None,
        disconnected: false,
    }
}
