use smithay::desktop::Window;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::utils::{Logical, Point, Size};
use compositor_y5_camera_transform_translate::slot;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_window_interface_record::data::WindowFullscreen;
use compositor_y5_window_interface_record::window::LoopWindow;
use compositor_support_smithay_state_window_find::find;
use compositor_support_smithay_state_window_shell::shell;

/// Apply (or clear) fullscreen on a window.
///
/// This compositor has no physical "screen" to fill (the canvas is a
/// pannable/zoomable y5-world), so "fullscreen" means: tell the client its
/// fullscreen size equals the region it conceptually owns.
///
///   * Ungrouped window  → its own current size (it is already "as large as
///     its screen"); we only flip the protocol state.
///   * Grouped window     → the group's padded bounding box, and we move the
///     window to fill that region.
///
/// The window is raised to the top of the stack so it stays above its peers
/// and captures input within its bounds. Pre-fullscreen geometry is stored so
/// it can be restored on un-fullscreen.
pub fn fullscreen_set(_loop: &mut Loop, window: Window, fullscreen: bool) {
    if fullscreen {
        if window.is_fullscreen() {
            return;
        }

        let current_loc = _loop
            .inner.space_state()
            .state
            .element_location(&window)
            .unwrap_or_default();
        // The window's PRE-fullscreen slot (what it's rendered at + what the group bbox uses).
        let current_size = slot::expected_size(&window)
            .filter(|s| s.w > 0 && s.h > 0)
            .unwrap_or_else(|| window.geometry().size);

        let (target_loc, target_size) =
            fullscreen_target(_loop, &window, current_loc, current_size);

        window.set_fullscreen(Some(WindowFullscreen {
            restore_loc: current_loc,
            restore_size: current_size,
        }));

        // Move into place and raise above peers (exclusive within its bounds).
        _loop
            .inner.space_state_mut()
            .state
            .map_element(window.clone(), target_loc, true);
        _loop.inner.space_state_mut().state.raise_element(&window, true);
        if let Some(uuid) = window.uuid() {
            _loop.inner.raise_drawable(uuid);
        }

        // The compositor-decided slot IS the fullscreen size — without this the render keeps
        // fitting the stale (pre-fullscreen) slot and the window never grows. The group bbox uses
        // the restore rect (above), so it doesn't feed back off this new slot.
        slot::set_expected_size(&window, target_size);

        shell::set_fullscreen(&window, true);
        shell::stage(&window, target_size, false);
        shell::send(&window);
    } else {
        let Some(restore) = window.fullscreen() else {
            return;
        };
        window.set_fullscreen(None);

        // Where to land.
        //
        // UNGROUPED keeps whatever the window is at NOW. Entering fullscreen
        // ungrouped is geometrically a no-op — `fullscreen_target` hands back the
        // window's own loc/size — so the stored restore rect can only ever differ
        // from the live one because the user moved or resized the window WHILE
        // fullscreen, and undoing that was the whole bug. Both live values are
        // maintained by the canvas grabs (`map_element` on move,
        // `set_expected_size` at `canvas.system/motion.rs`), so they are exactly
        // what the user left it at.
        //
        // GROUPED restores, because there the divergence is real: fullscreen
        // means the group's INNER BBOX, a different rect from the window's own
        // size by construction, so keeping it would silently adopt the group's
        // dimensions as the window's own.
        let (loc, size) = if group_of(_loop, &window).is_some() {
            (restore.restore_loc, restore.restore_size)
        } else {
            let loc = _loop
                .inner.space_state()
                .state
                .element_location(&window)
                .unwrap_or(restore.restore_loc);
            let size = slot::expected_size(&window)
                .filter(|s| s.w > 0 && s.h > 0)
                .unwrap_or(restore.restore_size);
            (loc, size)
        };

        _loop
            .inner.space_state_mut()
            .state
            .map_element(window.clone(), loc, false);

        slot::set_expected_size(&window, size);

        shell::set_fullscreen(&window, false);
        shell::stage(&window, size, false);
        shell::send(&window);
    }

    // Geometry changed; refresh the owning group's bounding box overlay.
    if let Some(uuid) = window.uuid() {
        compositor_y5_group_interface_base::interface::invalidate_bbox(_loop, uuid);
    }

    _loop.schedule_redraw();
}

/// F11: clear fullscreen on the keyboard-focused window, but only if it is
/// currently fullscreen (set via the protocol). Never enters fullscreen.
/// Returns `true` when it actually un-fullscreened a window (so the key is
/// consumed), `false` otherwise (so the key falls through to the client).
pub fn fullscreen_unset_focused(_loop: &mut Loop) -> bool {
    let Some(window) = focused_window(_loop) else {
        return false;
    };
    if !window.is_fullscreen() {
        return false;
    }
    fullscreen_set(_loop, window, false);
    true
}

/// Compute the fullscreen target rectangle (y5-world) for `window`: the group's
/// padded bbox if the window belongs to a group, otherwise its own geometry.
fn fullscreen_target(
    _loop: &mut Loop,
    window: &Window,
    current_loc: Point<i32, Logical>,
    current_size: Size<i32, Logical>,
) -> (Point<i32, Logical>, Size<i32, Logical>) {
    let Some(group_uuid) = group_of(_loop, window) else {
        return (current_loc, current_size);
    };

    let Some(group) = _loop.inner.group_mut()
        
        .group
        .iter()
        .find(|g| g.id == group_uuid)
        .cloned()
    else {
        return (current_loc, current_size);
    };

    // The group's INNER bbox (its padded bbox minus the margin), so the fullscreen window fills the
    // group's content area and the group keeps its surrounding margin.
    let rect = compositor_y5_group_interface_base::interface::bbox_inner(_loop, &group)
        .into_storage_rect();
    (rect.loc, rect.size)
}

/// The group `window` belongs to. The single answer to "does fullscreen mean
/// something other than this window's own rect for it", which both the enter
/// target and the exit restore turn on.
fn group_of(_loop: &mut Loop, window: &Window) -> Option<uuid::Uuid> {
    let uuid = window.uuid()?;
    _loop.inner.group_mut().window.get(&uuid).map(|g| g.as_ref().clone())
}

/// The window backing the current keyboard focus, if any.
fn focused_window(_loop: &Loop) -> Option<Window> {
    let focus = _loop
        .state
        .seat
        .seat
        .get_keyboard()
        .and_then(|kb| kb.current_focus())?;

    _loop
        .inner.space_state()
        .state
        .elements()
        .find(|w| find::is_surface(w, &focus))
        .cloned()
}
