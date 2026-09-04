use smithay::wayland::compositor::CompositorState;
use smithay::backend::renderer::utils::on_commit_buffer_handler;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{get_parent, is_sync_subsurface, with_states};
use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;
use smithay::desktop::{Space, Window};
use smithay::utils::{Logical, Rectangle};
use compositor_support_smithay_dispatch_state_base::state::{Dispatch, DispatchWire};
use compositor_support_smithay_state_compositor_place::WindowPlacedMarker;
use compositor_support_smithay_state_window_find::find;
use compositor_support_smithay_state_window_shell::shell;

pub fn compositor_state(dispatch: &mut Dispatch) -> &mut CompositorState {
    &mut dispatch.compositor.state
}

/// PROTOCOL-only commit (wayland `D` path): buffer state + popups, then record the
/// surface in the commit outbox. World effects run at drain via `apply_commit`
/// (document/SMITHAY_DECOUPLING.md).
pub fn commit(dispatch: &mut Dispatch, surface: &WlSurface) {
    on_commit_buffer_handler::<Dispatch>(surface);
    compositor_support_smithay_state_compositor_place::handle_commit(dispatch, surface);
    dispatch.committed.push(surface.clone());
}

/// World side of a commit (run by orchestration against the active Space at drain).
/// Returns the window ready for its initial map, if any; the caller performs it.
/// `initial`: the caller's `awaits_initial_configure` answer — read once, since the
/// caller gates the `restore_size` lookup (`WireTrait::session_restore_size`) on it.
/// A toplevel's commit BEFORE its initial configure — the one at which a
/// remembered size can still be proposed. One flag read, so callers gate the
/// per-world placeholder lookup on it rather than paying it on every commit.
pub fn awaits_initial_configure(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        states.data_map.get::<XdgToplevelSurfaceData>().is_some_and(|d| !d.lock().unwrap().initial_configure_sent)
    })
}

/// `committed` = the mapped window `surface` IS (not one it is a subsurface of),
/// `initial` = `awaits_initial_configure`: both read once by the caller, which gates
/// the `restore_size` lookup on them.
///
/// `committed`, not `toplevel`: it is `Some` for an X11 window too, which has no xdg
/// toplevel at all — and pairing it with `initial`, which can only ever be true for
/// xdg, is the whole reason the two are separate arguments.
pub fn apply_commit(
    space: &mut Space<Window>,
    surface: &WlSurface,
    committed: Option<Window>,
    initial: bool,
    restore_size: Option<smithay::utils::Size<i32, Logical>>,
) -> Option<(Window, Rectangle<i32, Logical>)> {
    if !is_sync_subsurface(surface) {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        if let Some(window) = find::in_space(space, &root) {
            window.on_commit();
        }
    }

    // Directly-committed toplevel: jiggle, then initial configure + placement.
    let mut to_place = None;
    if let Some(window) = committed {
        // Startup-grace jiggle toward the decided size (`reassert_size_if_diverged`).
        if let Some(size) = compositor_support_smithay_state_compositor_place::reassert_size_if_diverged(&window) {
            shell::stage(&window, size, false);
            shell::send(&window);
        }

        // xdg-only handshake, and already so: `awaits_initial_configure` reads
        // `XdgToplevelSurfaceData`, which an X11 window does not carry.
        if initial {
            // Propose the remembered size when the client named which window this
            // is: without it the restore is `0x0` -> client default -> correction
            // after the first buffer, a visible two-step resize.
            if let Some(size) = restore_size {
                shell::stage(&window, size, false);
            }
            shell::send(&window);
        }

        let mut geometry = window.geometry();
        geometry.size = shell::configured_size(&window);
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
