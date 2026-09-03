//! X11 / XWayland stress-test harness — shared library.
//!
//! Two binaries build on this crate:
//! - `x11-stress-controller` — spawns the subject and drives it, either interactively
//!   from a terminal or by replaying a named [`scenario`].
//! - `x11-stress-subject`    — the X11 client under test, which creates windows the
//!   compositor's XWM has to cope with.
//!
//! Sibling of `../window.stress`, which is the wayland-side equivalent, and it follows
//! the same split: one [`protocol::Command`] per stdin line, diagnostics to stderr.
//!
//! The subject is an X11 client and nothing else — it never speaks wayland. Point it
//! at a nested y5 through `DISPLAY` and everything it does arrives at the compositor
//! through Xwayland and the in-process XWM, which is the path under test.

pub mod protocol;
pub mod scenario;
