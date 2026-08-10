//! The worker thread: owns every `App`, every ring, and the wgpu work.
//!
//! Nothing here runs on the compositor thread, so a heavy bevy frame can no
//! longer delay input. Jobs arrive on a channel; finished buffers leave through
//! the `Board`. The loop BLOCKS between compositor frames rather than spinning —
//! `Job::Tick` is the only thing that advances a scene, and it is coalesced, so a
//! backlog of ticks queued while the GPU was busy still costs exactly one frame.

use compositor_kernel_graphic_bridge_publish_attempt::attempt::Attempt;
use compositor_kernel_graphic_bridge_publish_ring::ring::Ring;
use compositor_model_debug_instance_record::{error, info, warn};
use compositor_model_environment_interface_base::base as interface;
use compositor_support_bevy_core_context_base::WgpuVulkanContext;
use compositor_support_bevy_core_handle_base::HandleId;
use compositor_support_bevy_core_publish_base::{Board, Published};
use compositor_support_bevy_core_shared_base::SharedContext;
use compositor_support_bevy_core_worker_base::{AnyRuntime, Job};
use compositor_support_bevy_core_worker_slot::Slot;
use smithay::utils::{Physical, Size};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

struct Hosted {
    runtime: Box<dyn AnyRuntime>,
    ring: Ring<Slot>,
    size: Size<i32, Physical>,
    /// Next wall-clock slot for the rate gate, and for the cap gate.
    ///
    /// A SCHEDULE, not an elapsed-since-last test: it advances from the previous
    /// slot rather than from when a tick happened to finish, so a slow frame does
    /// not push the next one out and poll granularity does not accumulate. That is
    /// what lets the gates run with no tolerance term — and a tolerance is exactly
    /// what let a "1x refresh" cap admit 2x.
    due_rate: Option<Instant>,
    due_cap: Option<Instant>,
    /// Monotonic count of frames handed to the board, for THIS instance's whole
    /// life. Never reset.
    ///
    /// Deliberately not the ring's own generation, which restarts at zero on any
    /// rebuild (a resize calls `Ring::replace`). The consumer compares against
    /// the last value it saw to decide whether to damage, so a restart lets the
    /// first frame at the new size collide with the last at the old one — the
    /// consumer sees no change and never repaints.
    seq: u64,
    /// The ring depth that failed to allocate, if any. `want` does not change
    /// when the allocation fails, so an ungated resize would re-issue the same
    /// refused request on EVERY tick; keyed on `(depth, size)` it is re-tried
    /// only once one of them actually changes.
    depth_failed: Attempt<(usize, Size<i32, Physical>)>,
}

/// One gate: is `rate`, counted against `cadence`, satisfied for this instance?
///
/// `due_at` is this gate's next scheduled slot, used only by the wall-clock
/// branch. No tolerance term — see the note on [`Hosted::due_rate`].
fn gate(
    rate: interface::Rate,
    cadence: interface::Cadence,
    ticks: u64,
    refresh: std::time::Duration,
    due_at: Option<Instant>,
    reach: std::time::Duration,
) -> bool {
    match cadence {
        interface::Cadence::Vblank => ticks % interface::TripleBufferUI::every(rate, refresh) == 0,
        // Uncapped has no schedule; an instance that has never ticked is due now.
        interface::Cadence::Timer => match (rate.min_interval(refresh), due_at) {
            (Some(_), Some(d)) => Instant::now() + reach >= d,
            _ => true,
        },
    }
}

/// May a scene advance on this composite? Both gates must pass.
///
/// Rate and cap carry SEPARATE cadences because the useful pairing is a rate
/// counted in composites — phase-locked to the compositor, with no wall-clock
/// timer to drift against the retrace — under a cap expressed against the panel,
/// which means the same thing on any host. One field could not say that: it
/// forced the cap into the rate's space.
fn due(
    h: &Hosted,
    ticks: u64,
    settings: &interface::TripleBufferUI,
    refresh: std::time::Duration,
    reach: std::time::Duration,
) -> bool {
    gate(settings.rate, settings.cadence, ticks, refresh, h.due_rate, reach)
        && gate(settings.ceiling, settings.ceiling_cadence, ticks, refresh, h.due_cap, reach)
}

/// How far ahead of a slot a tick may still take it: half the gap between ticks.
///
/// Unlike the background worker, this one gets DISCRETE opportunities — one per
/// composite. A slot landing a hair after a tick would otherwise wait for the
/// next one, and when the cap period and the tick period are close (the ordinary
/// non-tearing case, both a refresh) that alternates render/skip and halves the
/// rate. Rounding to the NEAREST tick fixes it.
///
/// Scaled to the measured tick gap, never to the cap. That is the whole
/// difference from the tolerance this replaced: half a tick can only ever move a
/// render to the adjacent opportunity, whereas half a CAP let a 1x cap admit 2x.
fn reach(gap: Option<std::time::Duration>) -> std::time::Duration {
    gap.map_or(std::time::Duration::ZERO, |g| g / 2)
}

/// Advance both schedules after a tick. `max(slot + interval, now)` is the
/// resync: slots missed during a stall are DROPPED rather than fired back to
/// back to catch up.
fn schedule(h: &mut Hosted, settings: &interface::TripleBufferUI, refresh: std::time::Duration) {
    let now = Instant::now();
    let advance = |slot: Option<Instant>, step: Option<std::time::Duration>| {
        step.map(|i| slot.map_or(now + i, |d| (d + i).max(now)))
    };
    h.due_rate = advance(h.due_rate, settings.interval(refresh));
    h.due_cap = advance(h.due_cap, settings.cap(refresh));
}

pub fn run(
    rx: Receiver<Job>,
    board: Board,
    shared: SharedContext,
    ctx: Arc<WgpuVulkanContext>,
    render_node: String,
) {
    // FIRST: this thread inherited the compositor's SCHED_RR, which would have
    // bevy's ECS and render graph competing at realtime priority with the thread
    // that dispatches input — the very coupling this worker exists to break.
    compositor_kernel_graphic_bridge_publish_thread::thread::deprioritize("bevy worker");
    let mut hosted: HashMap<HandleId, Hosted> = HashMap::new();
    // Compositor frames seen. The divisor counts these, not wall time, so bevy
    // stays phase-locked to the composite instead of drifting against it.
    let mut ticks: u64 = 0;
    // Instant of the previous tick, for measuring the gap between opportunities.
    let mut prev_tick: Option<Instant> = None;
    info!("bevy worker: started on {render_node}");
    // Blocking recv: with no compositor frames there is nothing to advance, and
    // parking is what keeps an idle desktop idle.
    while let Ok(first) = rx.recv() {
        let mut tick = apply(&mut hosted, &board, &shared, &ctx, &render_node, first);
        while let Ok(job) = rx.try_recv() {
            tick |= apply(&mut hosted, &board, &shared, &ctx, &render_node, job);
        }
        if tick {
            ticks = ticks.wrapping_add(1);
            let settings = interface::get();
            let refresh = interface::refresh();
            let now = Instant::now();
            let reach = reach(prev_tick.map(|t| now.duration_since(t)));
            prev_tick = Some(now);
            // Advance the due scenes, then retire and publish for EVERY instance
            // — including ones this tick skipped. A pipelined frame is published
            // by the poll, and stranding it would leave the scene lurching at the
            // divisor's rate rather than simply running slower.
            //
            // Each ring waits inside `submitted`, as it always has. Hoisting that
            // to one wait for the whole batch is faster and moves an invariant
            // the ring used to enforce itself out into the caller, which is the
            // shape every regression here has had.
            for (id, h) in hosted.iter_mut() {
                // `submitted` retires synchronously when it cannot defer, so the
                // publish it performed is reported by `advance` and must be OR-ed
                // in here — a later `poll` finds nothing left and would conclude
                // nothing happened, which is how every scene stopped appearing.
                let mut moved = false;
                if due(h, ticks, &settings, refresh, reach) {
                    moved = advance(*id, h, &ctx, &render_node);
                }
                // Publish exactly when a frame was retired. A remembered
                // generation cannot be used: a resize replaces the ring and
                // restarts it at 0.
                if h.ring.poll() || moved {
                    publish(*id, h, &board);
                }
            }
        }
    }
    info!("bevy worker: stopped");
}

/// Returns whether the job was a tick.
fn apply(
    hosted: &mut HashMap<HandleId, Hosted>,
    board: &Board,
    shared: &SharedContext,
    ctx: &Arc<WgpuVulkanContext>,
    render_node: &str,
    job: Job,
) -> bool {
    match job {
        Job::Tick => return true,
        Job::Create { id, size, scale, factory } => {
            match spawn(shared, ctx, render_node, size, scale, factory) {
                // Creation failure is reported here and the instance simply never
                // appears. The compositor already returned its handle by now, so
                // there is nowhere left to hand an error back to.
                Err(e) => error!("bevy worker: create {id} failed: {e}"),
                Ok(h) => {
                    hosted.insert(id, h);
                }
            }
        }
        Job::Destroy(id) => {
            hosted.remove(&id);
            board.remove(id);
            // Dropping the `App` only QUEUES its GPU resources for destruction.
            // wgpu defers the actual `vkDestroyImage`/`vkFreeMemory` until the
            // device is polled and the GPU is known to be done with them, and
            // bevy's own render loop is the only thing that normally polls — from
            // inside a running App. Drop the last one and nothing polls again, so
            // the queue is never flushed and the memory is held for the life of
            // the process.
            //
            // That is the picker leak: open + close the picker without changing
            // world and roughly 300 MiB of view targets (`main_texture_*`,
            // `view_depth_texture` at output resolution) go unreclaimed per cycle.
            // It is also why only the overview ever gave the memory back — it
            // starts bevy work, which ticks an App, which polls, which flushes
            // every destruction queued since.
            //
            // `wait_indefinitely` rather than `Poll`: a non-blocking check drops
            // anything the GPU has not finished with yet, which on a teardown
            // immediately after a frame is exactly the resources being released.
            // This runs on the worker thread, only on close, so the wait is not on
            // any interactive path.
            if let Err(e) = ctx.device.poll(wgpu::PollType::wait_indefinitely()) {
                warn!("bevy worker: device poll after destroy failed: {e:?}");
            }
        }
        Job::Resize { id, size, scale } => {
            if let Some(h) = hosted.get_mut(&id) {
                if let Err(e) = resize(h, ctx, render_node, size, scale) {
                    warn!("bevy worker: resize {id} failed: {e}");
                }
            }
        }
        Job::Command { id, payload } => {
            if let Some(h) = hosted.get_mut(&id) {
                h.runtime.apply(payload);
            }
        }
    }
    false
}

fn spawn(
    shared: &SharedContext,
    ctx: &Arc<WgpuVulkanContext>,
    render_node: &str,
    size: Size<i32, Physical>,
    scale: f32,
    factory: compositor_support_bevy_core_worker_base::Factory,
) -> Result<Hosted, String> {
    let ring = build_ring(ctx, render_node, size, interface::get().depth())?;
    let first = Arc::new(ring.target().wgpu_texture.clone());
    let px = (size.w.max(1) as u32, size.h.max(1) as u32);
    // The `App` is constructed HERE, on the worker. That is the whole reason the
    // job carries a factory: bevy's `App` is `!Send` and could never be handed
    // across, but it never has to be.
    let runtime = factory(shared, first, px, scale);
    Ok(Hosted { runtime, ring, size, due_rate: None, due_cap: None, seq: 0, depth_failed: Attempt::new() })
}

fn build_ring(
    ctx: &Arc<WgpuVulkanContext>,
    render_node: &str,
    size: Size<i32, Physical>,
    depth: usize,
) -> Result<Ring<Slot>, String> {
    let mut slots = Vec::with_capacity(depth);
    for _ in 0..depth.max(1) {
        slots.push(Slot::allocate(render_node, ctx, size).map_err(|e| format!("{e:?}"))?);
    }
    Ok(Ring::new(slots, ctx.device.clone(), ctx.queue.clone()))
}

fn resize(
    h: &mut Hosted,
    ctx: &Arc<WgpuVulkanContext>,
    render_node: &str,
    size: Size<i32, Physical>,
    scale: f32,
) -> Result<(), String> {
    if size != h.size {
        let mut slots = Vec::with_capacity(h.ring.len());
        for _ in 0..h.ring.len() {
            slots.push(Slot::allocate(render_node, ctx, size).map_err(|e| format!("{e:?}"))?);
        }
        h.ring.replace(slots);
        h.size = size;
    }
    h.runtime.resize((size.w.max(1) as u32, size.h.max(1) as u32), scale);
    Ok(())
}

/// Hand the newest retired buffer to the board.
///
/// Called ONLY when `Ring::poll` reported that it advanced — that is the whole
/// gate. Comparing against a remembered generation instead looks equivalent and
/// is not: `resize` replaces the ring and restarts it at 0, so the first frame
/// at the new size comes back as generation 1 and collides with the 1 left over
/// from before, and is dropped as already published.
fn publish(id: HandleId, h: &mut Hosted, board: &Board) {
    h.seq += 1;
    board.publish(id, Published {
        dmabuf: h.ring.published().dmabuf().clone(),
        generation: h.seq,
        size: h.size,
    });
    // Wake the compositor. Its redraw loop is sustained by flip -> vblank ->
    // render, and a frame with no damage queues no flip — so once the scene
    // renders over here, this publish is the only thing that can restart a
    // stopped loop. Without it the picker wedged until the session was
    // re-activated.
    compositor_kernel_graphic_bridge_publish_wake::wake::notify_offthread_published();
}

/// One scene frame: claim a free slot, draw, submit. Publishing is the caller's,
/// gated on the ring having actually advanced — see `run`.
fn advance(
    h_id: HandleId,
    h: &mut Hosted,
    ctx: &Arc<WgpuVulkanContext>,
    render_node: &str,
) -> bool {
    let _ = h_id;
    let settings = interface::get();
    // Ring depth follows the live setting here rather than only at create/resize,
    // so the knob applies without restarting the session.
    let want = settings.depth();
    let request = (want, h.size);
    if want != h.ring.len() && h.depth_failed.worth_trying(&request) {
        let size = h.size;
        match h.ring.set_depth(want, || Slot::allocate(render_node, ctx, size)) {
            Ok(()) => h.depth_failed.succeeded(),
            Err(e) => {
                if h.depth_failed.failed(request) {
                    warn!("bevy worker: ring resize to {want} at {size:?} failed ({e:?}); \
                           not retried until the depth or size changes");
                }
            }
        }
    }
    h.ring.poll();
    h.ring.begin();
    if h.ring.take_retarget() {
        h.runtime.set_output_texture(Arc::new(h.ring.target().wgpu_texture.clone()));
    }
    h.runtime.update();
    schedule(h, &settings, interface::refresh());
    h.ring.submitted(settings.pipeline)
}
