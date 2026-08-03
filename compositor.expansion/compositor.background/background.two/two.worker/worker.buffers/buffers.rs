//! The buffer ring the worker renders into and the compositor samples.
//!
//! THE INVARIANT this type exists to enforce: the compositor must never sample a
//! buffer the worker is currently writing. Not for tearing reasons — a torn
//! backdrop is fine and expected — but because a write fence landing on that
//! dmabuf's `dma_resv` would make the COMPOSITOR's own GPU submission wait on the
//! worker's render inside the kernel. That would re-couple the two through
//! implicit sync and put the stall straight back onto the input thread, which is
//! the entire thing this worker exists to remove.
//!
//! Publishing a slot only after the worker's fence has signalled is what makes
//! the invariant hold.
//!
//! # How many slots
//!
//! The worker is signal-driven: it renders at most one frame per compositor draw
//! signal, so it can never produce frames faster than the compositor consumes
//! them. The compositor holds a published buffer for at most one frame.
//!
//! THREE closes the reuse window outright: round-robin means the worker cannot
//! return to a slot until it has published two others, so any composite that
//! sampled it has long retired. It is also what pipelining needs — one slot
//! pinned by the reader, one by the writer, one in flight.
//!
//! TWO is a supported trade, not a broken configuration. It cannot pipeline (the
//! caller publishes synchronously there), and it wraps straight back onto the
//! slot the compositor's previous composite may still be reading — which implicit
//! sync turns into a WAIT ON THE WORKER, never corruption and never compositor
//! latency. On a single-queue GPU that wait is near-unreachable: the composite
//! was submitted before our render, so it has retired by the time we unblock. It
//! saves one fullscreen dmabuf PER PANE, which on a multi-monitor Pi is the
//! largest memory item in this feature.

use smithay::backend::allocator::dmabuf::Dmabuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Depth bounds. Two is the memory-saving trade above; three is what pipelining
/// needs. Nothing above three: a fourth only lets the GPU fall a further frame
/// behind, which is latency rather than throughput.
pub const MIN_SLOTS: usize = 2;
pub const MAX_SLOTS: usize = 3;

/// Clamp a requested depth into range.
pub fn slots(requested: u8) -> usize {
    (requested as usize).clamp(MIN_SLOTS, MAX_SLOTS)
}

/// Whether a ring this deep can defer a publish.
///
/// The same rule `publish.ring`'s `capacity` encodes: deferring needs a slot
/// that is neither published nor being written, and `depth - 2` of those exist.
/// At two there are none, so the `pipeline` knob is INERT rather than wrong —
/// honouring it there would have the next render wrap straight onto the buffer
/// the compositor is reading. Both sides of the feature clamp it the same way so
/// a hand-edited `preferences.json` cannot reach a combination the docs say is
/// impossible.
pub fn can_pipeline(depth: usize) -> bool {
    depth.saturating_sub(2) > 0
}

pub struct Slots {
    bufs: Vec<Dmabuf>,
    /// Monotonic count of completed frames; `0` = nothing published yet, and the
    /// compositor must then draw no background rather than sample a slot that
    /// was never written.
    ///
    /// Doubles as the buffer index because publishes are strictly round-robin
    /// (see [`Slots::next_after`]), so one atomic carries both "which buffer is
    /// newest" and "how many frames have landed". The latter is what lets the
    /// compositor report damage only when the background actually changed,
    /// instead of re-compositing an identical buffer every frame.
    generation: AtomicUsize,
    /// The newest generation the compositor has actually taken.
    ///
    /// The acknowledgement half of the handshake: without it the worker renders
    /// frames that are overwritten before anyone reads them, since it produces at
    /// its own rate and the compositor consumes at its own. Holding to one
    /// unread frame ties them together without either side blocking.
    consumed: AtomicUsize,
}

impl Slots {
    pub fn new(bufs: Vec<Dmabuf>) -> Self {
        Self { bufs, generation: AtomicUsize::new(0), consumed: AtomicUsize::new(0) }
    }

    /// Compositor thread: the newest fully-rendered buffer and its generation, or
    /// `None` before the worker's first frame completes. The generation is stable
    /// while the worker has published nothing new, which is what makes the
    /// background report no damage on those frames.
    pub fn latest(&self) -> Option<(&Dmabuf, usize)> {
        let generation = self.generation.load(Ordering::Acquire);
        if generation == 0 {
            return None;
        }
        self.consumed.store(generation, Ordering::Release);
        Some((&self.bufs[(generation - 1) % self.bufs.len()], generation))
    }

    /// Worker thread: has the compositor taken the last frame we published? While
    /// false, another render would only overwrite a buffer nobody has read.
    pub fn drained(&self) -> bool {
        self.consumed.load(Ordering::Acquire) >= self.generation.load(Ordering::Acquire)
    }

    /// Worker thread: call ONLY once the render has signalled its fence, and only
    /// in round-robin slot order. The `Release` pairs with `latest`'s `Acquire` so
    /// the buffer's contents are visible before the generation naming it is.
    ///
    /// Also wakes the compositor, which is not an optimisation: an undamaged frame
    /// queues no flip, so no vblank arrives and its loop stops — while the
    /// background is the only thing animating, this alone can restart it.
    pub fn publish(&self) {
        self.generation.fetch_add(1, Ordering::Release);
        compositor_kernel_graphic_bridge_publish_wake::wake::notify_offthread_published();
    }

    /// Worker thread: the slot to render into after `previous`. Plain round-robin
    /// — see the module header for what each depth buys.
    pub fn next_after(&self, previous: usize) -> usize {
        (previous + 1) % self.bufs.len()
    }

    pub fn len(&self) -> usize {
        self.bufs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bufs.is_empty()
    }

    /// Every slot, for one-time setup (importing each as a render target).
    pub fn all(&self) -> &[Dmabuf] {
        &self.bufs
    }
}
