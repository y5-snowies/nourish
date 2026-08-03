//! Per-pane worker state, and the registry the compositor reads it through.
//!
//! One pane per background draw target. Panes must NOT share a buffer: each has
//! its own camera and size, so on multi-monitor a shared one is the wrong
//! resolution and view for all but one. They do share a device and pipeline
//! cache — same shader, one GPU queue.

use compositor_background_two_worker_buffers::buffers::Slots;
use compositor_background_two_worker_target::target::Target;
use smithay::backend::allocator::dmabuf::Dmabuf;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
/// Published as panes are allocated; read by the compositor to find a pane's
/// newest finished buffer. Absent until first allocation, and the compositor
/// then draws no background for that pane.
pub type Registry = Arc<Mutex<HashMap<u64, Arc<Slots>>>>;

pub struct Pane {
    pub targets: Vec<Target>,
    /// This pane's OWN command buffers and fences, one per ring slot and indexed
    /// by [`Self::cursor`] alongside `targets`. Per-pane, not shared: a shared
    /// ring made one pane's `acquire` wait on another monitor's fence, and made
    /// the depth a pane actually got depend on how many other panes were live.
    pub cmds: Vec<ash::vk::CommandBuffer>,
    pub fences: Vec<ash::vk::Fence>,
    pub slots: Arc<Slots>,
    pub size: (u32, u32),
    pub cursor: usize,
    /// This pane's monitor refresh, from each ping — `Rate::Multiplier` is
    /// relative to it.
    pub refresh: Duration,
    /// This pane's own slot index for a submitted-but-unpublished frame
    /// (pipelined only). Indexes `targets`/`cmds`/`fences` alike.
    pub pending: Option<usize>,
    /// Read by `worker.pace`, which owns when this pane may render next.
    pub last: Option<Instant>,
    pub last_serial: Option<u64>,
    /// Next wall-clock slot for the rate gate, and for the cap gate.
    ///
    /// A SCHEDULE, not an elapsed-since-last test. The schedule advances from the
    /// previous slot rather than from when a render happened to finish, so a slow
    /// frame does not push the next one out and a late poll does not accumulate
    /// drift. That is what lets the gates run with no tolerance term at all — and
    /// a tolerance is exactly what made a "1x refresh" cap admit 2x.
    pub due_rate: Option<Instant>,
    pub due_cap: Option<Instant>,
}

impl Pane {
    pub fn new(
        targets: Vec<Target>,
        cmds: Vec<ash::vk::CommandBuffer>,
        fences: Vec<ash::vk::Fence>,
        size: (u32, u32),
    ) -> Self {
        let dmabufs: Vec<Dmabuf> = targets.iter().map(|t| t.dmabuf.clone()).collect();
        info!("background worker: pane allocated {}x{} slots={}", size.0, size.1, dmabufs.len());
        Self {
            targets, cmds, fences, slots: Arc::new(Slots::new(dmabufs)), size, cursor: 0,
            // 30Hz until the first ping carries this pane's real refresh — the
            // slow guess, since this becomes a minimum interval (see `pace`).
            refresh: Duration::from_micros(33_333), pending: None, last: None, last_serial: None,
            due_rate: None, due_cap: None,
        }
    }
    /// Stamp a completed render and advance both schedules.
    ///
    /// `max(slot + interval, now)` is the resync: if the pane stalled long enough
    /// that the next slot is already in the past, the missed slots are DROPPED
    /// rather than fired back-to-back to catch up.
    pub fn mark_rendered(&mut self, serial: u64, rate: Option<Duration>, cap: Option<Duration>) {
        let now = Instant::now();
        self.last = Some(now);
        self.last_serial = Some(serial);
        let advance = |slot: Option<Instant>, step: Option<Duration>| {
            step.map(|i| slot.map_or(now + i, |d| (d + i).max(now)))
        };
        self.due_rate = advance(self.due_rate, rate);
        self.due_cap = advance(self.due_cap, cap);
    }

    /// Advance to the next slot after a successful publish.
    pub fn advance(&mut self) {
        self.cursor = self.slots.next_after(self.cursor);
    }
}

/// The compositor's view: this pane's newest fully-rendered buffer and its
/// generation, or `None` before its first frame completes.
pub fn latest(registry: &Registry, pane: u64) -> Option<(Dmabuf, usize)> {
    let map = registry.lock().ok()?;
    let (buf, generation) = map.get(&pane)?.latest()?;
    Some((buf.clone(), generation))
}
