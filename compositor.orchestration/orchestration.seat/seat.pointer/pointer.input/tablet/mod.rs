//! Native graphics-tablet (pen / stylus) input, sibling of `touch/`.
//!
//! The four `InputEvent::TabletTool*` variants are routed here from
//! `delegate_main`. The whole `zwp_tablet_manager_v2` protocol is implemented
//! externally on `Dispatch.tablet` (see `wire.tablet`); these handlers resolve the
//! pen position into y5-world space (reusing the pointer's absolute→world path),
//! hit-test with the SAME `surface_under_filtered` pointer/touch use, and either
//! forward native tool events to a tablet-aware client or (on the canvas) pan.
//!
//! The pen drives `wl_pointer` (the cursor follows it) for everything that isn't a
//! native tool stroke. The stroke role (`Tablet` draw vs `Pointer` mouse/pan) is
//! latched at tip-down and held until tip-up / proximity-out (`wire.tablet::Stroke`),
//! like `touch/session.rs` latches its `Role`. In the canvas **Hand grab** the pen
//! is a navigation tool — tip-drag pans the world and the dial zooms (see
//! [`hand_active`]) — so it never draws or forwards hover while the grab is held.

pub mod action;
pub mod axis;
pub mod button;
pub mod client;
pub mod coords;
pub mod cursor;
pub mod inject;
pub mod mode;
pub mod pad;
pub mod proximity;
pub mod tip;

pub use mode::{dial_zoom_active, hand_active, select_active};
