//! The compositor-side stand-in for an instance that lives on the worker.
//!
//! Implements the same `BevyInstanceAny` vtable as the inline `BevyInstance`, so
//! the registry, item wrapper and element builder do not care which they hold.
//! It owns only bookkeeping — id, placement, size, commit — and reads finished
//! frames off the `Board`. There is no `App` here and no GPU resource: the worker
//! owns both.

pub mod instance;
pub use instance::*;
