//! Deterministic spring solver. Output: progress 0→1 driven by physical
//! spring dynamics over a fixed duration. Returns `None` once the duration
//! is exceeded (plus a small epsilon for frame-drop tolerance).

pub mod spring;
pub use spring::*;
