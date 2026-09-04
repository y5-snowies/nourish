mod grab;
mod manager;

pub use grab::*;
pub use manager::*;
use wayland_server::protocol::wl_surface::WlSurface;

use crate::{
    utils::{IsAlive, Logical, Point, Rectangle},
    wayland::{
        compositor::with_states,
        input_method,
        shell::xdg::{self, SurfaceCachedState},
    },
};
#[cfg(feature = "xwayland")]
use crate::xwayland::X11Surface;
#[cfg(feature = "xwayland")]
use std::sync::Mutex;

/// An XWayland menu, tooltip or transient window presented as a popup.
///
/// X11 has no popup protocol: a menu is an ordinary window the client positions itself,
/// usually override-redirect. Presenting it as a [`PopupKind`] is what lets a compositor
/// give it the treatment a wayland popup gets — anchored to its parent, moved with it,
/// dismissed with it — instead of leaving it a detached top-level window.
///
/// **Only the OFFSET from the parent is ever used**, never either absolute position. An
/// X client's coordinates live in the X server's own space, which a compositor that
/// arranges windows on its own terms cannot interpret; the difference between two windows
/// in that space is meaningful regardless. That is what [`PopupKind::location`] returns
/// here, and it is why this needs the parent's surface as well as the child's.
///
/// Both surfaces are captured rather than reached for: an `X11Surface`'s `wl_surface` is
/// an `Option` behind a lock (Xwayland associates it after the window exists, and clears
/// it on unmap), while [`PopupKind::wl_surface`] must hand out a reference.
#[cfg(feature = "xwayland")]
#[derive(Debug, Clone)]
pub struct X11Popup {
    surface: WlSurface,
    parent: WlSurface,
    x11: X11Surface,
    parent_x11: X11Surface,
}

/// The parent link, mirrored onto the popup's own surface.
///
/// [`find_popup_root_surface`] and [`get_popup_toplevel_coords`] walk up the parent chain
/// given nothing but a surface, and they recognise a link by the surface's ROLE. An
/// Xwayland surface carries the xwayland-shell role rather than `xdg_popup`, so without
/// this the walk stops at the first X11 popup: a submenu would resolve its root to the
/// menu above it instead of the toplevel, and be offset by zero.
#[cfg(feature = "xwayland")]
#[derive(Debug)]
struct X11PopupLink(Mutex<X11Popup>);

#[cfg(feature = "xwayland")]
impl PartialEq for X11Popup {
    fn eq(&self, other: &Self) -> bool {
        // By window id, NEVER by `X11Surface` equality: that also requires both sides to
        // be ALIVE, so a dead surface is equal to nothing — not even itself — and every
        // teardown path holds one smithay has already marked dead.
        self.x11.window_id() == other.x11.window_id() && self.surface == other.surface
    }
}

#[cfg(feature = "xwayland")]
impl X11Popup {
    /// Present `x11` as a popup of `parent_x11`.
    ///
    /// Both surfaces must already be associated — Xwayland does that after the window is
    /// created, so a caller acting on the map request has to wait for it.
    ///
    /// The parent-relative offset is not supplied: it is read live from both windows on
    /// every query, so the popup follows a client that repositions it. See [`Self::offset`].
    pub fn new(
        x11: X11Surface,
        surface: WlSurface,
        parent_x11: X11Surface,
        parent: WlSurface,
    ) -> Self {
        let popup = X11Popup { surface, parent, x11, parent_x11 };
        with_states(&popup.surface, |states| {
            states
                .data_map
                .insert_if_missing_threadsafe(|| X11PopupLink(Mutex::new(popup.clone())));
            if let Some(link) = states.data_map.get::<X11PopupLink>() {
                if let Ok(mut link) = link.0.lock() {
                    *link = popup.clone();
                }
            }
        });
        popup
    }

    /// The X11 window behind this popup.
    pub fn x11_surface(&self) -> &X11Surface {
        &self.x11
    }

    /// The offset from the parent's origin, read LIVE from both windows.
    ///
    /// An override-redirect window is positioned by its client and by nobody else, and it
    /// keeps doing so after it is mapped: a tooltip re-anchors to follow the pointer, a
    /// menu panel shifts as it grows into a submenu. smithay tracks that —
    /// `ConfigureNotify` writes `last_configure` for override-redirect windows — so the
    /// current difference is always available. Freezing it at creation would make the
    /// popup stop tracking the moment it is first shown.
    fn offset(&self) -> Point<i32, Logical> {
        self.x11.last_configure().loc - self.parent_x11.last_configure().loc
    }
}

#[cfg(feature = "xwayland")]
impl IsAlive for X11Popup {
    #[inline]
    fn alive(&self) -> bool {
        // `is_mapped()` as well as `alive()`, and this is where the two shells differ. For
        // a wayland popup the `wl_surface` dying IS the dismissal. An X11 popup is
        // withdrawn by an UNMAP, and its `wl_surface` — owned by the one Xwayland client —
        // outlives that, while `X11Surface::alive` stays true until a `DestroyNotify` the
        // client need never send. `is_mapped()` is the surface ASSOCIATION, which smithay
        // clears inside `unmapped_window`, so it goes false exactly at withdrawal.
        //
        // `PopupManager::cleanup` retains tree nodes on this predicate, so a popup that
        // never reports dead is drawn forever.
        self.x11.alive() && self.x11.is_mapped() && self.surface.alive()
    }
}

/// The X11 popup parent of `surface`, if it is one. Lets the parent walks below follow a
/// chain that the xdg role test cannot see.
#[cfg(feature = "xwayland")]
fn x11_popup_link(surface: &WlSurface) -> Option<X11Popup> {
    with_states(surface, |states| {
        states
            .data_map
            .get::<X11PopupLink>()
            .and_then(|link| link.0.lock().ok().map(|link| link.clone()))
    })
}

#[cfg(not(feature = "xwayland"))]
fn x11_popup_link(_surface: &WlSurface) -> Option<std::convert::Infallible> {
    None
}

/// Represents a popup surface
#[derive(Debug, Clone, PartialEq)]
pub enum PopupKind {
    /// xdg-shell [`PopupSurface`](xdg::PopupSurface)
    Xdg(xdg::PopupSurface),
    /// input-method [`PopupSurface`](input_method::PopupSurface)
    InputMethod(input_method::PopupSurface),
    /// An XWayland window presented as a popup. See [`X11Popup`].
    #[cfg(feature = "xwayland")]
    X11(X11Popup),
}

impl IsAlive for PopupKind {
    #[inline]
    fn alive(&self) -> bool {
        match self {
            PopupKind::Xdg(p) => p.alive(),
            PopupKind::InputMethod(p) => p.alive(),
            #[cfg(feature = "xwayland")]
            PopupKind::X11(p) => p.alive(),
        }
    }
}

impl From<PopupKind> for WlSurface {
    #[inline]
    fn from(p: PopupKind) -> Self {
        p.wl_surface().clone()
    }
}

impl PopupKind {
    /// Retrieves the underlying [`WlSurface`]
    #[inline]
    pub fn wl_surface(&self) -> &WlSurface {
        match *self {
            PopupKind::Xdg(ref t) => t.wl_surface(),
            PopupKind::InputMethod(ref t) => t.wl_surface(),
            #[cfg(feature = "xwayland")]
            PopupKind::X11(ref t) => &t.surface,
        }
    }

    fn parent(&self) -> Option<WlSurface> {
        match *self {
            PopupKind::Xdg(ref t) => t.get_parent_surface(),
            PopupKind::InputMethod(ref t) => t.get_parent().map(|parent| parent.surface.clone()),
            #[cfg(feature = "xwayland")]
            PopupKind::X11(ref t) => Some(t.parent.clone()),
        }
    }

    /// Returns the surface geometry as set by the client using `xdg_surface::set_window_geometry`
    pub fn geometry(&self) -> Rectangle<i32, Logical> {
        let wl_surface = self.wl_surface();
        match *self {
            PopupKind::Xdg(_) => with_states(wl_surface, |states| {
                states
                    .cached_state
                    .get::<SurfaceCachedState>()
                    .current()
                    .geometry
                    .unwrap_or_default()
            }),
            PopupKind::InputMethod(ref t) => t.get_parent().map(|parent| parent.location).unwrap_or_default(),
            // Size only. An X11 window states no window geometry, and the LOCATION half
            // is `location()`'s answer — a parent-relative offset — not this one.
            #[cfg(feature = "xwayland")]
            PopupKind::X11(ref t) => Rectangle::from_size(t.x11.geometry().size),
        }
    }

    fn send_done(&self) {
        match *self {
            PopupKind::Xdg(ref t) => t.send_popup_done(),
            PopupKind::InputMethod(_) => {} //Nothing to do the IME takes care of this itself
            // X11 has no "popup done". Dismissal is the window going away, which the
            // client decides; unmapping it from under the client would only confuse it.
            #[cfg(feature = "xwayland")]
            PopupKind::X11(_) => {}
        }
    }

    fn location(&self) -> Point<i32, Logical> {
        match *self {
            PopupKind::Xdg(ref t) => {
                t.with_committed_state(|current| current.map(|state| state.geometry.loc).unwrap_or_default())
            }
            PopupKind::InputMethod(ref t) => t.location(),
            #[cfg(feature = "xwayland")]
            PopupKind::X11(ref t) => t.offset(),
        }
    }
}

#[cfg(feature = "xwayland")]
impl From<X11Popup> for PopupKind {
    #[inline]
    fn from(p: X11Popup) -> PopupKind {
        PopupKind::X11(p)
    }
}

impl From<xdg::PopupSurface> for PopupKind {
    #[inline]
    fn from(p: xdg::PopupSurface) -> PopupKind {
        PopupKind::Xdg(p)
    }
}

impl From<input_method::PopupSurface> for PopupKind {
    #[inline]
    fn from(p: input_method::PopupSurface) -> PopupKind {
        PopupKind::InputMethod(p)
    }
}
