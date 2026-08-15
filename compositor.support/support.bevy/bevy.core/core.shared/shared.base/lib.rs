//! Process-wide shared wgpu context.
//!
//! Equivalent of `SharedEngine` in the iced crate, but minimal: Bevy doesn't
//! share its renderer across instances (each `App` has its own ECS world and
//! render graph), so the only thing shared is the wgpu device + queue +
//! instance + adapter.
//!
//! The `Arc`-wrapped context is handed to each `BevyRuntime` at creation
//! time. Cloning is cheap.

pub mod base;
pub use base::*;
