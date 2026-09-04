//! Shell-agnostic window operations — how to SAY something to a window.
//!
//! A `smithay::desktop::Window` is either an xdg toplevel or an X11 (XWayland)
//! surface, and the two answer the same question through different protocols. An
//! xdg toplevel carries a DOUBLE-BUFFERED pending state that a later
//! `send_configure` commits; an X11 window has no pending state and is configured
//! by one immediate call carrying position AND size together.
//!
//! Everything above the wire layer only ever wants "tell this window to be this
//! big", so the split lives here and nowhere else — a `window.toplevel().unwrap()`
//! anywhere above this layer is an abort waiting for the first X11 client.
//!
//! **X11 gets the same two-step shape as xdg.** [`stage`] records the size and puts
//! the window on a dirty list; [`send`] is an xdg-only emit, and the X11 configure is
//! issued once per frame for each window [`take_staged`] hands back
//! ([`flush_pending`]), which coalesces a drag's motion events into one configure per
//! frame — an X11 configure is a request over the X socket, and a frame in which
//! nothing was staged costs nothing.
//!
//! **y5 never MOVES an X11 window** — see [`X11_ORIGIN`] and [`flush_pending`]. Every
//! toplevel is configured at the origin and a child keeps the position its client chose,
//! so a configure only ever resizes and the only geometric fact that crosses the X
//! boundary is a parent-relative DIFFERENCE.
//!
//! Note `last_configure` vs `geometry` throughout: smithay subtracts
//! `_GTK_FRAME_EXTENTS` from an X11 window's `geometry`, so that is the VISIBLE
//! rect while `configure` speaks the full X rect including shadow.

use smithay::desktop::Window;
use smithay::xwayland::xwm::WmAllowedAction;
use smithay::wayland::seat::WaylandFocus;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::utils::{Logical, Point, Rectangle, Size};
use std::sync::Mutex;

/// The size staged for an X11 window, awaiting the per-frame flush. The xdg
/// counterpart is the toplevel's own pending state.
#[derive(Debug, Default)]
struct PendingConfigure(Mutex<Option<Size<i32, Logical>>>);

/// The X11 windows with a size staged and not yet flushed — the DIRTY LIST the
/// per-frame flush drains through [`take_staged`].
///
/// Process-wide rather than a marker on each window, because [`stage`] runs from a dozen
/// call sites with no handle on the Orchestrator (the canvas drag reaches the Space
/// through the `Any`-downcast `Platform` hatch), and the alternative — walking every
/// window of every world every frame to find the marker — cost five mutex locks per idle
/// X11 window per vblank to discover nothing was staged. `Window` compares by `Arc`
/// identity, so membership is a pointer compare over a list that is almost always empty.
static STAGED: Mutex<Vec<Window>> = Mutex::new(Vec::new());

/// Stage a compositor-decided size. Nothing is emitted either way — for xdg the
/// next [`send`] does it, for X11 the next [`flush_pending`].
///
/// Returns whether the size could be staged at all, i.e. whether this window has a
/// shell that can be told anything. Callers that record a decided size ALONGSIDE this
/// — the slot, the resize throttle — should gate on it, so the compositor never
/// commits to a size it had no way to communicate. Today `WindowSurface` has exactly
/// two variants and both stage, so it is always `true`; the point is that the
/// invariant is expressed rather than assumed, since the guard it replaces
/// (`toplevel().is_none()`) had come to mean "not an xdg window" and was refusing X11
/// windows their resize.
pub fn stage(window: &Window, size: Size<i32, Logical>, resizing: bool) -> bool {
    if let Some(toplevel) = window.toplevel() {
        toplevel.with_pending_state(|state| {
            if resizing {
                state.states.set(xdg_toplevel::State::Resizing);
            }
            state.size = Some(size);
        });
        return true;
    }
    let Some(x11) = window.x11_surface() else { return false };
    // `size` is the VISIBLE size the compositor decided; an X11 configure speaks the FULL
    // rect, so the client's own frame — its CSD drop shadow, `_GTK_FRAME_EXTENTS` — is
    // added back here.
    //
    // This is what a window manager does: it decides how big the window IS, and the client
    // hangs its shadow outside that. Configuring the visible size directly handed a CSD
    // client a rect one shadow too small on every resize, and — because the slot then
    // counted the shadow while the fit's reference did not — left the difference as a
    // LETTERBOX. Firefox showed it exactly: slot 1045x641 against a visible 993x589, so
    // 26px bars left and right and 23/29 top and bottom, which is its shadow to the pixel.
    //
    // Adding it here rather than at the caller is the point of the facade: nothing above
    // this layer has to know that X11 windows carry a frame at all.
    let extents = x11.frame_extents();
    let full = Size::from((
        size.w + extents.left + extents.right,
        size.h + extents.top + extents.bottom,
    ));
    let data = window.user_data();
    data.insert_if_missing_threadsafe(PendingConfigure::default);
    *data.get::<PendingConfigure>().unwrap().0.lock().unwrap() = Some(full);
    let mut staged = STAGED.lock().unwrap_or_else(|e| e.into_inner());
    if !staged.contains(window) {
        staged.push(window.clone());
    }
    true
}

/// [`stage`] for an interactive resize drag, where the EMIT is throttled.
///
/// The two shells differ in what a stage costs. For xdg it is a write to the pending
/// state, and it must happen on EVERY motion: `send_pending_configure` runs on a focus
/// change mid-drag and reads that state, so a pending size a second stale is a stale
/// configure on the wire. For X11 staging IS the emit — the per-frame flush sends
/// whatever is staged — so staging every motion reconfigured the client every frame,
/// left no stale buffer and gave `resize_stretching` nothing to stretch. So xdg always
/// stages here and X11 only when `emit` says the throttle is open.
pub fn stage_drag(window: &Window, size: Size<i32, Logical>, emit: bool) {
    if emit || window.toplevel().is_some() {
        stage(window, size, true);
    }
}

/// The windows [`stage`] has touched since the last call, in staging order. The
/// per-frame flush (`Orchestrator::flush_x11_configures`) hands each to
/// [`flush_pending`].
pub fn take_staged() -> Vec<Window> {
    std::mem::take(&mut *STAGED.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Set on an X11 window the first time it is associated with a surface — the moment it
/// first becomes drawable, and the end of the window in which it may still size itself.
#[derive(Debug)]
struct Shown;

/// The user operations y5 will actually perform for an X11 window, published as
/// `_NET_WM_ALLOWED_ACTIONS` so a client can grey out what it cannot ask for.
///
/// The list is short because y5 is not a stacking WM with a window menu. Close is
/// `WM_DELETE_WINDOW` ([`close`]); fullscreen is honoured through
/// `XwmHandler::fullscreen_request`. Everything else — move, resize, minimize, maximize,
/// shade, stick, change-desktop — is either refused by design (the move/resize grabs, as
/// on the wayland side) or has no y5 concept behind it, and the matching `XwmHandler`
/// method is a stub or an unimplemented default. Saying so is the point: silence is
/// indistinguishable from a lost message.
const ALLOWED_ACTIONS: [WmAllowedAction; 2] = [WmAllowedAction::Fullscreen, WmAllowedAction::Close];

/// Publish [`ALLOWED_ACTIONS`], and open the window's self-sizing period.
///
/// Called once per X11 window as it becomes managed. Override-redirect windows are not
/// managed by definition and are not given the property.
pub fn declare_managed(window: &smithay::xwayland::X11Surface) {
    if window.is_override_redirect() {
        return;
    }
    if let Err(err) = window.set_allowed_actions(&ALLOWED_ACTIONS) {
        warn!("x11 set_allowed_actions failed: {err:?}");
    }
}

/// Close the self-sizing period: this window has been shown at least once.
///
/// Idempotent and FIRST-association only — a remap does not reopen it, which is the
/// point. See [`may_self_size`].
pub fn mark_shown(window: &smithay::xwayland::X11Surface) {
    window.user_data().insert_if_missing_threadsafe(|| Shown);
}

/// May this X11 window still choose its own size through a `ConfigureRequest`?
///
/// **Only before it has first been shown, and always for a child.** X11 has a request
/// wayland simply does not — a client asking to change its own geometry — and honouring
/// it for the life of the window is the one place an X client could do something an xdg
/// one cannot. So the answer is narrowed to the period that has a wayland equivalent:
/// the settling before first map, where an X client states how big it wants to be and
/// [`configured_size`] reads it straight back (`slot::set_expected_auto` is the same
/// "follow the client until the compositor decides" phase on the y5 side).
///
/// After that the compositor owns the size, and granting the request would desync
/// `last_configure` from what [`stage`] has staged — the value [`flush_pending`] diffs
/// against — for no visible effect, since a `Decided` slot letterboxes the client's own
/// geometry anyway.
///
/// A CHILD keeps the right for its whole life: its slot stays `Auto` ([`has_parent`] —
/// "such windows size themselves"), and a menu that grows as it fills has nothing else
/// to ask with.
///
/// The two arms mirror what the slot already does for xdg, which is why this is parity
/// and not a new rule: `on_window_map_initial` calls `set_expected_size` on a toplevel at
/// initial map (Decided from then on, client size letterboxed) and `set_expected_auto` on
/// a parented one (follows the client for good).
///
/// Override-redirect windows are not tested because they never arrive here: the X server
/// does not route a `ConfigureRequest` through the window manager for a window that
/// declared itself unmanaged. They resize themselves directly, and y5 learns of it in
/// `configure_notify`.
pub fn may_self_size(window: &smithay::xwayland::X11Surface) -> bool {
    window.is_transient_for().is_some() || window.user_data().get::<Shown>().is_none()
}

/// A map request that could not be queued yet, because the popup/window decision needs a
/// `wl_surface` the window does not have.
///
/// An `AtomicBool` rather than a bare marker because it is CONSUMED: one map request
/// arms it, the association that follows takes it, and a remap arms it again. A marker
/// that only ever went on would let any later association queue a map of its own.
#[derive(Debug)]
struct HeldForSurface(std::sync::atomic::AtomicBool);

/// Record that this window's map is waiting on its surface. See [`take_held_map`].
///
/// The point of latching rather than re-deciding: `ident::is_popup_x11_surface` reads
/// `_NET_WM_WINDOW_TYPE`, `_NET_WM_STATE_SKIP_TASKBAR` and `WM_PROTOCOLS`, all ordinary
/// properties a client may change at any time and smithay re-reads on `PropertyNotify`.
/// Asking it once at the map request and again at the association is asking a question
/// whose answer can differ between the two — and the direction that hurts is silent: held
/// at map, declined at association, so nothing ever queues the window and it never maps.
pub fn hold_map_for_surface(window: &smithay::xwayland::X11Surface) {
    use std::sync::atomic::Ordering;
    window
        .user_data()
        .insert_if_missing_threadsafe(|| HeldForSurface(std::sync::atomic::AtomicBool::new(false)));
    if let Some(held) = window.user_data().get::<HeldForSurface>() {
        held.0.store(true, Ordering::Relaxed);
    }
}

/// Take the held map, if there is one — true at most ONCE per [`hold_map_for_surface`].
///
/// This is also what keeps an association from queueing a map on its own. Association is
/// driven by a `WL_SURFACE_SERIAL` client message (or the reverse-order commit hook) and
/// checks nothing about map state, so without the latch a client that associates a
/// surface and never maps its window would still be presented as a popup.
pub fn take_held_map(window: &smithay::xwayland::X11Surface) -> bool {
    use std::sync::atomic::Ordering;
    window
        .user_data()
        .get::<HeldForSurface>()
        .is_some_and(|held| held.0.swap(false, Ordering::Relaxed))
}

/// A tearing-target verdict reached before the window had a surface to carry it.
/// Carries WHOSE verdict it was — `Y5_TEARING` and the Steam guess land in different
/// halves of the tag and rank differently against the client's own hint.
#[derive(Debug)]
struct PendingTearingTarget(bool);

/// Stamp the tearing-target marker, surviving an X11 window that has no surface yet.
///
/// The verdict is resolved ONCE, at map, because the heuristic behind it walks `/proc` and
/// the process tree — far too heavy for the commit path where the marker is read. But an
/// X11 window can be mapped before it has a `wl_surface`: Xwayland associates one after
/// the window exists, and `map_window_request` queues the map immediately. So for exactly
/// the windows this matters for — Steam titles under XWayland — the stamp could arrive
/// with nowhere to go, and since the verdict is never revisited it was lost for good. The
/// window looked identical to one the heuristic had rejected.
///
/// Recorded on the `X11Surface` in that case, and promoted by
/// [`promote_tearing_target`] when the association lands. Not on the `Window`'s user data:
/// the association callback has the `X11Surface` and the new surface, and no `Window` at
/// all.
pub fn mark_tearing_target(window: &Window, forced: bool) {
    if let Some(surface) = window.wl_surface() {
        stamp_tearing_target(&surface, forced);
        return;
    }
    if let Some(x11) = window.x11_surface() {
        x11.user_data().insert_if_missing_threadsafe(|| PendingTearingTarget(forced));
    }
}

/// Carry a verdict recorded before the surface existed onto the surface now that it does.
///
/// Called from the association callback. A no-op for the ordinary case, where the window
/// already had a surface when it was tagged.
pub fn promote_tearing_target(
    x11: &smithay::xwayland::X11Surface,
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
) {
    if let Some(pending) = x11.user_data().get::<PendingTearingTarget>() {
        stamp_tearing_target(surface, pending.0);
    }
}

fn stamp_tearing_target(
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    forced: bool,
) {
    // The COMPOSITOR's halves only. `wp_tearing_control_v1` writes the client's through
    // `set_hint` and the two never touch, which is what keeps a hint from erasing this
    // verdict — the bug from when both shared one flag.
    //
    // `forced` (`Y5_TEARING`) is a separate field from the guess rather than the same
    // one set harder, because they rank on opposite sides of the client's hint.
    use compositor_support_smithay_state_tearing_pacer::pacer::TearingTag;
    smithay::wayland::compositor::with_states(surface, |states| {
        states.data_map.insert_if_missing(TearingTag::default);
        if let Some(tag) = states.data_map.get::<TearingTag>() {
            if forced {
                tag.set_forced(true);
            } else {
                tag.set_heuristic(true);
            }
        }
    });
}

/// Can this window be told a size at all?
///
/// The question [`stage`] answers as a side effect of doing it, asked on its own — for a
/// caller that must decide whether to stage at all, such as one throttling an interactive
/// resize. Today both `WindowSurface` variants can be told, so it is always `true`; the
/// point is that the invariant is expressed rather than assumed, since the guard it
/// replaces (`toplevel().is_some()`) had come to mean "not an xdg window" and refused X11
/// windows their resize.
pub fn can_stage(window: &Window) -> bool {
    window.toplevel().is_some() || window.x11_surface().is_some()
}

/// Clear the `Resizing` state an interactive drag set. X11 has no such state — the
/// resize is expressed entirely by the geometry the flush sends.
pub fn unstage_resizing(window: &Window) {
    let Some(toplevel) = window.toplevel() else { return };
    toplevel.with_pending_state(|state| {
        state.states.unset(xdg_toplevel::State::Resizing);
    });
}

/// Emit the configuration staged above.
///
/// X11 is a deliberate no-op: its configure carries a position the caller does not
/// have, and [`flush_pending`] issues it once per frame from the Space instead.
pub fn send(window: &Window) {
    if let Some(toplevel) = window.toplevel() {
        toplevel.send_configure();
    }
}

/// Emit a configure only if something changed (activation, suspend, decoration
/// mode). X11 writes those as properties, so this is an xdg-only concern.
pub fn send_pending(window: &Window) {
    if let Some(toplevel) = window.toplevel() {
        toplevel.send_pending_configure();
    }
}

/// Where every X11 TOPLEVEL is told it is — one place, for all of them.
///
/// **X space is not a map of the canvas, and it does not have to be.** It exists for one
/// purpose: Xwayland builds a pointer event's root coordinate as `window position +
/// surface-local`, and X clients reason in that space. What they reason about is their OWN
/// window group — a menu grabs the pointer and tracks it against its parent, a drag runs
/// within one client. Nothing reasons across two unrelated applications' windows, so
/// nothing needs their coordinates to relate.
///
/// So toplevels all sit here, and children are left exactly where their client put them:
/// the client positions a menu against its parent, the parent is at the origin, and the
/// offset between them is therefore true. That is the only relationship that has to hold,
/// and it holds by construction.
///
/// This is xwayland-satellite's layout, and reading it is what settled the question. Its
/// `server/event.rs` computes a window's X position from the xdg configure — which for a
/// toplevel carries no position at all, so it is `0`, with a `.max(0)` clamping anything
/// negative regardless. Only popups get a non-zero value, from their parent-relative
/// positioner.
///
/// **It also removes a whole class of problem rather than managing it.** Placing windows
/// at distinct positions needs a coordinate space large enough to hold them all, and X
/// window coordinates are `INT16` while the canvas is unbounded — so any spread-out scheme
/// needs a bound, an anchor, re-anchoring when windows drift out of it, and a screen size
/// to measure against. All of that was built here and is now gone: the space required is
/// one window plus its menus, whatever the canvas holds.
///
/// The cost is that toplevels overlap in X, so which one an ungrabbed pointer event
/// reaches is settled by the X STACK — `Dispatch::raise_x11_for_pointer` raises the window
/// being entered, before the enter is sent. satellite does the same thing for the same
/// reason (`raise_to_top` in its `wl_pointer.enter` handler).
const X11_ORIGIN: Point<i32, Logical> = Point::new(0, 0);

/// Issue the X11 configure staged for `window`, if its rect differs from the one the X
/// server was last given. Returns whether anything was sent.
///
/// The position is [`X11_ORIGIN`] for a toplevel and the client's own for a child — see
/// that constant for why one place is enough. Called from the housekeeping flush for each
/// window [`take_staged`] hands back, once per frame.
///
/// The diff is against smithay's own `last_configure`: the rect the server was last given
/// by ANY route. `configure` writes it, and so does the client's own `ConfigureRequest`
/// when `XwmHandler::configure_request` grants a size. A private "last sent" cell here
/// did not see that second route, so after a client asked for 1000x700 a re-stage of the
/// 800x600 y5 had sent before compared equal to the stale cell and was silently skipped —
/// the client stayed at its size while the slot assumed y5's.
pub fn flush_pending(window: &Window) -> bool {
    let Some(x11) = window.x11_surface() else { return false };
    let staged = window
        .user_data()
        .get::<PendingConfigure>()
        .and_then(|p| p.0.lock().unwrap().take());
    let Some(size) = staged else { return false };
    // Staged and then destroyed before the frame: nothing to tell, and `configure` on a
    // dead surface only produces a warning.
    if !x11.alive() {
        return false;
    }
    // A CHILD keeps whatever position its client chose: the client placed it against its
    // parent, the parent is at the origin, so that offset is already true. Only toplevels
    // are moved, and only to the one place they all share.
    let at = match x11.is_transient_for().is_some() || x11.is_override_redirect() {
        true => x11.last_configure().loc,
        false => X11_ORIGIN,
    };
    let rect = Rectangle::new(at, size);
    if x11.last_configure() == rect {
        return false;
    }
    trace!("x11 configure window={} rect={rect:?}", x11.window_id());
    // Override-redirect windows go through the explicit escape hatch. `configure` refuses
    // them — such a window told the X server no window manager may position it — and y5
    // overrides that on purpose, because it places them itself like everything else.
    let sent = match x11.is_override_redirect() {
        true => x11.configure_override_redirect(rect),
        false => x11.configure(rect),
    };
    if let Err(err) = sent {
        warn!("x11 configure failed: {err:?}");
        return false;
    }
    true
}

/// Ask the client to dismiss just this window — the title-bar X, not the process.
///
/// `false` means nothing was asked and the caller must escalate (kill the pid); it
/// does NOT mean "no close protocol". An X11 window that does not list
/// `WM_DELETE_WINDOW` is destroyed outright instead — what every X window manager
/// does for the title-bar X — so the close still happened.
pub fn close(window: &Window) -> bool {
    if let Some(toplevel) = window.toplevel() {
        toplevel.send_close();
        return true;
    }
    let Some(x11) = window.x11_surface() else { return false };
    match x11.close() {
        Ok(()) => true,
        Err(err) => {
            warn!("x11 close failed: {err:?}");
            false
        }
    }
}

/// Write the protocol's fullscreen flag. xdg stages it for the configure that
/// follows; X11 has no configure to ride on — `_NET_WM_STATE_FULLSCREEN` is a
/// property, set immediately and separately from the geometry.
pub fn set_fullscreen(window: &Window, fullscreen: bool) {
    if let Some(x11) = window.x11_surface() {
        if let Err(err) = x11.set_fullscreen(fullscreen) {
            warn!("x11 set_fullscreen failed: {err:?}");
        }
        return;
    }
    let Some(toplevel) = window.toplevel() else { return };
    toplevel.with_pending_state(|state| match fullscreen {
        true => state.states.set(xdg_toplevel::State::Fullscreen),
        false => state.states.unset(xdg_toplevel::State::Fullscreen),
    });
}

/// "This window is not being presented, stop painting it."
///
/// xdg spells it `Suspended`, and needs a [`send_pending`] afterwards. Clients below
/// xdg_wm_base v6 never see the state — smithay filters it per client version
/// (`into_filtered_states`) — so nothing to gate here.
///
/// **X11 is deliberately a NO-OP, and it is the one facade arm with nothing on the other
/// side.** `_NET_WM_STATE_HIDDEN` is the nearest atom and it does not mean this, for two
/// independent reasons.
///
/// By the spec, it is for a window that "would not be visible on the screen if its
/// desktop/viewport were active AND its coordinates were within the screen bounds". That
/// counterfactual factors out position and desktop on purpose — so a window on an
/// inactive desktop is not hidden, and one scrolled off the viewport is not hidden. Those
/// are exactly and only the cases y5 suspends for (`Orchestrator::refresh_suspended`:
/// parked world, collapsed group, screen locked, scrolled away). Every one of them WOULD
/// be visible if the user panned back or switched worlds, which is the whole point of a
/// canvas.
///
/// And in this fork the atom simply IS `Iconic`, in both directions: `set_mapped` derives
/// `WM_STATE = Iconic` from it, and MapRequest derives it from
/// `WM_HINTS.initial_state == Iconic` (both `xwm/surface.rs` + `xwm/mod.rs`). So the flag
/// latches, any later map of a suspended window declares it MINIMIZED, and GTK/Qt read it
/// as iconified/`WindowMinimized`. The only lever it has on a client is therefore a FALSE
/// minimize — which is exactly the failure it would be bought with: five seconds off-pane
/// (`SUSPEND_DWELL`) turned a game black until it was focused again.
///
/// X's real equivalent is `VisibilityNotify` (`VisibilityFullyObscured`), which says
/// precisely this and nothing more — but it is generated by the SERVER from the window
/// hierarchy and cannot be asserted by a window manager, and `XSendEvent` is flagged
/// synthetic so toolkits ignore it (the same wall the pointer path hits). What the server
/// would compute is meaningless here anyway: every toplevel sits at `X11_ORIGIN`, so
/// visibility would follow the X STACK — which the pointer path restacks on every
/// crossing — and would flip as the mouse moved rather than as the canvas panned.
///
/// Nothing is lost by staying quiet, because the saving does not come from this flag.
/// Frame callbacks go only to the drawn set (`present.callbacks::send_window_frames`
/// over `vis.drawn`), so an off-pane client that paces on them stalls on its own, with
/// no claim to retract when the user pans back. That reaches X clients too: Xwayland
/// backs GLX/EGL swaps and the Present extension with frame callbacks on the window's
/// `wl_surface`, so an off-pane X client blocks in `glXSwapBuffers` exactly as a wayland
/// one blocks in its commit loop. What it leaves uncovered is a client that paces on
/// neither — and that is the same client that would have gone black and stayed there.
///
/// So the invariant is: y5 NEVER writes `_NET_WM_STATE_HIDDEN`, and it means only what
/// the client said. Nothing is stranded by not clearing it either — smithay rebuilds the
/// flag from `WM_HINTS.initial_state` on every MapRequest, inserting for `Iconic` and
/// REMOVING otherwise, so a client that is not asking to start minimized has it cleared
/// for it. (A client that IS asking still keeps it latched while y5 maps and shows the
/// window anyway — a pre-existing gap in not honouring iconic-at-map, not this arm's.)
pub fn set_suspended(window: &Window, suspended: bool) {
    let Some(toplevel) = window.toplevel() else { return };
    toplevel.with_pending_state(|state| match suspended {
        true => state.states.set(xdg_toplevel::State::Suspended),
        false => state.states.unset(xdg_toplevel::State::Suspended),
    });
}

/// Does this window declare itself subordinate to another one — an xdg `parent`, or
/// an X11 `WM_TRANSIENT_FOR`?
///
/// Such windows size themselves, so the initial-map path leaves their expected size
/// `Auto` instead of locking it to whatever they first mapped at.
pub fn has_parent(window: &Window) -> bool {
    if let Some(toplevel) = window.toplevel() {
        return toplevel.parent().is_some();
    }
    window.x11_surface().is_some_and(|x11| x11.is_transient_for().is_some())
}

/// The size to treat as this window's compositor-decided slot: the VISIBLE size, for
/// either shell.
///
/// **The measure any convergence test must use**, and the one the render/input fit takes
/// as its reference — those two agreeing is what keeps a window from being letterboxed by
/// its own frame.
///
/// It used to answer `last_configure().size` for X11, which is the FULL rect including
/// the client's CSD shadow, on the reasoning that configuring `geometry()` would hand the
/// client a rect one shadow too small. That reasoning was right about the CONFIGURE and
/// wrong about the SLOT: [`stage`] now adds the frame extents back when it configures, so
/// the two roles are separated and this can be the visible size that everything else
/// measures in. Leaving it as the full rect meant the slot counted the shadow while the
/// fit's reference (`geometry()`) did not, and the difference showed as a letterbox —
/// 26px bars around Firefox, its shadow to the pixel — and, in
/// `compositor.place::reassert_size_if_diverged`, as a comparison that could never
/// converge.
pub fn configured_size(window: &Window) -> Size<i32, Logical> {
    window.geometry().size
}
