//! `xdg_toplevel_icon_v1` — the client-declared toplevel icon.
//!
//! A client either names an icon from the XDG icon stock or hands over pixel
//! buffers. y5 consumes the NAME half: it is the only form that survives into
//! the introspection sample (a background thread that never touches wayland
//! objects), and it resolves through the same icon-theme lookup the desktop
//! entry already goes through.
//!
//! The global is advertised unconditionally — a client sets its icon long
//! before anything asks for one, and the icon is read straight off the
//! surface's double-buffered `ToplevelIconCachedState` when it is.
//!
//! smithay owns the whole `Dispatch2`/`GlobalDispatch2` surface for this
//! protocol, so unlike `wire.tearing` there is nothing to hand-roll here — only
//! the global, and `impl XdgToplevelIconHandler for Dispatch` in
//! `dispatch.state/state.base` (orphan rule).

use smithay::reexports::wayland_protocols::xdg::toplevel_icon::v1::server::xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1;
use smithay::reexports::wayland_server::{DisplayHandle, GlobalDispatch};
use smithay::wayland::xdg_toplevel_icon::{XdgToplevelIconHandler, XdgToplevelIconManager, XdgToplevelIconManagerUserData};

/// Icon edge sizes advertised to clients at bind time, mirroring the sizes the
/// icon-theme lookup actually searches for.
const ICON_SIZES: &[i32] = &[16, 24, 32, 48, 64, 128, 256];

/// Advertise the `xdg_toplevel_icon_manager_v1` global. Mirrors `wire.tearing::create_global`.
///
/// The returned manager is dropped: it owns nothing but a `GlobalId` and an
/// `Arc` the bind handler already holds a clone of, so the advertised sizes
/// stay live and the global is never revoked.
pub fn create_global<D>(dh: &DisplayHandle)
where
    D: XdgToplevelIconHandler + GlobalDispatch<XdgToplevelIconManagerV1, XdgToplevelIconManagerUserData>,
{
    let mut manager = XdgToplevelIconManager::new::<D>(dh);
    manager.replace_icon_sizes(ICON_SIZES.iter().copied());
    info!("icon: xdg_toplevel_icon_manager_v1 global advertised");
}
