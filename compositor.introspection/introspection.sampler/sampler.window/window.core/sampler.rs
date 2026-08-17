//! Background sampler for placeholder application data.
//!
//! Owns a dedicated thread that periodically re-extracts MetaNode +
//! InferredHints for each registered placeholder. Results are buffered
//! and flushed to the compositor's main thread in bulk via a calloop
//! channel, so the compositor sees a single batch event per flush.
//!
//! Cadence, debouncing, and the thread loop live in the sibling crates
//! `window.schedule`, `window.batch`, and `window.engine`. Registration
//! messages flow main → sampler over `std::sync::mpsc`; flush batches
//! flow sampler → main over a `calloop::channel`. The thread exits when
//! either channel closes (i.e., when this handle is dropped on the main
//! thread, or when the calloop receiver is removed).

use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use smithay::reexports::calloop::channel::Sender as CalloopSender;
use uuid::Uuid;
use compositor_introspection_extraction_window_base::{HandlerRegistry, Meta, MetaNode};
use compositor_introspection_sampler_window_engine::engine::run;
use compositor_introspection_sampler_window_schedule::schedule::{Entry, Registration};

pub use compositor_introspection_sampler_window_batch::batch::{SampleBatch, SampleResult};

/// Handle to the background sampler. Drop to terminate the thread.
pub struct Sampler {
    tx: mpsc::Sender<Registration>,
    _join: thread::JoinHandle<()>,
}

impl Sampler {
    /// Spawn the sampler thread.
    pub fn spawn(registry: Arc<HandlerRegistry>, results: CalloopSender<SampleBatch>) -> Self {
        let (tx, rx) = mpsc::channel::<Registration>();
        let join = thread::Builder::new()
            .name("y5-sampler".into())
            .spawn(move || run(rx, registry, results))
            .unwrap_or_else(|e| abort!("spawn sampler thread: {e:?}"));

        Self { tx, _join: join }
    }

    /// Register a placeholder for periodic sampling. `previous_meta`
    /// is the captured MetaNode so Wayland-side fields (uid/gid/app_id/
    /// title) can be preserved across refreshes.
    pub fn register(&self, uuid: Uuid, pid: u32, previous_meta: MetaNode) {
        let _ = self.tx.send(Registration::Add(Entry::new(uuid, pid, previous_meta)));
    }

    /// Stop sampling this placeholder.
    pub fn unregister(&self, uuid: Uuid) {
        let _ = self.tx.send(Registration::Remove(uuid));
    }

    /// REQUEST an out-of-cadence sample for these windows — granted only for the
    /// ones whose last sample is older than
    /// [`IMMEDIATE_MIN_AGE`](compositor_introspection_sampler_window_schedule::schedule::IMMEDIATE_MIN_AGE).
    /// A request where everything is already fresh is a no-op, so calling it
    /// every time a UI opens costs nothing.
    ///
    /// `surfaces` pairs each window's uuid with the wayland half of its identity
    /// read on the CALLING thread — app_id, title and credentials — because the
    /// sampler thread has no way to read a surface. Everything else it re-derives
    /// from `/proc` itself.
    ///
    /// Fire-and-forget: results arrive in the usual batch. A caller that wants
    /// fresh data shows the latest sample it has and lets this replace it.
    pub fn request_immediate(&self, surfaces: Vec<(Uuid, Meta)>) {
        if surfaces.is_empty() {
            return;
        }
        let _ = self.tx.send(Registration::RequestImmediate(surfaces));
    }

    /// Limit sampling to these windows — the current world's. Entries outside it
    /// stay queued but are skipped, so re-entering their world resumes them.
    pub fn set_scope(&self, uuids: std::collections::HashSet<Uuid>) {
        let _ = self.tx.send(Registration::Scope(uuids));
    }
}
