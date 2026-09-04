//! "The user did not place this window and will not come back to it."
//!
//! Two kinds of toplevel qualify: one a client marked modal through
//! `xdg_wm_dialog_v1`, and one belonging to a desktop entry that is not
//! user-launchable at all (`NoDisplay=true` — portal backends, MIME handlers).
//! The layers above read the mark to decide whether the window deserves to be
//! remembered when it closes.
//!
//! Nothing here knows what that decision is; the whole point of living this low
//! is that the two sites which can OBSERVE the fact — the xdg-dialog handler in
//! the dispatch layer, and window map in the expansion layer — can both reach it.

use smithay::desktop::Window;
use smithay::wayland::seat::WaylandFocus;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{SurfaceData, with_states};

/// Marker in a toplevel `wl_surface`'s `data_map`. Presence is the whole value.
///
/// Sticky by design — set once, never cleared, which is what makes it readable
/// during teardown. Both facts it stands for evaporate exactly when they would
/// be needed: `xdg_dialog_v1`'s destructor resets smithay's own `dialog_hint`
/// back to `Unknown` (and a well-behaved client fires it alongside the
/// toplevel), and the desktop entry is only resolvable while the process is
/// alive to be introspected.
pub struct Ephemeral;

/// Mark `surface` ephemeral. Idempotent; a second call is a no-op.
pub fn mark(surface: &WlSurface) {
    with_states(surface, |states| {
        states.data_map.insert_if_missing_threadsafe(|| Ephemeral);
    });
}

/// Whether `states` carries the mark.
///
/// Takes the already-borrowed `SurfaceData` rather than a `WlSurface` because
/// the read happens during surface destruction, inside a `with_states` the
/// caller has already opened — `with_states` is not re-entrant.
pub fn is_marked(states: &SurfaceData) -> bool {
    states.data_map.get::<Ephemeral>().is_some()
}

/// Mark a `Window` ephemeral — in BOTH homes, which is the whole point.
///
/// A caller holding a `Window` must never pick one. The surface is where the xdg
/// teardown reads (it is handed a `ToplevelSurface` and never sees the `Window`), and
/// the window is where the X11 teardown reads — because for an X11 window the surface
/// is the wrong place and silently so: smithay clears the association in
/// `unmapped_window`, the same event that queues the destroy, so by the time the drain
/// applies it there is nothing left to read the mark off. Only the `Window` outlives
/// that.
///
/// So this writes both and the caller states no preference. Leaving the choice to the
/// call site is what produced the bug this replaces: marking the surface alone, which
/// worked for every wayland window and silently did nothing for every X11 one.
///
/// [`mark`] remains for the one writer that has no `Window` — the xdg-dialog handler,
/// which observes the fact from the dispatch layer where windows do not exist.
pub fn mark_window(window: &Window) {
    if let Some(surface) = window.wl_surface() {
        mark(&surface);
    }
    window.user_data().insert_if_missing_threadsafe(|| Ephemeral);
}

/// Whether `window` carries the mark. The counterpart to [`is_marked`] for the
/// teardown path that holds a `Window` instead of a live surface.
///
/// The two READERS cannot be merged the way the writers were, and it is worth knowing
/// why before trying. [`is_marked`] takes an already-borrowed `SurfaceData` because
/// the xdg teardown reads it from inside a `with_states` it has already opened, and
/// `with_states` is not re-entrant — so that caller cannot be handed anything that
/// opens one. It also has no `Window` to give: it is passed a `ToplevelSurface`. And
/// this caller has the mirror problem — by the time an X11 destroy is applied there is
/// no surface to read. Two callers, two different handles, no overlap.
pub fn is_window_marked(window: &Window) -> bool {
    window.user_data().get::<Ephemeral>().is_some()
}
