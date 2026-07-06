#[macro_use]
extern crate compositor_developer_debug_instance_record;

use std::sync::Mutex;
use std::time::{Duration, Instant};
use smithay::desktop::{PopupKind, Window};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Size};
use compositor_support_smithay_dispatch_state_base::state::{Dispatch, DispatchWire};

/// Marks a window whose initial Space placement is done (set by `apply_commit`; read by the drain
/// and the canvas) so re-commits don't re-place.
pub struct WindowPlacedMarker;

/// Grace during which the compositor forces a client that re-sizes ITSELF at startup (e.g. QEMU a
/// second into boot) back to the decided size. A plain re-send is a client no-op, so we nudge with
/// a `+1px` change it can't ignore, then settle back on the next commit — net zero, no drift.
pub const INITIAL_SIZE_GRACE: Duration = Duration::from_secs(5);

/// Give up after this many nudge attempts, so a client that keeps rejecting `decided` isn't cycled
/// nudge→settle for the whole grace — "up to 3 chances".
const MAX_ATTEMPTS: u8 = 3;

struct SizeGrace {
    decided: Size<i32, Logical>,   // fixed size the client should end at (never drifts)
    last_seen: Size<i32, Logical>, // last client size reacted to (edge-trigger)
    deadline: Instant,
    nudged: bool,                  // true after the +1 nudge; next commit settles back to decided
    attempts: u8,                  // step-1 nudges so far, capped at MAX_ATTEMPTS
}

#[derive(Default)]
pub struct PendingSizePropagation(Mutex<Option<SizeGrace>>);

/// Arm the grace at map/restore. `size` is the decided size the client is held to for
/// `INITIAL_SIZE_GRACE`. Any explicit resize supersedes it (disarm).
pub fn arm_size_propagation(window: &Window, size: Size<i32, Logical>) {
    window.user_data().insert_if_missing_threadsafe(PendingSizePropagation::default);
    if let Some(p) = window.user_data().get::<PendingSizePropagation>() {
        *p.0.lock().unwrap() = Some(SizeGrace {
            decided: size,
            last_seen: size,
            deadline: Instant::now() + INITIAL_SIZE_GRACE,
            nudged: false,
            attempts: 0,
        });
    }
}

/// Clear the grace — an explicit sizing decision supersedes it.
pub fn disarm_size_propagation(window: &Window) {
    if let Some(p) = window.user_data().get::<PendingSizePropagation>() {
        *p.0.lock().unwrap() = None;
    }
}

/// Consume (per commit): two-step jiggle, edge-triggered on the client's size changes. Step 1 — if
/// the client committed a size that isn't `decided`, nudge with `decided + 1px`; step 2 — next
/// commit, settle back to `decided` (net zero). Gives up after `MAX_ATTEMPTS`; self-clears at deadline.
pub fn reassert_size_if_diverged(window: &Window) -> Option<Size<i32, Logical>> {
    let p = window.user_data().get::<PendingSizePropagation>()?;
    let mut guard = p.0.lock().unwrap();
    let g = guard.as_mut()?;
    if Instant::now() >= g.deadline { *guard = None; return None; }
    let cur = window.geometry().size;
    if cur == g.last_seen { return None; }
    g.last_seen = cur;
    if g.nudged {
        g.nudged = false;
        return Some(g.decided);
    }
    if cur != g.decided {
        if g.attempts >= MAX_ATTEMPTS { *guard = None; return None; }
        g.attempts += 1;
        g.nudged = true;
        return Some(Size::from((g.decided.w + 1, g.decided.h + 1)));
    }
    None
}

/// Popup commit — PROTOCOL only (Dispatch). Toplevel/window placement moved to the world-side
/// `apply_commit` (document/SMITHAY_DECOUPLING.md): `commit` must not touch the world.
pub fn handle_commit(
    _loop: &mut Dispatch,
    surface: &WlSurface,
) {
    _loop.popup.state.commit(surface);
    if let Some(popup) = _loop.popup.state.find_popup(surface) {
        match popup {
            PopupKind::Xdg(ref xdg) => {
                if !xdg.is_initial_configure_sent() {
                    xdg.send_configure()
                        .unwrap_or_else(|e| abort!("initial configure failed: {e:?}"));
                }
            }
            PopupKind::InputMethod(ref _input_method) => {}
        }
    }
}
