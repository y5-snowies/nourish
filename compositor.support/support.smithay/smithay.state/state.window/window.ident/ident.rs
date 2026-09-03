//! Window identity — who a window BELONGS to, and what it calls itself.
//!
//! The dangerous half is [`credentials`]. Every X11 window's `wl_surface` belongs
//! to the ONE Xwayland client, so surface credentials name the X SERVER rather than
//! the application: every X11 window would report the same pid, which collapses
//! per-window process introspection (the launcher match, the placeholder identity,
//! the Steam attribution the tearing tag is built on) and — worse — makes the kill
//! paths take down the X server and every other X11 app with it. `_NET_WM_PID` is
//! the X client's own pid, and that is what every caller here means.
//!
//! The naming half is mundane by comparison: X11 answers "what is this" through
//! `WM_CLASS` and `WM_NAME`, and a dock, the launcher matcher and the icon
//! inference all read the identical two fields — so the X11 `class` lands in
//! `app_id` and `WM_NAME` in `title`, with no third vocabulary for callers to
//! learn.

use smithay::desktop::Window;
use smithay::reexports::wayland_server::{DisplayHandle, Resource};
use smithay::wayland::compositor;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::shell::xdg::XdgToplevelSurfaceRoleAttributes;
use std::sync::Mutex;

/// The credentials of the process that owns a window.
///
/// `uid`/`gid` are unknown for an X11 window — `_NET_WM_PID` carries no such thing —
/// and the `/proc` walk keyed on the pid fills in everything else the same way it
/// does for a wayland client.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Credentials {
    pub pid: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

/// What a window calls itself. Empty strings are normalised to `None` — an unset X11
/// property reads as one, and callers test `is_none`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Names {
    pub title: Option<String>,
    pub app_id: Option<String>,
}

/// What the compositor has declared about a window's state, in the vocabulary a
/// dock expects.
///
/// `maximized` and `minimized` are ALWAYS false, for both shells: y5 has neither
/// concept, and the foreign-toplevel handler deliberately drops the requests that
/// would toggle them (`Dispatch`'s `zwlr_foreign_toplevel_handle_v1` arm ignores
/// `set_maximized`/`unset_maximized`/`set_minimized`, so a dock acting on them gets
/// nothing). Advertising either as true would put an affordance in every dock and
/// then swallow the press. See [`states`] for why `maximized` is not simply read
/// from the shell.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct States {
    pub maximized: bool,
    pub minimized: bool,
    pub activated: bool,
    pub fullscreen: bool,
}

/// See the module docs: `_NET_WM_PID` for X11, surface credentials otherwise.
pub fn credentials(window: &Window, display_handle: &DisplayHandle) -> Credentials {
    if let Some(x11) = window.x11_surface() {
        return Credentials { pid: x11_pid(x11), uid: None, gid: None };
    }
    let Some(client) = window.wl_surface().and_then(|s| s.client()) else {
        return Credentials::default();
    };
    match client.get_credentials(display_handle) {
        Ok(creds) => Credentials {
            pid: Some(creds.pid as u32),
            uid: Some(creds.uid as u32),
            gid: Some(creds.gid as u32),
        },
        Err(_) => Credentials::default(),
    }
}

/// The pid of an X11 window's client, asked of the X SERVER and cached.
///
/// **`XResQueryClientIds` first, `_NET_WM_PID` only as a fallback.** `_NET_WM_PID` is a
/// number the client wrote with its own `getpid()`, so it is the pid in the CLIENT's PID
/// namespace: a sandboxed client — Steam's pressure-vessel, Flatpak, any `bwrap` —
/// advertises one that on this side is unused or belongs to an unrelated process. That is
/// worse than having none, because every consumer here looks the pid up: the `/proc` walk
/// comes back empty (which is how a Steam title arrived with no exe, env or args), and the
/// kill paths in `select.overlay` would be aiming at a stranger.
///
/// The X-Resource answer comes from `SO_PEERCRED` on the client's connection, and peer
/// credentials are translated by the KERNEL into the reading process's namespace — the
/// same property that makes wayland's `get_credentials` trustworthy. Xwayland runs where
/// the compositor spawned it, so the pid is usable regardless of the client's sandbox.
///
/// Cached on the `X11Surface`, because it costs a round trip and cannot change: a client
/// cannot re-parent its own X connection. `credentials` is on the introspection sampler's
/// path, so asking per sample would put a synchronous X round trip on it.
fn x11_pid(x11: &smithay::xwayland::X11Surface) -> Option<u32> {
    struct ClientPid(Option<u32>);
    let data = x11.user_data();
    if let Some(cached) = data.get::<ClientPid>() {
        return cached.0;
    }
    let pid = x11.client_pid().or_else(|| x11.pid());
    data.insert_if_missing_threadsafe(|| ClientPid(pid));
    data.get::<ClientPid>().map_or(pid, |cached| cached.0)
}

/// The owning pid alone — the kill paths and the `/proc` walk.
pub fn pid(window: &Window, display_handle: &DisplayHandle) -> Option<u32> {
    credentials(window, display_handle).pid
}

/// The window's title and app id: xdg role attributes, or the X11 `WM_NAME` /
/// `WM_CLASS` pair.
///
/// BOTH at once, in one pass, because every caller wants both and the xdg half is not
/// free: each read opens a `with_states` on the surface and takes the role attributes'
/// mutex, so asking twice paid for two of each to answer one question.
///
/// `class` rather than `instance` for the app id: it is the per-APPLICATION half of
/// `WM_CLASS` (the second string), which is what an `app_id` means.
pub fn names(window: &Window) -> Names {
    let clean = |s: String| Some(s).filter(|s| !s.is_empty());
    if let Some(x11) = window.x11_surface() {
        return Names { title: clean(x11.title()), app_id: clean(x11.class()) };
    }
    let Some(toplevel) = window.toplevel() else { return Names::default() };
    compositor::with_states(toplevel.wl_surface(), |states| {
        let Some(role) = states.data_map.get::<Mutex<XdgToplevelSurfaceRoleAttributes>>() else {
            return Names::default();
        };
        let Ok(guard) = role.lock() else { return Names::default() };
        Names {
            title: guard.title.clone().and_then(clean),
            app_id: guard.app_id.clone().and_then(clean),
        }
    })
}

/// The window's own root surface — the one its identity is read from, and the one
/// whose credentials name its client.
///
/// For an xdg window this is EXACTLY `toplevel().wl_surface()`: `WaylandFocus for
/// Window` matches the same `WindowSurface::Wayland(ToplevelSurface)` and borrows the
/// same field, so substituting it changed nothing for wayland clients. It is the
/// window's root surface either way — never a subsurface or popup.
///
/// **`None` is reachable, and only for X11.** An X11 window has no wl_surface until
/// Xwayland associates one, which can still be pending just after it maps. Callers
/// replacing a `toplevel().map(…)` therefore inherit a `None` case that could not
/// arise before, so decide what it means rather than letting it fall through — and
/// note the decision genuinely differs per caller. For a hit test or a geometry mirror
/// "absent" is the right answer. For `KeyboardHandle::set_focus`, `None` means CLEAR
/// rather than skip, and clearing is usually what you want: skipping leaves the
/// PREVIOUS window holding the keyboard while the new one is shown as active
/// (`activate_window` documents that trade). Neither reading is the default.
///
/// **And for X11 it is a mutable slot, not a fixed field.** An xdg toplevel's surface
/// is set once and lives as long as the toplevel; `X11Surface::set_wl_surface` REPLACES
/// one (it tears down the previous surface's pre-commit hooks first), and clears it to
/// `None` on unmap. So long-lived state keyed on an X11 window's surface can silently
/// go missing — the uuid, the ephemeral mark, `DiscardPlaceholder`.
///
/// And it DOES bite, because an X11 unmap is a hide: the `Window` outlives the clear and
/// a remap associates a fresh surface. Durable per-window state therefore lives on the
/// `Window` (the uuid, the ephemeral mark, `DiscardPlaceholder`) or is re-applied at
/// `surface_associated` (the tearing tag). Key nothing lasting on this.
/// Is there anything on screen for this window — a buffer, at a non-degenerate size?
///
/// A Space element is not automatically one. An X11 unmap is a HIDE (the `Window` stays,
/// its `wl_surface` cleared, until `DestroyNotify`) and an xdg toplevel that commits a
/// null buffer keeps its element the same way, so both leave something that must not be
/// drawn, framed or navigated to.
///
/// Protocol-agnostic on purpose — [`surface`] would answer identically and
/// [`xdg_surface`] would answer wrongly (`None` for every X11 window).
pub fn is_drawn(window: &Window) -> bool {
    window.wl_surface().is_some()
        && window.geometry().size.w > 0
        && window.geometry().size.h > 0
}

pub fn surface(
    window: &Window,
) -> Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface> {
    window.wl_surface().map(|s| s.into_owned())
}

/// The surface for XDG-ONLY protocols, and `None` for an X11 window BY DESIGN.
///
/// The counterpart to [`surface`]. Both return the window's root surface for an xdg
/// window and they are interchangeable there — the difference is entirely what they
/// mean for X11, and therefore what a reader is entitled to assume:
///
/// - [`surface`] — "the window's surface", for protocol-agnostic work (seat focus,
///   surface markers, geometry). An X11 window HAS one; `None` only means Xwayland
///   has not associated it yet.
/// - this — "the surface this XDG PROTOCOL lives on". An X11 window has no xdg
///   protocols, so it has none of these.
///
/// **`None` here says nothing about the CAPABILITY.** It says the wayland protocol
/// does not apply — not that X11 cannot answer the question. X11 usually can, through
/// a property, and treating this `None` as "feature unavailable for X11" is how a
/// feature silently gets dropped for half the windows on screen. The three current
/// callers, and where each actually stands:
///
/// | xdg protocol | X11 equivalent | read by |
/// |---|---|---|
/// | `xdg_activation_v1` | `_NET_STARTUP_ID` | `LoopWindow::activation` |
/// | `xdg_session_management_v1` | `WM_WINDOW_ROLE` + `WM_CLASS` | `LoopWindow::session` |
/// | `xdg_toplevel_icon_v1` | `_NET_WM_ICON` | `extraction.window/window.icon.toplevel` |
///
/// All three now have an X11 route, so a `None` from here currently means only "not
/// the xdg one". Keep it that way: if a fourth caller appears, find the X11 property
/// before concluding the feature does not apply.
///
/// So use this to say "the xdg half", and then decide what the X11 half is — do not
/// stop at the `None`.
pub fn xdg_surface(
    window: &Window,
) -> Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface> {
    window.toplevel().map(|t| t.wl_surface().clone())
}

/// The window's state in the dock's vocabulary — see [`States`] for the two fields
/// that are constant.
///
/// **`maximized` is hardcoded rather than read**, and the two shells are the reason.
/// Nothing in y5 ever writes `xdg_toplevel::State::Maximized`, so the xdg arm could
/// only ever answer false — but X11's `_NET_WM_STATE_MAXIMIZED_HORZ`/`_VERT` are
/// properties on the CLIENT's window, and smithay copies whatever the client set
/// before mapping straight into its mirror (`xwm/mod.rs`, "Read initial
/// `_NET_WM_STATE`"). A "start maximized" app — common, and near-universal for games
/// — would therefore report maximized while an identical wayland app never can, and
/// nothing afterwards would ever clear it: y5 does not implement `maximize_request`,
/// so smithay's default no-op drops the client's later requests and that startup
/// guess is frozen for the window's life. Reading it would make this the one field
/// in [`States`] that is the X client's answer rather than y5's.
///
/// `activated` and `fullscreen` ARE read, because y5 writes both — but note the two
/// arms are not symmetric even so: the xdg bit is PENDING state (staged, reaching
/// the client at the next configure) while the X11 bit is a property already written
/// to the server, and a client may likewise pre-declare `_NET_WM_STATE_FOCUSED`
/// through the same pre-map path. Both read back our intent, not the client's
/// acknowledgement.
pub fn states(window: &Window) -> States {
    if let Some(x11) = window.x11_surface() {
        return States {
            maximized: false,
            minimized: false,
            activated: x11.is_activated(),
            fullscreen: x11.is_fullscreen(),
        };
    }
    let Some(toplevel) = window.toplevel() else { return States::default() };
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as Xdg;
    toplevel.with_pending_state(|s| States {
        maximized: false,
        minimized: false,
        activated: s.states.contains(Xdg::Activated),
        fullscreen: s.states.contains(Xdg::Fullscreen),
    })
}


/// Is this window a POPUP — a menu, tooltip, dropdown or drag icon?
///
/// A different question from [`is_ephemeral_x11`], and they must not be conflated. That
/// one asks "will the user come back to this?", which decides whether a placeholder is
/// left behind. This one asks "is this chrome belonging to another window?", which decides
/// whether it is presented through the `PopupManager` at all — and getting it wrong costs
/// a window its Space slot, its uuid and its decoration.
///
/// The classification is xwayland-satellite's, whose `guess_is_popup` is the accumulated
/// result of a lot of misbehaving X clients. Two of its rules are the ones that matter:
///
/// - **An explicit `_NET_WM_WINDOW_TYPE` is believed.** Only the genuinely-chrome types
///   count — menu, popup menu, dropdown, tooltip, dnd, combo. `Dock`, `Desktop`, `Splash`
///   and `Notification` are NOT popups however transient they look: a dock is a panel, and
///   presenting one as a popup would anchor a persistent piece of UI to whatever the user
///   last pointed at. `is_ephemeral_x11` does list them, correctly, for its own question.
/// - **`WM_DELETE_WINDOW` means TOPLEVEL.** A window that asks to be told before it closes
///   is independent and closeable; real menus are dismissed, not closed. This is satellite's
///   `--popup-fix`, which the y5 deployment ran with: without it a borderless CSD window
///   was demoted to a popup and rendered as an empty surface — Isaac Sim's torn-off
///   "Layer" and "Render Settings" panels were the case that found it.
///
/// Satellite's softer MOTIF and `WM_HINTS` heuristics are deliberately NOT carried over.
/// They are what `--popup-fix` had to suppress, so they are the part with a known false
/// positive rate; a window that declares no type and does not skip the taskbar is treated
/// as an ordinary window here rather than guessed at.
pub fn is_popup_x11(window: &Window) -> bool {
    window.x11_surface().is_some_and(is_popup_x11_surface)
}

/// [`is_popup_x11`] asked with the `X11Surface` alone.
///
/// The map drain has a `Window`; the surface-association callback has only this. Both
/// have to ask, because the question decides whether a window is HELD until its surface
/// arrives — see the map arm of `Wire::drain_protocol`.
pub fn is_popup_x11_surface(x11: &smithay::xwayland::X11Surface) -> bool {
    use smithay::xwayland::xwm::WmWindowType;
    // The client told the X server no window manager should place it. Nothing overrides
    // that, including `WM_DELETE_WINDOW`.
    if x11.is_override_redirect() {
        return true;
    }
    match x11.window_type() {
        Some(
            WmWindowType::Menu
            | WmWindowType::PopupMenu
            | WmWindowType::DropdownMenu
            | WmWindowType::Tooltip
            | WmWindowType::Dnd
            | WmWindowType::Combo,
        ) => true,
        // A declared type that is not chrome is taken at its word.
        Some(_) => false,
        // No declared type: skipping the taskbar is the only remaining hint that this
        // belongs to another window, and a closeable window overrules it.
        None => x11.is_skip_taskbar() && !x11.supports_delete_window(),
    }
}

/// Does the window DECLARE itself transient chrome — something the user never
/// launched and will not come back to?
///
/// The X11 answer to what `xdg_wm_dialog_v1`'s modal hint says on the wayland side,
/// and it says considerably more: EWMH gives a whole `_NET_WM_WINDOW_TYPE` vocabulary,
/// and override-redirect is a declaration in its own right — the client told the X
/// server no window manager should be involved, which is not something an application
/// window says.
///
/// `Dialog` is deliberately NOT on the list. A dialog is often a real window the user
/// wants back (a preferences window, a file chooser they reopen); it is *modal*
/// dialogs that are ephemeral, and `_NET_WM_STATE_MODAL` says that directly. Same
/// reasoning for `Utility`/`Toolbar` — a detached palette is a window someone arranged.
///
/// Named for the shell on purpose. It is `false` for a wayland window — not because
/// wayland windows are never ephemeral, but because they say so by other means (the
/// `xdg_wm_dialog_v1` modal hint), and because the OTHER ephemerality signal callers
/// combine this with, a `NoDisplay=true` desktop entry, is protocol-agnostic and
/// applies to both. Spelling the shell here is what keeps that distinction legible at
/// the call site: `no_display || is_ephemeral_x11(w)` reads as "either shell's
/// desktop-entry verdict, or the X11 window's own declaration".
pub fn is_ephemeral_x11(window: &Window) -> bool {
    use smithay::xwayland::xwm::WmWindowType;
    let Some(x11) = window.x11_surface() else { return false };
    if x11.is_override_redirect() || x11.is_modal() {
        return true;
    }
    matches!(
        x11.window_type(),
        Some(
            WmWindowType::Combo
                | WmWindowType::Desktop
                | WmWindowType::Dnd
                | WmWindowType::Dock
                | WmWindowType::DropdownMenu
                | WmWindowType::Menu
                | WmWindowType::Notification
                | WmWindowType::PopupMenu
                | WmWindowType::Splash
                | WmWindowType::Tooltip
        )
    )
}

// ── X11-only properties ────────────────────────────────────────────────────────
//
// Each of the three below is the X11 half of a WAYLAND protocol y5 already reads, and
// each exists so its caller can ask for the fact without holding an `X11Surface`. The
// pattern at every call site is the same and deliberately so:
//
//     if let Some(fact) = ident::x11_<thing>(window) { ...X11 answer...; return; }
//     let surface = ident::xdg_surface(window)?;      // ...wayland answer
//
// `xdg_surface` returning `None` means "not the xdg one", never "X11 cannot answer" —
// these are the answers. See its docs for the table.

/// `_NET_STARTUP_ID` — the X11 half of `xdg_activation_v1`.
///
/// Preferred over the `/proc` environment route even though both exist: that one needs
/// `_NET_WM_PID`, a readable `/proc/<pid>/environ`, and the variable to have survived
/// into it, while this is one property the X server already tracked. The env route
/// stays as the fallback for clients that consume the variable without setting the
/// property.
pub fn x11_startup_id(window: &Window) -> Option<String> {
    window.x11_surface()?.startup_id()
}

/// `WM_WINDOW_ROLE` + `WM_CLASS` — the X11 half of `xdg_session_management_v1`, as
/// `(session_id, name)`.
///
/// `class` is the session id and `role` the name, and BOTH are required. A window with
/// no role yields `None` rather than a key with an empty name: `(class, "")` is the
/// same key for every window of an application, so the first one to map would claim
/// another's placeholder. A missing signal costs a fallback to the token/pid pass; a
/// wrong one restores the wrong window.
///
/// NOT `SM_CLIENT_ID`, which is per LOGIN session — the match is exact equality on both
/// fields, so it would pass zero matches after a reboot. The application identity is
/// the durable thing X11 has.
pub fn x11_session_key(window: &Window) -> Option<(String, String)> {
    let x11 = window.x11_surface()?;
    let name = x11.window_role()?;
    let session_id = Some(x11.class()).filter(|class| !class.is_empty())?;
    Some((session_id, name))
}

/// `_NET_WM_ICON` — the X11 half of `xdg_toplevel_icon_v1`, as the raw CARD32 words
/// the property holds (EWMH packs `width, height, w*h` ARGB pixels, possibly repeated).
///
/// Unparsed on purpose: which image a caller wants is its own decision. Pixels only —
/// X11 has no name half — so a window that sets none still resolves an icon through the
/// desktop entry its `WM_CLASS` names, which is the path every iconless window takes.
pub fn x11_icon(window: &Window) -> Option<Vec<u32>> {
    window.x11_surface()?.icon()
}
