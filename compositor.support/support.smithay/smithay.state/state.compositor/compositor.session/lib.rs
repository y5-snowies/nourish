use smithay::wayland::compositor::{CompositorClientState, CompositorState};
use smithay::reexports::wayland_server::Client;
use smithay::backend::renderer::utils::on_commit_buffer_handler;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{get_parent, is_sync_subsurface, with_states};
use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;
use smithay::desktop::{Space, Window};
use smithay::utils::{Logical, Rectangle};
use compositor_support_smithay_dispatch_state_base::state::{Dispatch, DispatchWire};
use compositor_support_smithay_wayland_connection_record::record::WaylandClientSession;
use compositor_support_smithay_state_compositor_place::WindowPlacedMarker;

pub fn compositor_state(
    dispatch: &mut Dispatch,
) -> &mut CompositorState {
    &mut dispatch.compositor.state
}

pub fn client_compositor_state<'a>(
    dispatch: &Dispatch,
    client: &'a Client,
) -> &'a CompositorClientState {
    &client
        .get_data::<WaylandClientSession>()
        .unwrap()
        .compositor_state
}

/// PROTOCOL-only commit (wayland `D` path): buffer state + popups, then record the surface in the
/// commit outbox. World effects run at drain via `apply_commit` (document/SMITHAY_DECOUPLING.md).
pub fn commit(
    dispatch: &mut Dispatch,
    surface: &WlSurface,
) {
    on_commit_buffer_handler::<Dispatch>(surface);
    compositor_support_smithay_state_compositor_place::handle_commit(dispatch, surface);
    dispatch.committed.push(surface.clone());
}

/// World side of a commit (run by orchestration against the active Space at drain). Returns the
/// window that just became ready for its initial map, if any — the caller performs the map.
pub fn apply_commit(
    space: &mut Space<Window>,
    surface: &WlSurface,
) -> Option<(Window, Rectangle<i32, Logical>)> {
    // Root-window on_commit (mirror smithay's recommended commit handling).
    if !is_sync_subsurface(surface) {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        if let Some(window) = space
            .elements()
            .find(|w| w.toplevel().unwrap().wl_surface() == &root)
        {
            window.on_commit();
        }
    }

    // Directly-committed toplevel: startup-grace jiggle, then initial configure + placement.
    let mut to_place = None;
    if let Some(window) = space
        .elements()
        .find(|w| w.toplevel().unwrap().wl_surface() == surface)
        .cloned()
    {
        // Startup-grace jiggle back toward the decided size (see `reassert_size_if_diverged`).
        if let Some(size) = compositor_support_smithay_state_compositor_place::reassert_size_if_diverged(&window) {
            let tl = window.toplevel().unwrap();
            tl.with_pending_state(|s| s.size = Some(size));
            tl.send_configure();
        }

        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .initial_configure_sent
        });
        if !initial_configure_sent {
            window.toplevel().unwrap().send_configure();
        }

        let geometry = window.geometry();
        let is_ready_to_place = geometry.size.w > 0 && geometry.size.h > 0;
        let has_been_placed = window.user_data().get::<WindowPlacedMarker>().is_some();
        if is_ready_to_place && !has_been_placed {
            window.user_data().insert_if_missing(|| WindowPlacedMarker);
            to_place = Some((window, geometry));
        }
    }

    compositor_support_smithay_state_grab_base::resize::dispatch::handle_commit(space, surface);
    to_place
}
