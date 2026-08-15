//! compositor.developer structured logging — **init entry point**.
//!
//! Call [`spawn`] once, as early as possible, from the compositor's entry crate
//! (`loader.main.execute`). It wires the global start instant, the runtime level mask
//! (from the `log_level` field of the central environment config), the fan-in buffer, and
//! the drain + gRPC threads. After this, any crate's level macros
//! (`error!`/`warn!`/`info!`/`trace!`) emit records.
//!
//! `environment::init()` MUST run before this (it does, as the very first line of
//! `main()`), so `get().log_level` is available here.

pub mod process;
pub use process::*;
