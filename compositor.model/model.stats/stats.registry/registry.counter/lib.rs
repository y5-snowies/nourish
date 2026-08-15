//! Hot-path diagnostics counters: plain relaxed atomics — a per-frame increment is a few
//! ns, negligible. The render/backend paths call the cheap update fns; `snapshot()` (in
//! the sibling snapshot crate) reads the raw statics.

pub mod counter;
pub use counter::*;
