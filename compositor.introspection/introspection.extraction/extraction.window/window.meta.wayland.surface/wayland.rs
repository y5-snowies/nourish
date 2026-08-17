use smithay::desktop::Window;
use smithay::wayland::compositor;
use std::sync::Mutex;

/// Read app_id / title / target wl_surface from a window's toplevel role.
pub fn read_surface_identity(
    window: &Window,
) -> (
    Option<String>,
    Option<String>,
    Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
) {
    let mut app_id = None;
    let mut title = None;
    let mut target = None;

    if let Some(toplevel) = window.toplevel() {
        let wl_surface = toplevel.wl_surface();
        compositor::with_states(wl_surface, |states| {
            if let Some(role) = states
                .data_map
                .get::<Mutex<smithay::wayland::shell::xdg::XdgToplevelSurfaceRoleAttributes>>()
            {
                if let Ok(role_guard) = role.lock() {
                    title = role_guard.title.clone();
                    app_id = role_guard.app_id.clone();
                }
            }
        });
        target = Some(wl_surface.clone());
    }

    // NOTE: XWayland support omitted here; mirror this block with
    // window.x11_surface() if/when we support XWayland windows.

    (app_id, title, target)
}

