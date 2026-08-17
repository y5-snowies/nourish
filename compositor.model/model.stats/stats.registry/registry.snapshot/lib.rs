//! Read-side of the diagnostics registry: `snapshot()` reads the counters + metadata and
//! derives FPS / vblank-rate over the interval since the last snapshot. The developer-log
//! gRPC service calls it on demand (the Statistics tab's Refresh button).

pub mod snapshot;
pub use snapshot::*;
