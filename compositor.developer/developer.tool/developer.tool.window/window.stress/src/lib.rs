//! Wayland window-layout stress-test harness — shared library.
//!
//! Two binaries build on this crate:
//! - `window-stress-controller` — a well-behaved GUI window that spawns and drives the
//!   subject by writing [`protocol::Command`] lines to its stdin.
//! - `window-stress-subject`    — the experimental window under test, which deliberately
//!   misbehaves on command to stress the compositor's window-layout logic.
//!
//! See the module docs and `protocol::Command` for the full scenario vocabulary.

pub mod canvas;
pub mod diag;
pub mod dnd;
pub mod font;
pub mod protocol;
pub mod session;
