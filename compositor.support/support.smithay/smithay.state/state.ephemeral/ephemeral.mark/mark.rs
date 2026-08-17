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
