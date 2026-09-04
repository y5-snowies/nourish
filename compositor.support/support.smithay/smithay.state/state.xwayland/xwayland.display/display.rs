//! The X display number, as a process global.
//!
//! `DISPLAY` is the one piece of session environment y5 does not know at startup:
//! the Xwayland server picks its display asynchronously and reports it back through
//! `XWaylandEvent::Ready`, well after the launch executor has been built. The
//! executor's base environment is a CLOSURE re-evaluated per launch precisely so
//! late-arriving facts like this one can be honoured (see `executor.base::BaseEnv`),
//! and this is where it reads that fact.
//!
//! A global rather than a field on the dispatch state because the readers are not on
//! the dispatch path: a launch runs off the executor, which has no `Dispatch` and
//! must not grow one to learn a number that is fixed for the session.

use std::sync::atomic::{AtomicI32, Ordering};

/// No X server. Negative because a display number is a non-negative integer, so no
/// valid value can collide with it.
const NONE: i32 = -1;

static DISPLAY: AtomicI32 = AtomicI32::new(NONE);

/// Record the display the Xwayland server came up on.
pub fn set(number: u32) {
    DISPLAY.store(number as i32, Ordering::Release);
}

/// Forget it — the server died, or never started. Launches fall back to the empty
/// `DISPLAY` that keeps apps off a nonexistent X server.
pub fn clear() {
    DISPLAY.store(NONE, Ordering::Release);
}

/// The `DISPLAY` value for a child process (`":1"`), or `None` while there is no X
/// server to point one at.
pub fn get() -> Option<String> {
    match DISPLAY.load(Ordering::Acquire) {
        NONE => None,
        number => Some(format!(":{number}")),
    }
}
