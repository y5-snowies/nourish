//! "Do not ask the driver the same question twice."
//!
//! Every ring in this family re-checks its buffers on a hot path: the worker
//! loops, the compositor calls `sync_depth`/`ensure_backing` once per frame per
//! surface. Those calls are cheap no-ops when they succeed, so "try, log the
//! error, try again next frame" reads fine — right up until the allocation or
//! the import fails, at which point it runs at the loop's full rate: thousands
//! of failed attempts and log lines a second.
//!
//! A TIMED BACKOFF WOULD BE THE WRONG FIX. A dmabuf allocation or a wgpu import
//! is a hard, deterministic answer about a specific request: this size, this
//! depth, this format, on this device. Nothing about it is transient, so waiting
//! and asking again gets the same refusal — a retry loop that is merely slower.
//!
//! So the gate is keyed on the REQUEST, not on the clock. A failure records what
//! was asked for and every identical ask is skipped outright; the next attempt
//! happens only once something that could change the answer has changed — a
//! resize, a new ring depth, a surface re-created. That also makes the log
//! honest: one line per distinct failing request, not one per frame.

/// Remembers the request that failed, so an identical one is not re-attempted.
///
/// `K` is whatever fully determines the outcome at the call site — a size, a
/// depth, a `(size, depth)` pair. Getting `K` wrong in the narrow direction
/// (leaving out an input that matters) only costs a missed retry; getting it
/// wrong in the wide direction re-opens the storm, so prefer the narrow one.
#[derive(Debug)]
pub struct Attempt<K> {
    failed: Option<K>,
}

impl<K> Default for Attempt<K> {
    fn default() -> Self {
        Self { failed: None }
    }
}

impl<K: PartialEq> Attempt<K> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `request` is worth trying: true unless this exact one already
    /// failed.
    pub fn worth_trying(&self, request: &K) -> bool {
        self.failed.as_ref() != Some(request)
    }

    /// Record that `request` failed. Returns whether the caller should LOG it —
    /// true only the first time a given request fails, so a wedged surface
    /// produces one line rather than one per frame.
    pub fn failed(&mut self, request: K) -> bool {
        let first = self.failed.as_ref() != Some(&request);
        self.failed = Some(request);
        first
    }

    /// Clear the record. Call on success, including the no-op case, so a surface
    /// that recovers is not still blocked by an old refusal.
    pub fn succeeded(&mut self) {
        self.failed = None;
    }
}
