//! Which X11 window a `wl_surface` backs — the one question the keyboard-focus path
//! asks and cannot answer from a `Window`.
//!
//! `SeatHandler::focus_changed` is implemented on `Dispatch`, which is world-free and
//! has no Space to look a window up in: it is handed a `WlSurface` and nothing else.
//! It has to answer anyway, because y5's `SeatHandler::KeyboardFocus` is a plain
//! `WlSurface`, so smithay's `KeyboardTarget for X11Surface` — where a compositor with
//! an enum focus target gets the X11 half of focus for free — is never reached and
//! `X11Surface::set_input_focus` must be driven by hand.
//!
//! **The answer is kept on the SURFACE, not in a map.** A `HashMap<WlSurface,
//! X11Surface>` has to be un-indexed as carefully as it is indexed, and the events
//! that would do it do not line up: ICCCM lets a client withdraw a window and map it
//! again without ever destroying it — what a toolkit reusing one menu window does — so
//! keying removal on destroy leaks an entry per open-and-close, for the commonest X11
//! window there is. Owned by the surface, the entry dies with it: there is no removal
//! to get wrong, and no teardown ordering to reason about when the X server goes away.

use std::sync::Mutex;

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::IsAlive;
use smithay::wayland::compositor;
use smithay::xwayland::X11Surface;

/// `Mutex<Option<_>>` rather than a bare `X11Surface` because `UserDataMap` inserts
/// once and never replaces, while `X11Surface::set_wl_surface` may re-associate.
struct X11Focus(Mutex<Option<X11Surface>>);

/// Record which X11 window a `wl_surface` backs — from the association callback, the
/// one moment both are in hand.
pub fn index(surface: &WlSurface, window: X11Surface) {
    compositor::with_states(surface, |states| {
        states.data_map.insert_if_missing_threadsafe(|| X11Focus(Mutex::new(None)));
        if let Some(slot) = states.data_map.get::<X11Focus>() {
            if let Ok(mut slot) = slot.0.lock() {
                *slot = Some(window);
            }
        }
    });
}

/// The X11 window behind `surface`, or `None` for a wayland surface.
///
/// Both liveness checks are load-bearing rather than defensive: `with_states` panics
/// on a surface whose data is gone, and `focus_changed` is routinely handed the
/// PREVIOUS focus, which is frequently a surface that has just been destroyed; and
/// after `xwayland::died` every surviving surface still names a dead `X11Surface`,
/// which is what makes clearing this on teardown unnecessary.
pub fn indexed(surface: &WlSurface) -> Option<X11Surface> {
    if !surface.alive() {
        return None;
    }
    compositor::with_states(&root_of(surface), |states| {
        let slot = states.data_map.get::<X11Focus>()?;
        let window = slot.0.lock().ok()?.clone()?;
        window.alive().then_some(window)
    })
}

/// Walk a surface up to the root of its tree.
///
/// **The index is keyed on the ROOT surface** — `surface_associated` is handed the one
/// Xwayland attached to the X11 window — while callers arrive holding whatever the hit
/// test resolved, and that is the DEEPEST surface under the cursor
/// (`surface_under(.., TOPLEVEL | SUBSURFACE)`). For a client whose window is a single
/// flat surface the two are the same and the lookup happened to work; for one that builds
/// its window out of subsurfaces the caller holds a child, misses the index, and is told
/// no X11 window is there at all.
///
/// That asymmetry is worth naming because of how it presents: it looks like the pointer
/// entering nothing rather than like a failed lookup, so every consequence downstream —
/// no window made reachable, X free to route the event wherever its own geometry says —
/// reads as a routing bug rather than an indexing one. xterm resolved correctly and
/// Firefox did not, in the same session.
fn root_of(surface: &WlSurface) -> WlSurface {
    let mut root = surface.clone();
    while let Some(parent) = compositor::get_parent(&root) {
        if !parent.alive() {
            break;
        }
        root = parent;
    }
    root
}
