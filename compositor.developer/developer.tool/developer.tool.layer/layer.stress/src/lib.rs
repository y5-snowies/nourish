//! Wayland layer-shell stress-test harness — shared library.
//!
//! Two binaries build on this crate:
//! - `layer-stress-controller` — a well-behaved xdg GUI window that spawns and drives the
//!   subject by writing [`protocol::Command`] lines to its stdin.
//! - `layer-stress-subject`    — a single `zwlr_layer_surface_v1` under test, which
//!   reconfigures itself on command to stress the compositor's layer-shell logic.
//!
//! Sibling of the `window.stress` harness (xdg toplevels); the `canvas`, `diag` and `font`
//! modules are shared verbatim. See `protocol::Command` for the full scenario vocabulary.

pub mod canvas;
pub mod diag;
pub mod font;
pub mod protocol;
