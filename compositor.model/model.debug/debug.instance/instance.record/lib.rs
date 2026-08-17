//! compositor.developer structured logging — **frontend** (tracing-free).
//!
//! Callsites use the level macros (`error!`/`warn!`/`info!`/`trace!`) plus `abort!`
//! (e.g. `#[macro_use] extern crate compositor_model_debug_instance_record;`).
//! Two independent controls: the cargo features `error/warn/info/trace` strip a level's
//! macro at compile time (see Cargo.toml); [`set_enabled_mask`] (`COMPOSITOR_LOG_LEVEL`)
//! gates at runtime. Records go to a global fan-in buffer drained by the log process.
//! Types live in `instance.level`, global state in `instance.channel`; everything is
//! re-exported here so `$crate::...` macro paths and downstream uses keep resolving.

pub mod record;
pub use record::*;
