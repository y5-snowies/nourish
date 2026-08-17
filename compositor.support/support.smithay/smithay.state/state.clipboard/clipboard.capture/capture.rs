//! The clipboard's single persisted slot.
//!
//! A wayland selection is a live `wl_data_source` owned by the copying client and there
//! is no OS-level clipboard behind it, so it dies with that client. This is the snapshot
//! taken while the source is alive, to offer the same bytes afterwards.
//!
//! No history: one head, or nothing. `generation` is what makes that true — every copy
//! bumps it and drops the head BEFORE any byte of the new selection is read, so a
//! capture that lands late is discarded instead of resurrecting a replaced clipboard.

use std::sync::Arc;

/// One captured flavor of the current head.
pub struct Flavor {
    pub mime: String,
    /// Shared, not owned: every paste hands the same bytes to another async writer, and
    /// a client may ask for one flavor repeatedly. Cloning per paste would let it
    /// multiply the budget by the number of requests it makes.
    pub bytes: Arc<Vec<u8>>,
    /// Position in the client's advertised list — what the offer is sorted by.
    pub order: usize,
}

/// The persisted clipboard slot.
///
/// Reads themselves are not here — the capture worker owns those, and hands finished
/// flavors over for [`ClipboardCapture::admit`] to accept or refuse.
#[derive(Default)]
pub struct ClipboardCapture {
    generation: u64,
    flavors: Vec<Flavor>,
    used: usize,
    advertised: usize,
}

impl ClipboardCapture {
    /// Invalidate-first: drop the head and bump the generation before anything is read,
    /// so the slot is either the current head's data or nothing — never an older head.
    ///
    /// Reads already in flight are cancelled separately, by arming the worker with the
    /// generation this returns. A superseded heavy capture must not keep filling a doomed
    /// buffer, and closing its read end releases a client blocked writing into a pipe
    /// nobody is draining.
    pub fn arm(&mut self, advertised: usize) -> u64 {
        self.generation += 1;
        self.flavors.clear();
        self.used = 0;
        self.advertised = advertised;
        self.generation
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn is_current(&self, generation: u64) -> bool {
        generation == self.generation
    }

    /// Admit a completed flavor. Refused when the generation is stale (a newer copy
    /// landed while this one was reading) or when it would exceed the budget.
    pub fn admit(&mut self, at: u64, order: usize, mime: String, bytes: Vec<u8>, budget: usize) -> bool {
        if !self.is_current(at) || bytes.is_empty() || self.used + bytes.len() > budget {
            return false;
        }
        self.used += bytes.len();
        self.flavors.push(Flavor { mime, bytes: Arc::new(bytes), order });
        true
    }

    /// What is left of the budget. The worker charges its own in-flight reads against
    /// this so a flavor is capped before it is buffered, not after.
    pub fn headroom(&self, budget: usize) -> usize {
        budget.saturating_sub(self.used)
    }

    /// The mime types actually captured, in the order the CLIENT advertised them.
    ///
    /// Not the advertised superset — a paste of a flavor dropped for budget must fail at
    /// negotiation rather than hand over an empty pipe. And not capture order, which is
    /// ours: presenting survivors as the client presented them means the only thing a
    /// receiver can observe across the exit is the absences, never a reshuffle.
    pub fn mime_types(&self) -> Vec<String> {
        let mut flavors: Vec<&Flavor> = self.flavors.iter().collect();
        flavors.sort_by_key(|flavor| flavor.order);
        flavors.into_iter().map(|flavor| flavor.mime.clone()).collect()
    }

    pub fn bytes_for(&self, mime: &str) -> Option<Arc<Vec<u8>>> {
        let found = self.flavors.iter().find(|flavor| flavor.mime == mime);
        found.map(|flavor| flavor.bytes.clone())
    }

    pub fn is_empty(&self) -> bool {
        self.flavors.is_empty()
    }

    pub fn used(&self) -> usize {
        self.used
    }

    /// How many flavors the client advertised for the current head.
    pub fn advertised(&self) -> usize {
        self.advertised
    }
}
