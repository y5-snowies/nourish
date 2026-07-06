//! FPS overlay: a small, solid-black, top-right SCREEN-space iced surface
//! showing the current composited-frame rate.
//!
//! It is click-through (passthrough) so it never steals pointer input, and it
//! is pushed a new value only when the shown number changes, so an idle overlay
//! costs nothing per frame. Ported from the `debug-placeholder-performance`
//! debug branch; the DrawOrder-bench "opt ON/LEGACY" mode label was dropped
//! since that bench does not exist in this tree.

// Developer logging: bring error!/warn!/info!/trace!/abort! into scope.
#[macro_use]
extern crate compositor_developer_debug_instance_record;

pub mod fps;
