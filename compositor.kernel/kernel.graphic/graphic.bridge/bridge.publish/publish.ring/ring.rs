//! The publish ring: N buffers, one published, one drawn into, the rest absorbing
//! frames whose GPU work has not finished yet.
//!
//! Generic over the buffer type because its two users hold different imports —
//! the bevy and iced surfaces each pair a dmabuf with their own texture views —
//! while the rotation and the publish rule are identical, and that rule is the
//! whole point: the compositor is only ever handed a buffer whose write has
//! already completed, so its composite never waits on the UI's GPU work through
//! the dmabuf's implicit fence.
//!
//! # A single slot is not a one-deep ring
//!
//! At depth one there is nothing to decouple: the compositor samples the very
//! buffer the producer just drew, which is the pre-ring behaviour and is what
//! "triple buffering off" must mean. So depth one takes [`Ring::passthrough`] —
//! no completion callback, no host wait, no device poll, nothing but a
//! generation bump. Sequencing there is the dmabuf's implicit fence, exactly as
//! before, and the cost is exactly what it was.
//!
//! That is not a micro-optimisation. Running the ring machinery at depth one put
//! a BLOCKING `Device::poll(Wait)` on the compositor thread once per surface per
//! frame — the thread that also dispatches input — where the original code had
//! no host synchronisation at all. Turning the feature off has to actually turn
//! it off.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Ring<T> {
    slots: Vec<T>,
    /// Being drawn into.
    cursor: usize,
    /// What the compositor samples.
    published: usize,
    /// Submitted, not yet known complete. FIFO — one queue completes in order.
    inflight: VecDeque<usize>,
    /// One completion flag PER SLOT, reused rather than reallocated.
    ///
    /// `Queue::on_submitted_work_done` needs an owned `Send` sink, and minting a
    /// fresh `Arc<AtomicBool>` for it every frame is an allocation per surface
    /// per frame for a value whose lifetime is exactly one slot's turn. A slot
    /// cannot be in flight twice at once — that is what `capacity` enforces — so
    /// its flag can simply be cleared and handed out again.
    done: Vec<Arc<AtomicBool>>,
    /// Bumped on every publish. The commit counter follows it, so a frame that
    /// publishes nothing reports no damage.
    generation: u64,
    /// A depth-one submit awaiting its (immediate) publish.
    ///
    /// At depth one nothing is in flight — the compositor samples the buffer the
    /// producer just drew — so `inflight` stays empty and `poll` would report
    /// "nothing moved" forever. Callers gate their publish on that, so without
    /// this flag a ring collapsed to one slot never publishes again and every
    /// surface on it freezes. Reachable live: toggling the feature off drops
    /// `depth()` to one under a worker that is already running.
    pending: bool,
    /// Set whenever the buffer to draw into stops being the one the renderer was
    /// last pointed at. Rebinding a render target is not free, so a ring that
    /// never rotates — the single-slot disabled path — must not ask for one.
    retarget: bool,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl<T> Ring<T> {
    pub fn new(slots: Vec<T>, device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let done = (0..slots.len()).map(|_| Arc::new(AtomicBool::new(false))).collect();
        Self {
            slots, cursor: 0, published: 0, inflight: VecDeque::new(), done,
            generation: 0, pending: false, retarget: false, device, queue,
        }
    }

    /// Depth one: the compositor samples what the producer is drawing, with no
    /// completion tracking, no host wait and no device poll. See the module note.
    fn passthrough(&self) -> bool {
        self.slots.len() < 2
    }

    /// Whether the renderer must be re-pointed at [`target`](Self::target).
    /// Clears the flag.
    pub fn take_retarget(&mut self) -> bool {
        std::mem::take(&mut self.retarget)
    }

    pub fn len(&self) -> usize { self.slots.len() }
    pub fn is_empty(&self) -> bool { self.slots.is_empty() }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn published(&self) -> &T { &self.slots[self.published] }
    pub fn target(&self) -> &T { &self.slots[self.cursor] }

    /// Whether a submitted frame has yet to be published. A dirty-driven host
    /// must keep scheduling while this holds, or a deferred publish lands on a
    /// frame that never comes and the surface sits on stale pixels.
    pub fn has_pending(&self) -> bool { !self.inflight.is_empty() }

    /// Frames allowed in flight: every slot except the published one and the one
    /// being drawn into.
    ///
    /// A two-slot ring yields zero, so `submitted` publishes synchronously there
    /// whatever the caller asked for — two buffers cannot pipeline, and the knob
    /// is inert rather than wrong. The third slot is what buys the overlap, and
    /// with it the wider gap before a slot is reused.
    fn capacity(&self) -> usize {
        self.slots.len().saturating_sub(2)
    }

    /// Retire finished frames; the newest retired one becomes what the compositor
    /// samples. Returns whether anything was published.
    ///
    /// POLLS THE DEVICE ITSELF, and that is deliberate even though the sweep is
    /// device-global and several rings repeat it in a frame. It was briefly
    /// hoisted to a per-frame `pump` the owner had to call, which is cheaper and
    /// wrong in a way that does not announce itself: the precondition then lived
    /// in four call sites instead of here, and any path that polled without one
    /// left its frames in flight forever — a surface that simply never appears.
    /// A redundant sweep costs microseconds; a stranded frame costs the window.
    pub fn poll(&mut self) -> bool {
        let _ = self.device.poll(wgpu::PollType::Poll);
        // Depth one publishes at submit — there is no completion to wait for, so
        // report the submit here, where `moved` is the caller's publish signal.
        if self.passthrough() {
            if std::mem::take(&mut self.pending) {
                self.generation += 1;
                return true;
            }
            return false;
        }
        let mut moved = false;
        while self.inflight.front().is_some_and(|s| self.done[*s].load(Ordering::Acquire)) {
            let slot = self.inflight.pop_front().expect("front just matched");
            self.published = slot;
            self.generation += 1;
            moved = true;
        }
        moved
    }

    /// Pick and return the slot to draw into. Blocks only once the GPU has fallen
    /// a whole ring behind, which is the backpressure — without it a slow GPU
    /// would have us overwrite a buffer it is still reading.
    pub fn begin(&mut self) -> &T {
        if self.passthrough() {
            return &self.slots[self.cursor];
        }
        while self.inflight.len() > self.capacity() {
            let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
            if !self.poll() {
                // The wait returned with nothing retired: rather than spin on a
                // callback that may never arrive, take the slot and accept the
                // overlap for this one frame.
                break;
            }
        }
        let next = self.next_free();
        self.retarget |= next != self.cursor;
        self.cursor = next;
        &self.slots[self.cursor]
    }

    /// The next slot round-robin that is neither published nor still being
    /// written. Falls back to the current cursor when nothing is free.
    fn next_free(&self) -> usize {
        let n = self.slots.len();
        (1..=n)
            .map(|i| (self.cursor + i) % n)
            .find(|c| *c != self.published && !self.inflight.contains(c))
            .unwrap_or(self.cursor)
    }

    /// Record that the renderer has submitted its work for the current slot.
    ///
    /// `pipeline` defers the publish to a polled completion so recording the next
    /// frame overlaps this one running. It is ignored where deferring would be
    /// wrong: a ring with no spare slot has nowhere to defer to, and the first
    /// frame has nothing published yet, so deferring would show one frame of an
    /// unwritten buffer.
    /// Returns whether this call published — the caller's publish signal, so it
    /// never has to reconstruct one from a remembered generation.
    pub fn submitted(&mut self, pipeline: bool) -> bool {
        // Depth one: no completion to track and nothing to wait for — the
        // compositor is sampling this very buffer, sequenced by the dmabuf's
        // implicit fence. Just count the frame so damage still follows.
        if self.passthrough() {
            self.pending = true;
            return self.poll();
        }
        let done = self.done[self.cursor].clone();
        done.store(false, Ordering::Release);
        let flag = done.clone();
        self.queue.on_submitted_work_done(move || flag.store(true, Ordering::Release));
        self.inflight.push_back(self.cursor);

        if !pipeline || self.capacity() == 0 || self.generation == 0 {
            let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        }
        self.poll()
    }

    /// Grow or shrink the ring to `depth`, calling `make` for each slot added.
    ///
    /// Shrinking waits for the ring to quiesce and keeps the published buffer by
    /// moving it to the front, so a settings change cannot drop the very buffer
    /// the compositor is about to sample.
    pub fn set_depth<E>(
        &mut self,
        depth: usize,
        mut make: impl FnMut() -> Result<T, E>,
    ) -> Result<(), E> {
        let depth = depth.max(1);
        while self.slots.len() < depth {
            self.slots.push(make()?);
            self.done.push(Arc::new(AtomicBool::new(false)));
        }
        if self.slots.len() > depth {
            let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
            self.poll();
            self.inflight.clear();
            self.slots.swap(0, self.published);
            self.slots.truncate(depth);
            self.done.truncate(depth);
            self.published = 0;
            self.cursor = 0;
            self.retarget = true;
        }
        Ok(())
    }

    /// Replace every slot — the resize path, where each buffer must be
    /// reallocated at the new size and nothing may be carried over.
    pub fn replace(&mut self, slots: Vec<T>) {
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        self.inflight.clear();
        self.pending = false;
        self.done = (0..slots.len()).map(|_| Arc::new(AtomicBool::new(false))).collect();
        self.slots = slots;
        self.cursor = 0;
        self.published = 0;
        self.retarget = true;
        // Back to "nothing published", so the next frame publishes synchronously
        // and the new buffers are never sampled before they hold anything.
        self.generation = 0;
    }
}
