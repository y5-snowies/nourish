//! Where a CHILD window goes, expressed the only way that crosses the X boundary.
//!
//! An X11 client positions its own menus, tooltips and dialogs by choosing a place in X
//! space. That absolute number means nothing to y5 — `shell::X11_ORIGIN` explains why —
//! but the DIFFERENCE between a child and its parent is meaningful in whatever space
//! they share, so it survives the boundary intact while neither absolute does. That is
//! exactly how xwayland-satellite placed X11 popups: `create_popup` feeds
//! `child.x - parent.x` into an `xdg_positioner` and never the absolutes.
//!
//! The parent is `WM_TRANSIENT_FOR`, X11's own statement of the relationship. A window
//! that names no parent gets `None` and is placed like any other window; nothing here
//! guesses.

use smithay::desktop::{PopupKind, Window};
use smithay::desktop::X11Popup;
use smithay::utils::{Logical, Point, Size};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::seat::WaylandFocus;

/// The parent this window declares, and the offset from that parent's origin the
/// client placed itself at.
///
/// `None` for a wayland window, for an X11 window with no `WM_TRANSIENT_FOR`, and for
/// one whose parent is not among `windows` — a child whose parent y5 does not have
/// mapped has nothing to be relative to.
///
/// Read live from both windows' positions, which is sound because y5 never moves an X11
/// window: `shell::flush_pending` echoes the position back, so a configure only ever
/// resizes and X keeps its own layout. The difference is therefore the client's own
/// arithmetic, whatever absolute space it did it in.
pub fn parent_offset<'a, I>(
    windows: I,
    child: &Window,
    fallback: Option<smithay::xwayland::xwm::X11Window>,
) -> Option<(Window, Point<i32, Logical>)>
where
    I: IntoIterator<Item = &'a Window>,
{
    let x11 = child.x11_surface()?;
    // `WM_TRANSIENT_FOR` first, then the caller's fallback — the last hovered or focused
    // window. X11 menus mostly name no parent: an override-redirect window told the X
    // server no WM should be involved and then positioned itself, so the relationship
    // exists in the user's head and nowhere in the protocol. Without a fallback such a
    // menu is placed like a freshly launched window, which is Steam's topbar menu landing
    // nowhere near Steam. satellite's rule, verbatim.
    let parent_id = x11.is_transient_for().or(fallback)?;
    let parent = windows
        .into_iter()
        .find(|w| w.x11_surface().is_some_and(|p| p.window_id() == parent_id))?;
    let offset = x11.last_configure().loc - parent.x11_surface()?.last_configure().loc;
    Some((parent.clone(), offset))
}

/// Present this window as a POPUP of the parent it declares, if it should be one.
///
/// The other half of [`parent_offset`], and the one that makes an X11 menu behave like a
/// wayland one: tracked in the `PopupManager`, it is drawn and hit-tested through the
/// paths that already walk `popups_for_surface`, it follows its parent when the parent
/// moves, and it is not a window — no uuid, no Space slot, no decoration, no placeholder,
/// nothing in the dock or the navigator.
///
/// `None` means "present it as an ordinary window instead", and it is returned for four
/// distinct reasons, none of them failures:
///
/// - it is not an X11 window;
/// - it is not a POPUP (`ident::is_popup_x11`) — a dialog, dock or splash is a window in
///   its own right even when it names a parent — or it is FULLSCREEN, which `outputs`
///   (the logical output sizes) is there to recognise;
/// - it names no parent AND the caller offers no fallback, so there is nothing to anchor
///   to;
/// - a surface is missing on either side. Xwayland associates a `wl_surface` after the
///   window exists, and a popup needs both its own and its parent's, so a caller acting
///   too early gets `None` and should try again once the association lands.
/// `popup_parent` is what makes a chain work: a submenu's parent is the MENU, and a
/// tracked popup is not a Space element, so `windows` cannot answer for it. Resolving it
/// is not a placement detail — an unresolvable parent makes this return `None`, and the
/// window path then gives the submenu a uuid, a Space slot, a decoration and a
/// camera-centred position. Depth-2 chains were unreachable without it.
pub fn as_popup<'a, I, O, P>(
    windows: I,
    child: &Window,
    fallback: Option<smithay::xwayland::xwm::X11Window>,
    outputs: O,
    popup_parent: P,
) -> Option<PopupKind>
where
    I: IntoIterator<Item = &'a Window>,
    O: IntoIterator<Item = Size<i32, Logical>>,
    P: Fn(smithay::xwayland::xwm::X11Window) -> Option<(smithay::xwayland::X11Surface, WlSurface)>,
{
    // `is_popup_x11`, NOT `is_ephemeral_x11`: the two answer different questions, and the
    // ephemeral one is wrong here — it counts docks, desktops, splashes and notifications,
    // which are transient but are not chrome belonging to another window.
    if !compositor_support_smithay_state_window_ident::ident::is_popup_x11(child) {
        return None;
    }
    // A FULLSCREEN window is never a popup. Some games map override-redirect at output
    // size, satisfying every ephemerality test there is; presenting one as a popup would
    // deny it a Space slot, a uuid, a placeholder and a close path, and anchor it to
    // whatever was last pointed at.
    //
    // Two tests, because the first cannot see the case it was written for: smithay reads
    // `_NET_WM_STATE_FULLSCREEN` in its MapRequest arm, and an override-redirect window
    // never takes that arm. So an override-redirect window that names NO parent and
    // covers an output is taken as fullscreen on its size alone. One that DOES name a
    // parent is a menu however large. (xwayland-satellite has no size guard at all — its
    // `guess_is_popup` answers override-redirect first.)
    if compositor_support_smithay_state_window_ident::ident::states(child).fullscreen {
        return None;
    }
    // Collected because the resolver runs up to twice — once for the declared parent,
    // once for the fallback — and `I` is a one-shot iterator.
    let windows_vec: Vec<&Window> = windows.into_iter().collect();
    let x11 = child.x11_surface()?;
    if x11.is_override_redirect() && x11.is_transient_for().is_none() {
        let size = x11.last_configure().size;
        if outputs.into_iter().any(|output| size.w >= output.w && size.h >= output.h) {
            return None;
        }
    }
    // The declared parent first, then the caller's fallback — and each is tried against
    // BOTH sources before the next id is considered, because "the id resolved to nothing"
    // and "there is no id" are different failures. The old code consumed the fallback at
    // the id (`is_transient_for().or(fallback)`) and then gave up if the lookup missed,
    // so a submenu naming a menu that is a popup resolved nothing at all.
    let x11 = x11.clone();
    let parent_id = x11.is_transient_for();
    let resolve = |id| {
        // A Space element first: the ordinary case, a child of a real window.
        windows_vec
            .iter()
            .find(|w| w.x11_surface().is_some_and(|p| p.window_id() == id))
            .and_then(|w| Some((w.x11_surface()?.clone(), w.wl_surface()?.into_owned())))
            // Then a tracked popup, which is what a submenu's parent is.
            .or_else(|| popup_parent(id))
    };
    let (parent_x11, parent_surface) = parent_id
        .and_then(resolve)
        .or_else(|| fallback.and_then(resolve))?;
    let surface = child.wl_surface()?.into_owned();
    Some(PopupKind::X11(X11Popup::new(x11, surface, parent_x11, parent_surface)))
}
