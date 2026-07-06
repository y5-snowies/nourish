//! Per-window **expected size** — the size the COMPOSITOR has decided for a window and
//! enforces. y5's policy is that windows never resize themselves: the compositor decides the
//! size at map and on resize/tile, and any client content of a different size is letterboxed
//! into this slot (see the authoritative-sizing plan).
//!
//! Stored in the window's `UserDataMap`. `None` means "no decided size yet" (e.g. the
//! compositor deliberately deferred with a 0x0 configure) → callers render natively, no fit.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use smithay::desktop::Window;
use smithay::utils::{Logical, Size};

/// A window's slot state.
#[derive(Debug, Clone, Copy)]
enum Slot {
    /// The compositor has not yet made an explicit sizing decision for this window, so the
    /// slot **follows the client's committed `geometry()`**. This is the pre-decision phase
    /// (map → first `reform`/tile/fullscreen): the compositor sends no constraining configure
    /// here, so the client is already free to pick its size. Tracking (rather than snapshotting
    /// the first frame) makes a window that finalizes its geometry *after* its first mapped
    /// buffer — sets `window_geometry` to exclude its CSD shadow, applies a saved/relaid-out
    /// size — fill its frame, instead of being covered & cropped at the stale first-frame size
    /// (the "needs a nudge to occupy its frame" symptom). The instant the compositor decides
    /// (`set_expected_size`), the slot freezes to `Decided` and is enforced from then on.
    Auto,
    /// The compositor-decided size; enforced — content of a different size is letterboxed/covered.
    Decided(Size<i32, Logical>),
}

#[derive(Debug, Default)]
pub struct ExpectedSize(Mutex<Option<Slot>>);

/// Max interval between `send_configure`s during a continuous resize drag. Long on purpose: the
/// render follows the cursor by **stretching** the current buffer, so we avoid poking the client
/// mid-drag (each configure makes it re-render at a new size — for heavy apps like Firefox/Chrome
/// that's a visible jump). The real size is applied immediately on release (`finish_resize`); this
/// only bounds the stretch distortion on an unusually long (> 1 s) drag.
pub const RESIZE_CONFIGURE_THROTTLE: Duration = Duration::from_millis(1000);
/// **Safety net only.** After the gesture ends the stretch normally clears the instant the client
/// commits the target size (`resize_stretching` settle path) — which self-adapts to each app's
/// commit lag (heavy apps like Firefox/Chrome stretch longer, snappy apps clear at once). This
/// caps how long we wait if the client never commits (frozen / clamps to a different size), so a
/// stale buffer can't stay stretched forever.
pub const RESIZE_PENDING_TIMEOUT: Duration = Duration::from_millis(1000);

/// A resize the compositor has decided but whose matching client buffer hasn't arrived yet.
/// While set (and live), the window is rendered **stretched** to `target` so it follows the
/// cursor smoothly. `last_configure` drives the send throttle.
/// How long a settling resize's content must stay stable before we settle (clear the stretch).
/// Serves two purposes: detects a clamped client (committed a non-target size and stopped), and
/// holds the stretch a few frames past the geometry match to cover the blank/no-content frames
/// some apps commit right after acking (so the background doesn't flash through).
pub const RESIZE_SETTLE_STABLE: Duration = Duration::from_millis(120);

#[derive(Debug)]
struct ResizePending {
    target: Size<i32, Logical>,
    deadline: Instant,
    last_configure: Instant,
    /// `None` while the gesture is live (stretch held unconditionally — no flicker as the client
    /// catches intermediate sizes); `Some(t)` after the gesture ends (resize-end flush), `t` being
    /// when it ended. While settling the stretch holds until the client's content reaches the
    /// target (adapts to commit lag), OR until it settles stable at a non-target size (clamped).
    settle_at: Option<Instant>,
    /// Last content (`geometry().size`) observed, and when it last changed — to detect a clamped
    /// client that committed a NEW non-target size after the gesture ended and then went stable
    /// (vs a merely-slow client whose old buffer is trivially stable but will still commit).
    last_content: Size<i32, Logical>,
    last_content_change: Instant,
}

#[derive(Debug, Default)]
pub struct ResizeState(Mutex<Option<ResizePending>>);

/// The window's current slot size, if any. `Decided` returns the compositor's enforced size;
/// `Auto` (no decision yet) follows the client's live `geometry()` so the window fills its frame
/// as the client finalizes its geometry; unset / 0x0 geometry returns `None` (render natively).
pub fn expected_size(window: &Window) -> Option<Size<i32, Logical>> {
    match window
        .user_data()
        .get::<ExpectedSize>()
        .and_then(|e| *e.0.lock().unwrap())
    {
        Some(Slot::Decided(size)) => Some(size),
        Some(Slot::Auto) => {
            let g = window.geometry().size;
            (g.w > 0 && g.h > 0).then_some(g)
        }
        None => None,
    }
}

/// Put `window` in `Auto`: the slot follows the client's committed `geometry()` until the
/// compositor makes its first explicit sizing decision (`set_expected_size`). Used at initial
/// map — accept the client's size while it settles, without freezing the stale first frame.
pub fn set_expected_auto(window: &Window) {
    window.user_data().insert_if_missing_threadsafe(ExpectedSize::default);
    if let Some(e) = window.user_data().get::<ExpectedSize>() {
        *e.0.lock().unwrap() = Some(Slot::Auto);
    }
}

/// Record the compositor-decided size for `window` — freezes the slot to `Decided` and enforces
/// it from now on (the window is no longer in `Auto` / client-follows mode).
pub fn set_expected_size(window: &Window, size: Size<i32, Logical>) {
    window.user_data().insert_if_missing_threadsafe(ExpectedSize::default);
    if let Some(e) = window.user_data().get::<ExpectedSize>() {
        *e.0.lock().unwrap() = Some(Slot::Decided(size));
    }
    // An explicit sizing decision supersedes the startup-grace jiggle (map/restore) — else a stale
    // grace target would fight this size on a later commit (`apply_commit` consume path).
    compositor_support_smithay_state_compositor_place::disarm_size_propagation(window);
}

/// Note that the compositor has decided `target` for `window` as part of an in-flight resize.
/// Returns `true` if a `send_configure` is due now (the throttle has elapsed, or this is a new
/// resize); the caller sends and need not call again until the throttle passes. The deadline is
/// (re)armed each call, so the stretch stays live for the whole drag.
pub fn note_resize(window: &Window, target: Size<i32, Logical>) -> bool {
    window.user_data().insert_if_missing_threadsafe(ResizeState::default);
    let Some(rs) = window.user_data().get::<ResizeState>() else { return true };
    let now = Instant::now();
    let mut guard = rs.0.lock().unwrap();
    let due = match guard.as_ref() {
        Some(p) => now.duration_since(p.last_configure) >= RESIZE_CONFIGURE_THROTTLE,
        None => true,
    };
    // Preserve the content-change tracking across the (per-motion) re-arm; reset settle_at to live.
    let (last_content, last_content_change) = guard
        .as_ref()
        .map(|p| (p.last_content, p.last_content_change))
        .unwrap_or((target, now));
    *guard = Some(ResizePending {
        target,
        deadline: now + RESIZE_PENDING_TIMEOUT,
        last_configure: if due {
            now
        } else {
            guard.as_ref().map(|p| p.last_configure).unwrap_or(now)
        },
        settle_at: None,
        last_content,
        last_content_change,
    });
    due
}

/// During a LIVE resize gesture (not yet released), report whether a debounced `send_configure`
/// is due now (the throttle interval has elapsed). Lets a per-frame tick emit the configure
/// automatically — e.g. during a mid-drag pause with no pointer motion — instead of waiting for
/// the next motion or release. Updates `last_configure` when it returns a size.
pub fn resize_due(window: &Window) -> Option<Size<i32, Logical>> {
    let rs = window.user_data().get::<ResizeState>()?;
    let mut guard = rs.0.lock().unwrap();
    let p = guard.as_mut()?;
    if p.settle_at.is_some() {
        return None; // gesture ended — the final size was already flushed on release
    }
    let now = Instant::now();
    if now.duration_since(p.last_configure) >= RESIZE_CONFIGURE_THROTTLE {
        p.last_configure = now;
        Some(p.target)
    } else {
        None
    }
}

/// Mark the resize as settling — the gesture has ended (resize-end flush sent the final size), so
/// from now the stretch is held only until the client commits the target (or settles stable at a
/// clamped size, or the safety timeout).
pub fn mark_resize_settling(window: &Window) {
    if let Some(rs) = window.user_data().get::<ResizeState>() {
        if let Some(p) = rs.0.lock().unwrap().as_mut() {
            p.settle_at = Some(Instant::now());
        }
    }
}

/// Whether `window` should currently render **stretched** to its slot. The stretch references the
/// **geometry** (it fills the slot, and is the identity once the client commits the new size ==
/// the steady-state `cover` fit), so holding it on fills the slot continuously with no
/// "smaller than the slot" gap. Held UNCONDITIONALLY during the live gesture (no flicker as the
/// client catches intermediate sizes); after the gesture ends (`settling`), held until the
/// client's `content` (`geometry().size`) reaches the target — i.e. until the resized buffer
/// lands, which self-adapts to the app's commit lag. The `RESIZE_PENDING_TIMEOUT` deadline is only
/// a frozen-client safety net. Lazily clears (render + input both call this) so they converge.
pub fn resize_stretching(window: &Window, content: Size<i32, Logical>) -> bool {
    let Some(rs) = window.user_data().get::<ResizeState>() else { return false };
    let mut guard = rs.0.lock().unwrap();
    let now = Instant::now();
    let clear = {
        let Some(p) = guard.as_mut() else { return false };
        // Track content (geometry) changes.
        if (content.w - p.last_content.w).abs() > 2 || (content.h - p.last_content.h).abs() > 2 {
            p.last_content = content;
            p.last_content_change = now;
        }
        match p.settle_at {
            // Live gesture: hold unconditionally — NO deadline. The grab is active (a mid-drag
            // pause must not let the timeout expire and snap the window to a letterbox); release
            // always sets `settle_at`, after which the deadline applies.
            None => false,
            Some(settle_at) => {
                if now >= p.deadline {
                    true // safety net (frozen / never-committing / endlessly-clamping client)
                } else {
                    // Settle only once the content has gone STABLE for `RESIZE_SETTLE_STABLE` —
                    // this both detects a clamped client AND holds the stretch a few frames past
                    // the geometry match, covering the blank/no-content frames some apps commit
                    // right after acking (otherwise the background flashes through). Require either
                    // the target, or a size the client committed *after* the gesture ended (clamp);
                    // a merely-slow client whose old buffer never changed keeps waiting.
                    let stable = now.duration_since(p.last_content_change) >= RESIZE_SETTLE_STABLE;
                    let matched_target = (content.w - p.target.w).abs() <= 2
                        && (content.h - p.target.h).abs() <= 2;
                    stable && (matched_target || p.last_content_change > settle_at)
                }
            }
        }
    };
    if clear {
        *guard = None;
    }
    !clear
}
