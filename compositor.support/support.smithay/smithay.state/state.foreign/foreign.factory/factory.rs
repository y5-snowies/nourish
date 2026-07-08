//! Create the foreign-toplevel state. `enabled` (the `protocol_foreign` preference,
//! a startup snapshot) decides whether either global is advertised at all: when
//! disabled, neither the wlr manager nor the ext list is offered in the registry.

use smithay::reexports::wayland_protocols::ext::foreign_toplevel_list::v1::server::{
    ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1,
};
use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1;
use smithay::reexports::wayland_server::{Dispatch, DisplayHandle, GlobalDispatch};
use smithay::wayland::foreign_toplevel_list::{
    ForeignToplevelHandle, ForeignToplevelListGlobalData, ForeignToplevelListHandler,
};
use compositor_support_smithay_dispatch_state_base::state::DispatchWire;
use compositor_support_smithay_state_foreign_base::base::{ForeignManagerGlobalData, ForeignToplevel};

pub fn new<I>(display_handle: &DisplayHandle, enabled: bool) -> ForeignToplevel
where
    I: DispatchWire
        + GlobalDispatch<ZwlrForeignToplevelManagerV1, ForeignManagerGlobalData>
        + GlobalDispatch<ExtForeignToplevelListV1, ForeignToplevelListGlobalData>
        + ForeignToplevelListHandler
        + Dispatch<ExtForeignToplevelHandleV1, ForeignToplevelHandle>
        + 'static,
{
    ForeignToplevel::new::<I>(display_handle, enabled)
}
