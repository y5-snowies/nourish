//! Native graphics-tablet (pen / stylus) input, sibling of `touch/`.
//!
//! The four `InputEvent::TabletTool*` variants are routed here from
//! `delegate_main`. The whole `zwp_tablet_manager_v2` protocol is implemented
//! externally on `Dispatch.tablet` (see `wire.tablet`); these handlers resolve the
//! pen position into y5-world space (reusing the pointer's absolute→world path),
//! hit-test with the SAME `surface_under_filtered` pointer/touch use, and either
//! forward native tool events to a tablet-aware client or (on the canvas) pan.
//!
//! Tablet-only: we never drive `wl_pointer` from the pen. The stroke role
//! (Client vs Pan) is latched at tip-down and held until tip-up / proximity-out
//! (`wire.tablet::Stroke`), exactly like `touch/session.rs` latches its `Role`.

pub mod axis;
pub mod button;
pub mod client;
pub mod coords;
pub mod cursor;
pub mod proximity;
pub mod tip;
