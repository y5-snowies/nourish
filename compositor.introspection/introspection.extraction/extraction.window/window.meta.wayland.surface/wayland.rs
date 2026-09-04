use smithay::desktop::Window;
use compositor_support_smithay_state_window_ident::ident;

/// Read app_id / title / target wl_surface from a window's shell role.
///
/// X11 answers the same two questions through `WM_CLASS` and `WM_NAME`, and a dock,
/// the launcher matcher and the icon inference all read the identical two fields —
/// so `window.ident` puts the X11 `class` in `app_id` and `WM_NAME` in `title`, with
/// no third vocabulary introduced for callers to learn.
///
/// The surface may be `None` for an X11 window until Xwayland associates one; every
/// caller already tolerates that (a window with no surface identity yet).
pub fn read_surface_identity(
    window: &Window,
) -> (
    Option<String>,
    Option<String>,
    Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
) {
    let names = ident::names(window);
    (names.app_id, names.title, ident::surface(window))
}
