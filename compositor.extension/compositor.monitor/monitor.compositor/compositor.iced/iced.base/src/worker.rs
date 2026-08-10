//! The iced worker: the channel, the type erasure, and the thread body.
//!
//! `iced_wgpu::Renderer` is `!Send` — it holds the shared text atlas and pipeline
//! caches — so unlike bevy's context it cannot be handed across. It is therefore
//! CONSTRUCTED on the worker, from the wgpu device/adapter/queue (which are all
//! `Send + Sync`). Everything else follows bevy's shape: `Job::Create` carries a
//! factory rather than a runtime, so nothing `!Send` ever crosses.

use crate::handle::HandleId;
use crate::publish::{Board, Published};
use compositor_kernel_graphic_bridge_publish_ring::ring::Ring;
use compositor_kernel_graphic_bridge_publish_attempt::attempt::Attempt;
use compositor_model_debug_instance_record::{error, info, warn};
use compositor_model_environment_interface_base::base as interface;
use compositor_monitor_runtime_surface_base::{WgpuVulkanContext, WorkerSlot};
use compositor_support_iced_core_engine_base::{EngineSettings, IcedEvent, SharedEngine};
use smithay::utils::{Physical, Point, Size};
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

/// One instance's iced runtime, erased of its UI type so the worker can hold a
/// heterogeneous set without being generic over `U: IcedUi`.
pub trait AnyUi: 'static {
    fn queue_event(&mut self, event: IcedEvent);
    /// Payload is `U::Message`, boxed by the registry; the impl downcasts it back.
    fn queue_message(&mut self, message: Box<dyn Any + Send>);
    fn tick(&mut self) -> bool;
    fn is_dirty(&self) -> bool;
    fn render_into(&mut self, view: &wgpu::TextureView);
    fn resize(&mut self, size: (u32, u32), scale: f32);
    fn acknowledge_frame(&mut self);
    /// Install the compositor's observer, already wrapped so it takes `&dyn Any`.
    fn set_handler(&mut self, handler: Box<dyn FnMut(&dyn Any) + Send>);
    /// A shareable copy of whatever compositor-side code reads synchronously.
    /// `None` for the UIs that are never read back, which is most of them.
    fn snapshot(&self) -> Option<Arc<dyn Any + Send + Sync>>;
}

/// Builds one runtime ON the worker thread, given the engine it constructed.
pub type Factory = Box<dyn FnOnce(&SharedEngine, (u32, u32), f32) -> Box<dyn AnyUi> + Send>;

pub enum Job {
    Create { id: HandleId, size: Size<i32, Physical>, scale: f32, location: Point<i32, Physical>, factory: Factory },
    Destroy(HandleId),
    Resize { id: HandleId, size: Size<i32, Physical>, scale: f32, location: Point<i32, Physical> },
    Event { id: HandleId, event: IcedEvent },
    Message { id: HandleId, payload: Box<dyn Any + Send> },
    Handler { id: HandleId, handler: Box<dyn FnMut(&dyn Any) + Send> },
    /// Compositor-side visibility. The worker owns the buffers, so it is the one
    /// that releases and re-allocates them; the compositor only says which
    /// surfaces are worth keeping resident.
    Visible { id: HandleId, visible: bool },
    /// One compositor frame. Coalesced, so a backlog queued while the GPU was
    /// busy still advances each instance exactly once.
    Tick,
}

/// `Clone` so ONE worker serves every world's registry. The thread, and the
/// `SharedEngine` (and image atlas) born on it, are the expensive parts — one
/// per world cost ~20 threads and ~20 atlases. Both fields are already handles:
/// `Sender` clones, and `Board` is `Arc`-backed.
#[derive(Clone)]
pub struct Worker {
    tx: Sender<Job>,
    board: Board,
}

impl Worker {
    pub fn board(&self) -> &Board {
        &self.board
    }
    /// `false` once the worker thread is gone.
    pub fn send(&self, job: Job) -> bool {
        self.tx.send(job).is_ok()
    }
}

/// Start THE worker, if the interface preference engages it. One per process:
/// every registry clones the returned handle. `None` leaves every caller on the
/// inline path, which is a supported mode rather than a degradation.
///
/// Vulkan-only, deliberately: the worker's ring slots carry no GLES view, because
/// building one needs `&mut GlesRenderer` — the single thing that cannot leave
/// the compositor thread. On GLES everything stays inline, as it always has.
pub fn spawn_shared(ctx: &Arc<WgpuVulkanContext>) -> Option<Worker> {
    let interface = compositor_model_environment_interface_base::base::get();
    if !interface.engaged() {
        if interface.enabled && !compositor_model_stats_registry_base::base::compositor_prefers_dmabuf() {
            info!("iced: triple buffering requested but the compositor is on GLES; staying inline");
        }
        return None;
    }
    spawn(ctx, compositor_model_environment_config_base::base::get().render_node.clone())
}

fn spawn(ctx: &Arc<WgpuVulkanContext>, render_node: String) -> Option<Worker> {
    let board = Board::new();
    let (tx, rx) = channel::<Job>();
    let (ctx, node, board_for_thread) = (ctx.clone(), render_node.clone(), board.clone());
    match std::thread::Builder::new()
        .name("y5-iced-worker".into())
        .spawn(move || run(rx, board_for_thread, ctx, node))
    {
        Ok(_) => {
            info!("iced worker: thread spawned for {render_node}");
            Some(Worker { tx, board })
        }
        Err(e) => {
            error!("iced worker: thread spawn failed ({e}); staying inline");
            None
        }
    }
}

struct Hosted {
    ui: Box<dyn AnyUi>,
    /// `None` while the backing is released to reclaim memory off-screen. The
    /// runtime keeps ticking regardless, exactly as the inline path does.
    ring: Option<Ring<WorkerSlot>>,
    size: Size<i32, Physical>,
    visible: bool,
    /// Ticked while off-screen, so it re-renders once on reveal.
    stale: bool,
    /// The world location paired with `size` — echoed back on every publish so
    /// the compositor can draw the rect this frame was actually rendered for.
    location: Point<i32, Physical>,
    /// Monotonic count of frames handed to the board, for THIS instance's whole
    /// life. Never reset.
    ///
    /// Deliberately not the ring's own generation. That one restarts at zero
    /// whenever the ring is rebuilt — a resize, or a release/re-ensure — and the
    /// consumer compares it against the last value it saw to decide whether to
    /// damage. Restart it and the first frame at the new size can land on the
    /// same number as the last frame at the old one, so the consumer concludes
    /// nothing changed and never repaints. During a drag that repeats every step
    /// and the surface freezes.
    seq: u64,
    /// The ring-depth request that failed, if any. `want` does not change when
    /// the allocation fails, so an ungated resize re-issues the same refused
    /// request every tick — the one call here that can storm.
    depth_failed: Attempt<(usize, Size<i32, Physical>)>,
}

fn run(rx: Receiver<Job>, board: Board, ctx: Arc<WgpuVulkanContext>, render_node: String) {
    // FIRST: this thread inherited the compositor's SCHED_RR, which would have
    // iced's rasterization competing at realtime priority with the thread that
    // dispatches input.
    compositor_kernel_graphic_bridge_publish_thread::thread::deprioritize("iced worker");
    // The `Renderer` is born here and dies here. This is the whole reason the
    // engine is built on the worker rather than handed to it.
    let engine = SharedEngine::new(
        &ctx.adapter,
        Arc::new(ctx.device.clone()),
        Arc::new(ctx.queue.clone()),
        compositor_monitor_runtime_surface_base::TEXTURE_FORMAT,
        EngineSettings::default(),
    );
    let mut hosted: HashMap<HandleId, Hosted> = HashMap::new();
    info!("iced worker: started on {render_node}");
    while let Ok(first) = rx.recv() {
        let mut tick = apply(&mut hosted, &board, &engine, &ctx, &render_node, first);
        while let Ok(job) = rx.try_recv() {
            tick |= apply(&mut hosted, &board, &engine, &ctx, &render_node, job);
        }
        if tick {
            // Render, then retire and publish — for EVERY instance, including
            // ones this tick skipped: a pipelined frame is published by the poll
            // and stranding it leaves the surface on stale pixels until the next
            // time it happens to be due.
            //
            // Each ring does its own wait inside `submitted`, as it always has.
            // Hoisting that to one wait for the batch is faster and asks the
            // caller to honour a contract the ring used to enforce itself, which
            // is the shape every regression in this file has had.
            let mut wants = false;
            for (id, h) in hosted.iter_mut() {
                // `submitted` retires synchronously when it cannot defer, so the
                // publish it performed is reported HERE and must be carried into
                // `retire` — its own `poll` would find nothing left and conclude
                // nothing happened, which is how every surface stopped appearing.
                let moved = advance(*id, h, &board, &ctx, &render_node);
                wants |= retire(*id, h, &board, moved);
            }
            board.set_wants(wants);
        }
    }
    info!("iced worker: stopped");
}

fn apply(
    hosted: &mut HashMap<HandleId, Hosted>,
    board: &Board,
    engine: &SharedEngine,
    ctx: &Arc<WgpuVulkanContext>,
    render_node: &str,
    job: Job,
) -> bool {
    match job {
        Job::Tick => return true,
        Job::Create { id, size, scale, location, factory } => {
            let ui = factory(engine, (size.w.max(1) as u32, size.h.max(1) as u32), scale);
            let ring = match build_ring(ctx, render_node, size) {
                Ok(r) => Some(r),
                Err(e) => {
                    error!("iced worker: create {id:?} backing failed: {e}");
                    None
                }
            };
            hosted.insert(id, Hosted { ui, ring, size, visible: true, stale: true, location, seq: 0, depth_failed: Attempt::new() });
        }
        Job::Destroy(id) => {
            hosted.remove(&id);
            board.remove(id);
            // Same deferred-destruction flush as the bevy worker's `Destroy`, for
            // the same reason: dropping the instance queues its ring's images,
            // and wgpu only performs the free when the device is polled. The
            // amounts here are smaller — surface-sized rather than output-sized —
            // but the retention is unbounded in exactly the same way.
            if let Err(e) = ctx.device.poll(wgpu::PollType::wait_indefinitely()) {
                warn!("iced worker: device poll after destroy failed: {e:?}");
            }
        }
        Job::Resize { id, size, scale, location } => {
            if let Some(h) = hosted.get_mut(&id) {
                h.location = location;
                if size != h.size && h.ring.is_some() {
                    match build_ring(ctx, render_node, size) {
                        Ok(r) => {
                            h.ring = Some(r);
                        }
                        Err(e) => warn!("iced worker: resize {id:?} failed: {e}"),
                    }
                }
                h.size = size;
                h.ui.resize((size.w.max(1) as u32, size.h.max(1) as u32), scale);
                h.stale = true;
            }
        }
        Job::Event { id, event } => {
            if let Some(h) = hosted.get_mut(&id) {
                h.ui.queue_event(event);
            }
        }
        Job::Message { id, payload } => {
            if let Some(h) = hosted.get_mut(&id) {
                h.ui.queue_message(payload);
            }
        }
        Job::Handler { id, handler } => {
            if let Some(h) = hosted.get_mut(&id) {
                h.ui.set_handler(handler);
            }
        }
        Job::Visible { id, visible } => {
            if let Some(h) = hosted.get_mut(&id) {
                h.visible = visible;
                match visible {
                    // Released while hidden; the runtime keeps ticking, and
                    // `stale` makes it repaint once on reveal.
                    false => h.ring = None,
                    true if h.ring.is_none() => {
                        h.ring = build_ring(ctx, render_node, h.size).ok();
                        h.stale = true;
                    }
                    true => {}
                }
            }
        }
    }
    false
}

fn build_ring(
    ctx: &Arc<WgpuVulkanContext>,
    render_node: &str,
    size: Size<i32, Physical>,
) -> Result<Ring<WorkerSlot>, String> {
    let depth = interface::get().depth().max(1);
    let mut slots = Vec::with_capacity(depth);
    for _ in 0..depth {
        slots.push(WorkerSlot::allocate(render_node, ctx, size).map_err(|e| format!("{e:?}"))?);
    }
    Ok(Ring::new(slots, ctx.device.clone(), ctx.queue.clone()))
}

/// One instance's frame: tick the runtime and submit. Publishing is `retire`'s
/// job, gated on the ring having actually advanced.
fn advance(
    id: HandleId,
    h: &mut Hosted,
    board: &Board,
    ctx: &Arc<WgpuVulkanContext>,
    render_node: &str,
) -> bool {
    let settings = interface::get();
    if let Some(ring) = h.ring.as_mut() {
        let want = settings.depth().max(1);
        // Gated on the request: `want` does not change when the allocation
        // fails, so an ungated resize re-issues the identical refused request on
        // EVERY tick — a failed allocation and a log line per composite, forever.
        let request = (want, h.size);
        if want != ring.len() && h.depth_failed.worth_trying(&request) {
            let size = h.size;
            match ring.set_depth(want, || WorkerSlot::allocate(render_node, ctx, size)) {
                Ok(()) => h.depth_failed.succeeded(),
                Err(e) => {
                    if h.depth_failed.failed(request) {
                        warn!("iced worker: ring resize to {want} at {size:?} failed ({e:?}); \
                               not retried until the depth or size changes");
                    }
                }
            }
        }
        ring.poll();
    }
    // Tick ALWAYS — animations and messages must advance whether or not the
    // surface is on-screen, which is what the inline path's `skip_render` does.
    let due = h.ui.tick();
    if let Some(snap) = h.ui.snapshot() {
        board.put_snapshot(id, snap);
    }
    let mut moved = false;
    match (h.visible, h.ring.as_mut()) {
        (true, Some(ring)) => {
            if due || h.stale {
                let view = ring.begin().view();
                h.ui.render_into(&view);
                moved = ring.submitted(settings.pipeline);
                h.stale = false;
            }
        }
        // Off-screen or released: acknowledge so a dirty runtime stops pinning
        // the redraw loop, and remember to repaint once it is visible again.
        _ => {
            if due {
                h.ui.acknowledge_frame();
                h.stale = true;
            }
        }
    }
    moved
}

/// Retire whatever finished and hand the newest buffer to the board. Runs for
/// EVERY instance, including ones this tick skipped: a deferred publish is
/// retired here or not at all, and stranding it leaves the surface on stale
/// pixels until the next time it happens to be due.
///
/// Returns whether the instance still wants another tick.
fn retire(id: HandleId, h: &mut Hosted, board: &Board, submitted_moved: bool) -> bool {
    let mut pending = false;
    if let Some(ring) = h.ring.as_mut() {
        // THE signal is whether this poll retired a frame, not whether the
        // generation differs from a number we remembered. A resize builds a
        // fresh ring that restarts at generation 0, so a remembered cursor
        // collides with it — the new frame comes back as generation 1, matches
        // the 1 left from before the resize, and is skipped as "already
        // published". During a drag that repeats every step and the surface
        // freezes at the old content and the old size. `poll` cannot be wrong
        // about this: it returns true exactly when it advanced the ring.
        let moved = ring.poll() || submitted_moved;
        pending = ring.has_pending();
        if moved {
            h.seq += 1;
            board.publish(id, Published {
                dmabuf: ring.published().dmabuf().clone(),
                generation: h.seq,
                size: h.size,
                location: h.location,
            });
            // Wake the compositor: an undamaged frame queues no flip and so
            // produces no vblank, and once this surface renders over here its
            // publish is the only event that can restart a stopped loop.
            // `wants()` alone is not enough — that is only read while a frame is
            // already being built.
            compositor_kernel_graphic_bridge_publish_wake::wake::notify_offthread_published();
        }
    }
    pending || (h.visible && h.ui.is_dirty())
}
