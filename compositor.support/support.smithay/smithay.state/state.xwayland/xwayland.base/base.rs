//! The XWayland half of the protocol state, held by `Dispatch`.
//!
//! Two connections make XWayland work (see smithay's `xwayland` module): the
//! Xwayland process is an ordinary WAYLAND client of ours whose surfaces arrive
//! through the normal compositor path, and separately we act as its X11 WINDOW
//! MANAGER over a private socket. `shell` is the wayland half — the
//! `xwayland_shell_v1` global Xwayland binds to tell us which wl_surface backs
//! which X11 window — and `xwm` is the window-manager half.
//!
//! The outboxes below exist for the same reason as the ones on `Dispatch` itself:
//! `XwmHandler` runs inside the X11 event source with no access to the world, so it
//! records what happened and `Wire::drain_protocol` applies it against the Space.
//!
//! There are deliberately NO per-kind outboxes here. Map, destroy and fullscreen all
//! go onto the SAME `Dispatch` buffers as their xdg counterparts
//! (`new_toplevels`, `destroyed_toplevels`, `fullscreen_requests`), tagged with
//! `find::Shell`. Two reasons: one drain arm per kind means an X11 window picks up the
//! same uuid, placeholder, draw-order and introspection treatment as any other, and
//! the buffers stay ORDERED — a separate X11 drain ran after `foreign_reconcile` had
//! already published the frame's dock state, so a destroyed X11 window lingered there
//! for a frame.
//!
//! What is left here is state with no xdg counterpart at all.

use smithay::wayland::xwayland_shell::XWaylandShellState;
use smithay::xwayland::X11Wm;

pub struct Xwayland {
    pub shell: XWaylandShellState,
    /// `None` until the Xwayland server signals ready and the loader starts the
    /// window manager, and `None` again once the server dies — every X11 path has to
    /// tolerate both windows.
    pub xwm: Option<X11Wm>,
    /// The Xwayland server's own wayland client, set alongside `xwm`.
    ///
    /// Kept purely as a LIVENESS handle: smithay's `XWayland` event source disables
    /// itself after `Ready` and only ever watched the displayfd pipe, so nothing
    /// reports the server exiting later. Asking the display whether this client still
    /// exists is the cheap, exact test — the client is destroyed when the process's
    /// socket closes, whatever killed it.
    pub client: Option<smithay::reexports::wayland_server::backend::ClientId>,
    /// The FALLBACK PARENT for a newly mapped X11 window that declares none: the window
    /// the pointer was last over, and behind it the one that last held keyboard focus.
    ///
    /// Named for what they are USED for, not what they record. Neither is focus or hover
    /// state; nothing reads them to decide what is focused, raised or routed input. They
    /// are consumed at exactly two one-shot-at-map sites — `child::as_popup` in the drain
    /// (popup-or-window, and where it goes) and `child::parent_offset` in
    /// `_initial_mapped` (where a window `as_popup` declined goes). Once a window is
    /// placed they are irrelevant to it forever.
    ///
    /// Needed because X11 menus mostly set no `WM_TRANSIENT_FOR` — the client asked that
    /// no window manager be involved and positions itself absolutely — while the only
    /// geometric fact that crosses the X boundary is a parent-relative DIFFERENCE. With
    /// no parent there is nothing to anchor to, so the question is answered from what the
    /// user was doing. satellite's rule verbatim (`popup_for = last_hovered.or(
    /// last_focused_toplevel)`), and what made X11 popups land correctly there.
    ///
    /// `_hover` excludes popups (`raise_x11_for_pointer`): a menu is chrome, never the
    /// answer to "what does this chrome belong to", and recording one leaves the NEXT
    /// tooltip unresolvable. `_focus` needs no such test only because a popup is not a
    /// Space element and so can never take y5's keyboard focus.
    pub map_position_parent_hover: Option<smithay::xwayland::xwm::X11Window>,
    pub map_position_parent_focus: Option<smithay::xwayland::xwm::X11Window>,
    /// Set by `XwmHandler::disconnected` — the ONLY signal an ORDERLY Xwayland exit
    /// produces. smithay's X11 source turns connection loss into `Ok`, calls that
    /// handler and removes itself, so the loader's pump never sees a dispatch error,
    /// and the wayland client can outlive the X connection by an iteration. The pump
    /// reads this on the same iteration and runs the crash teardown; `died` clears it.
    pub disconnected: bool,
}

/// Whether a given wayland client is the Xwayland server, answerable WITHOUT `&Dispatch`.
///
/// A mirror of the `client` field above, and it exists for one reason: global visibility
/// is decided by `GlobalDispatch::can_view`, an associated function that is handed a
/// `Client` and the global's data and nothing else. There is no route from there to the
/// compositor state, so a filter that must exclude Xwayland cannot read the field.
///
/// A `OnceLock`, which states the lifetime rule in the type: ONE Xwayland per compositor
/// process. `xwayland::died` retires the server and deliberately does not respawn, so
/// there is never a second id to record — and if that decision is ever revisited, this is
/// where it stops compiling, which is the right place to have the conversation.
///
/// It is also why nothing CLEARS this when the server dies, unlike [`Xwayland::client`]:
/// a `ClientId` is `{ id, serial }` with the serial a generation counter, so a reused slot
/// yields an UNEQUAL id. The stale entry matches no live client and [`is_xwayland`] keeps
/// answering `false` for everyone. Clearing would be tidiness, not correctness.
static XWAYLAND_CLIENT: std::sync::OnceLock<smithay::reexports::wayland_server::backend::ClientId> =
    std::sync::OnceLock::new();

/// Record which client is Xwayland. Call alongside the write of [`Xwayland::client`].
///
/// `false` = already set, i.e. a second Xwayland was spawned, which nothing does today.
/// The caller decides whether that deserves noise; this crate has no logging instance.
pub fn set_filter_client(
    id: smithay::reexports::wayland_server::backend::ClientId,
) -> bool {
    XWAYLAND_CLIENT.set(id).is_ok()
}

/// Is this the Xwayland server's own wayland client?
///
/// `false` before Xwayland has connected, and `false` for every live client after it dies
/// (see the static's note on `ClientId` generations) — which is the safe answer for a
/// visibility filter either way: an unknown client is an ordinary one.
pub fn is_xwayland(client: &smithay::reexports::wayland_server::Client) -> bool {
    XWAYLAND_CLIENT.get().is_some_and(|id| *id == client.id())
}
