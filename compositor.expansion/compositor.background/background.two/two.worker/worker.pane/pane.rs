//! Per-pane worker state, and the registry the compositor reads it through.
//!
//! One pane per background draw target. Panes must NOT share a buffer: each has
//! its own camera and size, so on multi-monitor a shared one is the wrong
//! resolution and view for all but one. They do share a device and pipeline
//! cache — same shader, one GPU queue.

use compositor_background_two_worker_buffers::buffers::Slots;
use compositor_background_two_worker_key::key::PaneKey;
use compositor_background_two_worker_target::target::Target;
use smithay::backend::allocator::dmabuf::Dmabuf;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
/// Published as panes are allocated; read by the compositor to find a pane's
/// newest finished buffer. Absent until first allocation, and the compositor
/// then draws no background for that pane.
pub type Registry = Arc<Mutex<HashMap<PaneKey, Arc<Slots>>>>;

/// Host-side results the worker produces FOR the compositor, per pane.
///
/// The return path. Everything else about this feature flows one way — the
/// compositor states a pane's request, the worker renders it, the compositor
/// samples the dmabuf — and that covers pixels. It does not cover a pass that
/// computes something the COMPOSITOR has to act on, and there is already one:
/// a bundle declaring `evaluate: "map"` produces a warp grid, and the pointer
/// has to be un-warped by it on the compositor thread. A fully-offloaded bundle
/// renders that grid on the worker's device, so the value exists only over here.
///
/// It was a process-global `RwLock` in `kernel.graphic/bridge.window/warp`, which
/// two producers wrote — the compositor's own warp map and this one — racing for
/// one 128 KB slot with no way to tell whose grid was in it.
///
/// Keyed per PANE and reached through the `Worker` the world's own instance
/// holds, so it is neither global nor guessed: the reader already knows which
/// pane it is asking about. Kept deliberately open — readback is the shape any
/// future "the shader computed something" answer wants, not just this one.
#[derive(Clone, Default)]
pub struct Readback {
    /// The GPU warp grid this pane's bundle produced, one frame old. Row-major
    /// `EDGE`x`EDGE` source-UV pairs; `None` until a frame has produced one, and
    /// cleared when the bundle that asked for it goes — a stale grid would
    /// displace the pointer by an effect that is no longer on screen.
    pub warp_grid: Option<Arc<Vec<[f32; 2]>>>,
}

/// The readback map, shared with the worker thread exactly as [`Registry`] is.
pub type Readbacks = Arc<Mutex<HashMap<PaneKey, Readback>>>;

pub struct Pane {
    pub targets: Vec<Target>,
    /// This pane's multipass executor: intermediate targets + per-pass pipelines
    /// on the worker device. PER PANE because the intermediates are sized from
    /// the pane's extent — one shared executor would rebuild every time the serve
    /// loop moved between panes of different size.
    pub graph: compositor_pipeline_execute_graph_base::graph::GraphExec,
    /// This pane's GPU warp-map producer, for a bundle that declared
    /// `evaluate: "map"`. PER PANE for the same reason the executor is: it owns
    /// device objects on the worker's device.
    ///
    /// It exists here at all because a fully-offloaded bundle never reaches the
    /// compositor's producer — the graph runs HERE, so the only place that can
    /// render the grid is here too.
    pub warpmap: Option<compositor_pipeline_execute_warpmap_base::base::WarpMap>,
    /// This pane's `history` image — the previous PASS's `content`, derived on
    /// the worker rather than transported. Allocated only while a bundle samples
    /// the built-in `history` target, and freed the moment one stops, so a bundle
    /// that never asks for it costs nothing. See `worker.history`.
    pub history: Option<compositor_background_two_worker_history::history::History>,
    /// This pane's own render count, driving per-pass `cadence`. Per pane, not
    /// global: panes render at their own rates, so a shared counter would make a
    /// `cadence: 2` pass fire at a rate that depended on the OTHER monitors.
    pub tick: u64,
    /// Id of the multipass graph this pane last rendered, so taking the worker's
    /// graph path is announced ONCE per pane per bundle rather than per frame.
    /// Without it there is no positive signal that offload happened at all — the
    /// verdict is logged at load, which only says what was ELIGIBLE.
    pub last_pipeline: u64,
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
            graph: Default::default(),
            warpmap: None,
            history: None,
            tick: 0,
            last_pipeline: 0,
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
pub fn latest(registry: &Registry, pane: &PaneKey) -> Option<(Dmabuf, usize)> {
    let map = registry.lock().ok()?;
    let (buf, generation) = map.get(pane)?.latest()?;
    Some((buf.clone(), generation))
}
