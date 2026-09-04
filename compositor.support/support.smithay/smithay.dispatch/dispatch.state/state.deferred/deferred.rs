//! One ordered queue for the deferred effects a protocol handler records.
//!
//! Every smithay handler in `dispatch.state` runs WORLD-FREE — it can see the surface
//! in front of it and nothing else — so an effect on a window, a layer or the pointer
//! is recorded here and applied against the Space by `Wire::drain_protocol`.
//!
//! ONE queue rather than one per kind, because these are ordered signals about the same
//! subject matter and separate queues can only be drained in a fixed sequence. That
//! sequence then decides the outcome instead of the order things actually happened:
//! six `mem::take`s in source order meant a window mapped after another was destroyed
//! got applied first simply because the map loop was written above the destroy loop.
//! Arrival order is the only ordering that is a fact rather than an artefact of layout.
//!
//! What is deliberately NOT here: `committed` (its own hot path, one `WlSurface` per
//! entry, and the drain needs it in two passes), and `pending_dmabuf` /
//! `pending_blockers`, which are unrelated machinery that merely share the deferral —
//! and whose payloads (`Dmabuf`, `DrmSyncPointSource`) would be paid for by every
//! element of this `Vec`, since an enum is as large as its widest variant.

use smithay::desktop::Window;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::shell::wlr_layer::{Layer as WlrLayer, LayerSurface};
use compositor_support_smithay_state_window_find::find::Shell;

/// What a map event hands the drain.
///
/// The asymmetry is the point, and it is the rule [`Shell`] states: a handler may queue a
/// `Window` only when it is the one place that can build one.
///
/// `XdgShellHandler::new_toplevel` is such a place — handed a `ToplevelSurface` that
/// exists nowhere else, firing exactly once per toplevel. The X11 handlers are not: they
/// hold an `X11Surface`, whose `window_id()` is already the window's identity, and they
/// can fire more than once for one window (override-redirect is mutable and smithay
/// re-reads it at `MapNotify`). `Window::new_x11_window` mints a fresh identity on every
/// call, so building one in a world-free handler would give one `X11Surface` two Space
/// elements sharing a `wl_surface` — content drawn twice. The drain can see the Space, so
/// it resolves identity there.
pub enum Mapped {
    /// An xdg toplevel, with the `Window` only its handler could build.
    Xdg(Window),
    /// An X11 window, identified by its surface and resolved at drain.
    X11(smithay::xwayland::X11Surface),
}

/// A world effect a handler recorded, to be applied in the order it arrived.
pub enum Deferred {
    /// A window asked to be mapped — an xdg toplevel, a managed X11 window, or an X11
    /// override-redirect surface. See [`Mapped`] for why only the xdg arm carries a
    /// constructed `Window`.
    WindowMapped(Mapped),
    /// An X11 window WITHDREW: unmapped, but the X window still exists and may be
    /// mapped again (ICCCM's `Withdrawn` state).
    ///
    /// It leaves the Space, because Space membership means "this window exists and can
    /// be shown" and a withdrawn one cannot — it has no `wl_surface`, is not drawn and
    /// is not hit. Keeping it there made every consumer that trusts `elements()` wrong
    /// one at a time (the hit test, the navigator, the decoration, which drew a border
    /// around nothing for a closed Steam).
    ///
    /// Its IDENTITY is kept instead, off to the side, because withdraw and close are the
    /// same operation in X11 and only a later `DestroyNotify` says which it was. A remap
    /// resolves the same uuid, position and world; a destroy retires them.
    WindowWithdrawn(smithay::xwayland::X11Surface),
    /// A window was destroyed. An X11 UNMAP is deliberately not this — see
    /// [`Self::WindowWithdrawn`]; only `DestroyNotify` and the X server dying queue it.
    ///
    /// `drag_discard` is "was this toplevel being carried by a live
    /// `xdg_toplevel_drag_v1` when it died", sampled at destroy time rather than at
    /// drain time, since the drag may well have ended by then.
    WindowDestroyed { window: Shell, drag_discard: bool },
    /// A (un)fullscreen request from the client.
    WindowFullscreen { window: Shell, on: bool },
    /// A layer surface was created. A `None` output means "the monitor the user is on",
    /// which only the rim can resolve.
    LayerMapped { surface: LayerSurface, output: Option<WlOutput>, layer: WlrLayer, namespace: String },
    LayerDestroyed(LayerSurface),
    /// A pointer-constraint restoration token: warp the pointer back to `at` once the
    /// constraint it belongs to is no longer holding it.
    PointerRestore { surface: WlSurface, at: Point<f64, Logical> },
}
