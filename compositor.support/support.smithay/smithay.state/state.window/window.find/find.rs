//! Finding the window behind a `wl_surface`.
//!
//! One expression, previously inlined thirteen times and wrapped by three separate
//! near-identical helpers. It is here because the obvious spelling —
//! `w.toplevel().unwrap().wl_surface() == surface` — is wrong twice: it aborts on
//! any window that is not an xdg toplevel (an X11 window is not), and the variants
//! that guarded with `map(..).unwrap_or(false)` each spelled it differently enough
//! that the tree had four subtly divergent copies of "is this that window".
//!
//! `WaylandFocus::wl_surface` is the shell-agnostic answer: the toplevel's surface
//! for xdg, the associated surface for X11. It is an `Option` — an X11 window has
//! no surface until Xwayland associates one — so every caller here is `None`-safe
//! by construction rather than by remembering.

use smithay::desktop::{Space, Window};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::seat::WaylandFocus;
use smithay::xwayland::X11Surface;

/// Is `surface` this window's own surface?
pub fn is_surface(window: &Window, surface: &WlSurface) -> bool {
    window.wl_surface().is_some_and(|s| s.as_ref() == surface)
}

/// The window in `space` whose surface is `surface`.
pub fn in_space(space: &Space<Window>, surface: &WlSurface) -> Option<Window> {
    space.elements().find(|w| is_surface(w, surface)).cloned()
}

/// The window among `windows` whose surface is `surface` — for the callers holding
/// an iterator over several spaces rather than one.
pub fn among<'a, I>(windows: I, surface: &WlSurface) -> Option<Window>
where
    I: IntoIterator<Item = &'a Window>,
{
    windows.into_iter().find(|w| is_surface(w, surface)).cloned()
}

/// The window among `windows` backed by `surface`.
///
/// Keyed on the X11 surface rather than the wl_surface because by the time the X
/// server says a window is gone, the association may already be — which is exactly
/// when the teardown paths need to find it.
///
/// Compares the X11 window ID, and **must not** use `==` on the surfaces.
/// `impl PartialEq for X11Surface` is
///
/// ```text
/// self.xwm == other.xwm && self.window == other.window && self_alive && other_alive
/// ```
///
/// — a DEAD surface is equal to nothing, not even to itself. Every caller here is a
/// teardown path holding the surface smithay handed to `destroyed_window`, and
/// `handle_destroyed` clears `alive` while the X11 event is being dispatched, before
/// the drain that uses it ever runs. Matching on `==` therefore finds nothing exactly
/// when it matters: the window is never unmapped and never gets its `Destroyed`
/// lifecycle event, so it leaves no placeholder. The id is a plain `u32` field that
/// outlives the surface, which is why smithay hands the surface back at all.
pub fn by_x11<'a, I>(windows: I, surface: &X11Surface) -> Option<Window>
where
    I: IntoIterator<Item = &'a Window>,
{
    let id = surface.window_id();
    windows
        .into_iter()
        .find(|w| w.x11_surface().is_some_and(|x11| x11.window_id() == id))
        .cloned()
}

/// Which shell a queued protocol event arrived on.
///
/// The outboxes on `Dispatch` are ordered buffers, and an event is only meaningful in
/// the order it happened — so xdg and X11 events of the same KIND belong in the same
/// buffer, not in two that are drained at different points. The handlers cannot queue
/// a `Window` (that lives in the Space, which the wayland dispatch layer cannot see,
/// and constructing a fresh one would not be the same window), so they queue the
/// surface they were handed and the drain resolves it with [`window_of`].
#[derive(Debug, Clone, PartialEq)]
pub enum Shell {
    Xdg(smithay::wayland::shell::xdg::ToplevelSurface),
    X11(X11Surface),
}

/// The mapped window a queued [`Shell`] event refers to.
///
/// Searched across every space it is given, because teardown does not respect which
/// world the user is looking at: a window can die while its world is off screen, and
/// resolving against the hosted space alone would drop the event and leave the window
/// mapped forever in the world it actually lives in.
pub fn window_of<'a, I>(windows: I, shell: &Shell) -> Option<Window>
where
    I: IntoIterator<Item = &'a Window>,
{
    match shell {
        Shell::Xdg(toplevel) => among(windows, toplevel.wl_surface()),
        Shell::X11(surface) => by_x11(windows, surface),
    }
}
