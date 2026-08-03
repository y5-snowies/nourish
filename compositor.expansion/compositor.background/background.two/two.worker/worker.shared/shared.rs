//! The process-wide background worker, started on first use.
//!
//! One worker serves every pane and every world — panes are keyed inside it, and
//! they share a device and pipeline cache because the shader is the same and the
//! GPU has one queue. Spawning per rebuild would instead create a fresh device
//! every time the selected shader changed.
//!
//! Its tunables are NOT captured here: the worker re-reads the process-global
//! `TripleBuffer` each pass, so rate, cadence, ceiling and pipelining all apply
//! live. Only `enabled` needs a restart, since it decides whether this exists.

use compositor_background_two_worker_base::base::Worker;
use std::sync::{Arc, OnceLock};

/// The shared worker, spawning it on first call. `None` means it could not
/// start — the caller must then draw no background rather than quietly reverting
/// to the inline shader path, which is the thing this exists to avoid.
pub fn worker() -> Option<Arc<Worker>> {
    static W: OnceLock<Option<Arc<Worker>>> = OnceLock::new();
    W.get_or_init(|| {
        match Worker::spawn() {
            Ok(w) => Some(Arc::new(w)),
            Err(e) => {
                error!("background worker failed to start: {e}; background disabled");
                None
            }
        }
    })
    .clone()
}
