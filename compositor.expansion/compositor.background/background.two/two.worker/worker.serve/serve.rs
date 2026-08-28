//! The worker thread body: paces over the live panes at its own rate (see
//! `worker.signal`).

use compositor_background_two_draw_vulkan::vulkan::ParallaxPass;
use compositor_background_two_worker_device::device::Device;
use compositor_background_two_worker_pace::pace;
use compositor_background_two_worker_pane::pane::{Pane, Readback, Readbacks, Registry};
use compositor_background_two_worker_pipeline::pipeline::Passes;
use compositor_background_two_worker_signal::signal::{DrawRequest, Signal};
use compositor_background_two_worker_target::target::Config;
use compositor_background_two_worker_key::key::{PaneKey, Region};
use compositor_kernel_graphic_bridge_publish_attempt::attempt::Attempt;
use compositor_pipeline_bundle_require_base::require::Requirement;
use compositor_model_environment_background_base::base::TripleBufferBackground;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

/// Backstop for the ack wait: a pane is due, but the compositor has not taken
/// the frame we last published, so rendering again would overwrite something
/// nobody has read.
///
/// A BACKSTOP, not a poll interval. `Signal::acknowledged` wakes us the moment
/// the compositor takes the frame, which on a running desktop is every frame —
/// so this should never be what ends the wait. It used to be a ONE MILLISECOND
/// nap, i.e. a thousand wakeups a second spent asking whether an event that the
/// compositor already knows about had happened yet.
///
/// HALF A FRAME, not a tenth of a second. It should never fire — the ack arrives
/// on every composite — but it also gates visible animation: the pass it releases
/// is the one that picks up a new camera, so anything the ack path misses shows
/// up as the background stuttering rather than as a missed wakeup. Short enough
/// that the worst case matches the millisecond poll this replaced, long enough
/// that it costs a fraction of its wakeups.
const ACK_BACKSTOP: Duration = Duration::from_millis(8);

/// Backstop for a stall: every live pane is gated off after a refused
/// allocation, so there is nothing this loop can do.
///
/// The gate reopens only when the request changes — a resize, which
/// `Signal::ping` wakes us for, or a settings change to the ring depth, which
/// nothing signals. This backstop exists for that second case alone, so it can
/// be long: a second's extra latency on recovering a surface that is already
/// failing costs nothing, and polling for it costs a wakeup every millisecond
/// for as long as the fault lasts.
const STALLED_BACKSTOP: Duration = Duration::from_secs(1);

/// BACKSTOP ONLY. Every real way a pane disappears is reported explicitly — a
/// monitor removal and a world leaving the screen both arrive through
/// `publish.retire`, a viewport collapse through the region count on every
/// request — so this timeout should never be what frees anything. It exists for
/// the case none of them covers: a caller that stops drawing a pane without
/// going through any, where the alternative is leaking a fullscreen dmabuf per
/// slot for the session. An overlay backdrop (picker, lock, overview) closing is
/// the one such case left; it is bounded at one pane per namespace per output.
///
/// Long, because being late costs memory and being early costs a full ring
/// reallocation on a pane that was only briefly not drawn.
const RETIRE: Duration = Duration::from_secs(30);


/// What determines whether a pane's buffers can be allocated at all. A failure
/// is remembered against this, and re-attempted only when it changes — see
/// `publish.attempt` for why a timed retry would be the wrong shape.
type Alloc = ((u32, u32), usize);

pub fn run(
    formats: compositor_kernel_graphic_format_registrar_base::registrar::Registrar,
    signal: Arc<Signal>,
    registry: Registry,
    readbacks: Readbacks,
    tx: mpsc::Sender<Result<(), String>>,
) {
    // FIRST, before any GPU work: this thread inherited the compositor's SCHED_RR
    // (see `loader.main/main.priority`), which would have a background shader
    // competing at realtime priority with the thread that dispatches input.
    compositor_kernel_graphic_bridge_publish_thread::thread::deprioritize("background worker");
    let device = match Device::new() {
        Ok(d) => d,
        Err(e) => return drop(tx.send(Err(e))),
    };
    // Resolved HERE and not in `Config`, because it needs the physical device —
    // which is created on this thread, deliberately, so no Vulkan object is
    // built on one thread and used from another.
    let cfg = Config {
        fourcc: compositor_background_two_worker_format::format::select(&formats, &device.phd),
    };
    if tx.send(Ok(())).is_err() {
        return;
    }
    let (mut passes, mut panes) = (Passes::new(), HashMap::<PaneKey, Pane>::new());
    // Per WORKER, not per pane: the same window can appear on several panes and
    // must import once.
    let mut imports = compositor_background_two_worker_import::import::Imports::new();
    // Per-pane record of the allocation that failed, so an identical request is
    // never re-issued. Without it a standing refusal — out of GPU memory, a size
    // the driver will not take — is retried on every pass of a loop whose idle
    // nap is a millisecond: a thousand failed allocations and log lines a second.
    let mut failed = HashMap::<PaneKey, Attempt<Alloc>>::new();
    // When each absent pane was last seen live, for the backstop sweep below.
    let mut absent = HashMap::<PaneKey, Instant>::new();
    let start = Instant::now();
    // Our cursors into the two retirement mailboxes — panes are keyed by output
    // AND by world, so both axes have to be reclaimable.
    let mut retired_cursor = compositor_kernel_graphic_bridge_publish_retire::retire::retired_epoch();
    let mut world_cursor =
        compositor_kernel_graphic_bridge_publish_retire::retire::worlds_retired_epoch();
    let mut overlay_cursor =
        compositor_kernel_graphic_bridge_publish_retire::retire::overlays_retired_epoch();
    while let Some(live) = signal.live(!panes.is_empty()) {
        // Snapshot BEFORE the pass: a ping or an ack that lands while we work
        // must not be slept through. `park` compares against this.
        let seq = signal.progress();
        // Re-read every pass: the settings window republishes on save.
        let tb = compositor_model_environment_background_base::base::get();
        let slots = compositor_background_two_worker_buffers::buffers::slots(tb.slots);
        let mut drew = false;
        // Live panes skipped because this exact allocation already failed. When
        // that is all of them there is nothing this loop can do until a request
        // changes, and it must not busy-wait for that.
        let mut blocked = 0usize;
        let seen: HashSet<PaneKey> = live.iter().map(|(k, _)| k.clone()).collect();
        // A pane collapsed away by a viewport change, retired now rather than on
        // the backstop.
        //
        // ONLY ASK A WORLD ABOUT ITS OWN PANES, ON ITS OWN OUTPUT. A group absent
        // from `live` has told us nothing — it simply is not drawing this pass,
        // which happens constantly: the picker owns the frame while the world
        // behind it does not draw, a monitor's pass is skipped, an overlay takes
        // over. Treating absence as "collapsed" freed a whole ring every time the
        // active pass changed, and the compositor then had no published frame to
        // sample, so the background VANISHED for the frames until the worker
        // refilled it — which is a hole where a backdrop should be, not a slow
        // backdrop.
        //
        // The WORLD is part of the grouping, not just the output: a region count
        // describes one world's viewport, so letting another world's count speak
        // for these panes would retire a split world's panes whenever an unsplit
        // one happened to be the thing drawing.
        //
        // Absence is what the removal mailboxes and the timeout backstop are for.
        let collapsed = |k: &PaneKey| {
            if seen.contains(k) {
                return false;
            }
            // An overlay backdrop has no region index, so no region count can
            // speak for it. It retires with its output or with its world.
            let Region::Viewport(index) = k.region else {
                return false;
            };
            let mut siblings = live
                .iter()
                .filter(|(o, _)| o.output == k.output && o.world == k.world)
                .peekable();
            // This world is not drawing on this output at all: not our call.
            if siblings.peek().is_none() {
                return false;
            }
            siblings.all(|(_, r)| index as usize >= r.regions)
        };
        for (pane, req) in live.iter() {
            let req = req.clone();
            let request: Alloc = (req.size, slots);
            let gate = failed.entry(pane.clone()).or_default();
            if !gate.worth_trying(&request) {
                blocked += 1;
                continue;
            }
            match serve_pane(&device, &mut passes, &mut imports, &mut panes, &registry, &readbacks, &cfg, &tb, pane, req, start) {
                Ok(did) => {
                    drew |= did;
                    failed.entry(pane.clone()).or_default().succeeded();
                }
                Err(e) => {
                    if failed.entry(pane.clone()).or_default().failed(request) {
                        warn!(
                            "background worker: pane {pane} dropped at {}x{} slots={slots} ({e}); \
                             not retried until the size or depth changes",
                            request.0.0, request.0.1
                        );
                    }
                }
            }
        }
        // Reclaim. Three signals, in descending certainty: an output the
        // compositor explicitly removed, a region index past its output's
        // current count, and only then the timeout backstop. Their buffers are
        // the largest item this feature owns (a fullscreen dmabuf per slot per
        // pane) and nothing else frees them — the signal drops its entry, but
        // the targets, the registry entry and the `Arc<Slots>` the compositor
        // may still hold all live here.
        // One relaxed atomic load in the common case — this loop can run at a
        // kilohertz, so it must not lock and clone to learn that nothing changed.
        if compositor_kernel_graphic_bridge_publish_retire::retire::retired_epoch() != retired_cursor {
            let (gone_outputs, next) =
                compositor_kernel_graphic_bridge_publish_retire::retire::retired_since(retired_cursor);
            retired_cursor = next;
            for output in gone_outputs {
                // An exact compare against the key the kernel published. This
                // used to be a hash of the output half of a packed integer, and
                // an overlay's namespace was baked into the string that was
                // hashed — so picker, lock and overview panes never matched and
                // leaked a fullscreen dmabuf per slot until the backstop. The
                // namespace now lives in `Region`, and this compares the output.
                let gone: Vec<PaneKey> = panes
                    .keys()
                    .filter(|k| k.output.as_ref() == output.as_str())
                    .cloned()
                    .collect();
                free(&device, &mut panes, &registry, &readbacks, &mut absent, &mut failed, &gone, "output removed");
            }
        }
        // The other axis: a world that left the screen. Its panes hold the largest
        // items here — targets, a `history` image, a persistent ping-pong pair —
        // and nothing else will ever draw them, so waiting out the backstop would
        // mean several worlds' rings resident at once after a trip through the
        // picker.
        if compositor_kernel_graphic_bridge_publish_retire::retire::worlds_retired_epoch() != world_cursor {
            let (gone_worlds, next) =
                compositor_kernel_graphic_bridge_publish_retire::retire::worlds_retired_since(world_cursor);
            world_cursor = next;
            for world in gone_worlds {
                let gone: Vec<PaneKey> = panes
                    .keys()
                    .filter(|k| k.world.as_u128() == world)
                    .cloned()
                    .collect();
                // Logged even at ZERO, and deliberately: `free` returns silently on
                // an empty list, so without this "the signal never arrived" and
                // "the signal arrived and matched nothing" look identical in the
                // log — and they have completely different causes.
                info!(
                    "background worker: world {world:032x} retired — {} of {} pane(s) match",
                    gone.len(),
                    panes.len()
                );
                free(&device, &mut panes, &registry, &readbacks, &mut absent, &mut failed, &gone, "world retired");
            }
        }
        // The third axis: an overlay backdrop that closed. Namespace-wide — the
        // picker is up or it is not, on every monitor at once — so this needs no
        // per-output or per-world qualification, and it is the signal that keeps
        // the timeout below from being the thing that reclaims a picker ring.
        if compositor_kernel_graphic_bridge_publish_retire::retire::overlays_retired_epoch() != overlay_cursor {
            let (gone_overlays, next) =
                compositor_kernel_graphic_bridge_publish_retire::retire::overlays_retired_since(overlay_cursor);
            overlay_cursor = next;
            for ns in gone_overlays {
                let gone: Vec<PaneKey> = panes
                    .keys()
                    .filter(|k| matches!(k.region, Region::Overlay(n) if n == ns))
                    .cloned()
                    .collect();
                free(&device, &mut panes, &registry, &readbacks, &mut absent, &mut failed, &gone, "overlay closed");
            }
        }
        let gone: Vec<PaneKey> = panes.keys().filter(|k| collapsed(k)).cloned().collect();
        free(&device, &mut panes, &registry, &readbacks, &mut absent, &mut failed, &gone, "region collapsed");
        retire(&device, &mut panes, &registry, &readbacks, &mut absent, &mut failed, &seen);
        // Evict imports the compositor has dropped, EVERY PASS.
        //
        // Here rather than inside `render_chain`, which runs only for a pane
        // actively rendering an offloaded graph: the import cache is per-WORKER,
        // shared by every pane, so nothing about eviction belongs to one pane's
        // render. `free` releases a pane's targets and never looks at the cache.
        //
        // Cheap by construction: one liveness check per entry, over a map that
        // holds only live drawables in the steady state.
        //
        // NOTE: this cache measured empty in every session instrumented, so the
        // move is a structural correction rather than a fix for observed growth.
        imports.reap();
        // Free what `reap` evicted, but only while every pane's last submit has
        // retired — the import cache is shared, so one pane's fence cannot speak
        // for another's in-flight frame.
        if panes.values().all(|p| p.pending.is_none_or(|r| device.signalled(p.fences[r]))) {
            imports.release(&device);
        }
        // Live entries are re-read, not consumed, so the loop must yield or spin.
        //
        // `until_any_due` returns `None` both for "a pane is due right now" and
        // for "there are no panes", and the second case is what a permanently
        // refused allocation leaves behind — every pane gated, none allocated,
        // nothing to pace against. Left on `IDLE_NAP` that woke the thread a
        // thousand times a second forever.
        let stalled = live.is_empty() || blocked == live.len();
        match pace::until_any_due(&panes, &tb) {
            // A real render deadline: waiting on TIME, not on the compositor.
            //
            // `for_ack` is false deliberately. An ack fires once per pane per
            // composite, so waking on one puts this thread on the COMPOSITE rate:
            // thousands of passes a second on a fast host, each a full pass —
            // lock, clone the live map, read the settings, build the seen set,
            // sweep, lock again — plus mutex contention with the compositor twice
            // per composite, and every one of them can only conclude "not due
            // yet" and park again.
            //
            // Tried both ways against the slow-picker symptom; the ack wake made
            // no difference to it, so the cheap reading of a deadline stands.
            Some(n) => signal.park(seq, n, false),
            None if drew => {}
            None if stalled => signal.park(seq, STALLED_BACKSTOP, false),
            // Due, but held by the ack handshake. This is THE case that used to
            // poll; it now sleeps until the compositor takes the frame.
            None => signal.park(seq, ACK_BACKSTOP, true),
        }
    }
    passes.destroy(&device.dev);
    imports.destroy_all(&device);
    for (_, mut p) in panes {
        device.free_ring(&p.cmds, &p.fences);
        // The pane's graph owns intermediate images on this device; freeing the
        // slots alone would leak them (a full-res set per pane).
        p.graph.destroy(&device.dev);
        if let Some(h) = p.history.take() {
            h.destroy(&device);
        }
        for t in p.targets { t.destroy(&device.dev); }
    }
}

/// The BACKSTOP sweep: panes absent from the live set for longer than
/// [`RETIRE`]. Every way a pane really disappears is reported explicitly and has
/// already run by here, so anything this catches is a caller that stopped
/// drawing without saying so — which is to say, nothing we know of. It stays
/// deliberately vague and deliberately long.
fn retire(
    device: &Device,
    panes: &mut HashMap<PaneKey, Pane>,
    registry: &Registry,
    readbacks: &Readbacks,
    absent: &mut HashMap<PaneKey, Instant>,
    failed: &mut HashMap<PaneKey, Attempt<Alloc>>,
    seen: &HashSet<PaneKey>,
) {
    let now = Instant::now();
    absent.retain(|k, _| !seen.contains(k));
    let stale: Vec<PaneKey> = panes
        .keys()
        .filter(|k| !seen.contains(*k))
        .filter(|k| now.duration_since(*absent.entry((*k).clone()).or_insert(now)) >= RETIRE)
        .cloned()
        .collect();
    free(device, panes, registry, readbacks, absent, failed, &stale, "not drawn");
}

/// Release `keys`. The registry entry goes FIRST, so the compositor cannot pick
/// up a handle to memory about to be freed; it then draws no background for that
/// key, which is already the pre-first-frame behaviour.
fn free(
    device: &Device,
    panes: &mut HashMap<PaneKey, Pane>,
    registry: &Registry,
    readbacks: &Readbacks,
    absent: &mut HashMap<PaneKey, Instant>,
    failed: &mut HashMap<PaneKey, Attempt<Alloc>>,
    keys: &[PaneKey],
    why: &str,
) {
    if keys.is_empty() {
        return;
    }
    if let Ok(mut map) = registry.lock() {
        for k in keys {
            map.remove(k);
        }
    }
    if let Ok(mut map) = readbacks.lock() {
        for k in keys {
            map.remove(k);
        }
    }
    for k in keys {
        absent.remove(k);
        failed.remove(k);
        if let Some(mut p) = panes.remove(k) {
            info!("background worker: pane {k} freed ({}x{}, {why})", p.size.0, p.size.1);
            device.free_ring(&p.cmds, &p.fences);
            p.graph.destroy(&device.dev);
            if let Some(h) = p.history.take() {
                h.destroy(device);
            }
            for t in p.targets {
                t.destroy(&device.dev);
            }
        }
    }
}

/// `Ok(true)` when a frame was rendered and published.
#[allow(clippy::too_many_arguments)]
fn serve_pane(
    device: &Device, passes: &mut Passes,
    imports: &mut compositor_background_two_worker_import::import::Imports,
    panes: &mut HashMap<PaneKey, Pane>, registry: &Registry, readbacks: &Readbacks,
    cfg: &Config, tb: &TripleBufferBackground, key: &PaneKey, req: DrawRequest, start: Instant,
) -> Result<bool, String> {
    compositor_background_two_worker_ensure::ensure::ensure(
        panes, registry, device, cfg, key, req.size,
        compositor_background_two_worker_buffers::buffers::slots(tb.slots),
    )?;
    let pane = panes.get_mut(key).ok_or("pane vanished")?;
    pane.refresh = req.refresh;
    // Retire a pipelined frame if the GPU finished it. POLLED, never waited.
    if pane.pending.is_some_and(|r| device.signalled(pane.fences[r])) {
        pane.pending = None;
        pane.slots.publish();
    }
    // Ack before the rate: rendering over an unread frame is waste.
    if !pane.slots.drained() || !pace::due(pane, tb, req.serial) {
        return Ok(false);
    }
    // Wall-clock, NOT a frame counter — the worker's rate is its own.
    let mut u = req.uniforms;
    // Shared with the compositor's own `t` and with the published window
    // timestamps — see `window.clock`. The worker's own `start` would have made
    // an offloaded bundle read a different clock than an inline one.
    u.time = compositor_pipeline_abi_clock_base::base::now();
    // The pane's OWN command slot, which is its buffer slot. No global ring, so
    // this never waits on another monitor's frame.
    let slot = pane.cursor;
    let (cmd, fence) = (pane.cmds[slot], pane.fences[slot]);
    let size = pane.size;
    // Split the borrow: the executor is `&mut` while the slot image is `&`, and
    // both are fields of the same pane.
    let Pane { targets, graph, warpmap, tick, last_pipeline, history: hist, .. } = pane;
    *tick = tick.wrapping_add(1);
    let tick = *tick;
    let target = &targets[slot];

    match &req.pipeline {
        // A fully offloadable graph: run every pass here, on this device. Only
        // `Offload::Whole` bundles are sent (see `worker_can_render`), so there is
        // no window set to bind and no composited `content` to sample.
        Some(cp) => {
            let seam =
                compositor_pipeline_build_seam_base::base::multipass_pass(cp, &u, &req.params);
            // The seam carries the bundle as an opaque handle so orchestration
            // need not name one; this is the other renderer that understands it.
            let gp = match compositor_pipeline_abi_seam_base::base::as_pipeline(
                seam.pipeline.as_ref(),
            ) {
                Some(p) => compositor_pipeline_execute_graph_base::graph::from_seam(p.clone()),
                None => return Err("multipass request carried no pipeline".into()),
            };
            // Stage 4 inverts which band travels: the compositor composited
            // background AND windows into `content`, and the worker decorates it.
            let after_band =
                cp.offload == compositor_pipeline_build_place_base::place::Offload::AfterBand;
            let after = match after_band {
                false => None,
                true => {
                    // Carried on the request, from the world this pane draws —
                    // these were two process-global slots, which on two monitors
                    // handed the worker whichever output composited last.
                    //
                    // Absent = the compositor has not completed a frame since this
                    // bundle was selected. Skip rather than decorate a band that
                    // does not exist.
                    let Some(cs) = req.content_share.as_ref() else { return Ok(false) };
                    let content = imports.shared_view(device, cs)?;
                    // The window layer is exported only while a bundle samples
                    // it, so its absence when a pass DOES name it means the
                    // compositor has not allocated the layer yet — one frame of
                    // waiting, not an error.
                    let windows = match gp.requires.has(Requirement::WindowLayer) {
                        false => ash::vk::ImageView::null(),
                        true => match req.windows_share.as_ref() {
                            None => return Ok(false),
                            Some(ws) => imports.shared_view(device, ws)?,
                        },
                    };
                    // Allocated on demand and released the moment a bundle stops
                    // asking: this is a full-screen image per pane, which is too
                    // much to hold for a bundle that has no temporal pass.
                    match gp.requires.has(Requirement::PreviousFrame) {
                        true => compositor_background_two_worker_history::history::ensure(
                            hist, device, size, target.format,
                        )?,
                        false => {
                            if let Some(h) = hist.take() {
                                h.destroy(device);
                            }
                        }
                    }
                    Some(compositor_background_two_worker_chain::chain::After {
                        content,
                        windows,
                        history: hist,
                    })
                }
            };
            if *last_pipeline != gp.id {
                *last_pipeline = gp.id;
                info!(
                    "background worker: pane running graph offthread \
                     ({} passes, {} targets, {}x{})",
                    gp.passes.len(), gp.targets.len(), size.0, size.1
                );
            }
            let warp_grid = compositor_background_two_worker_chain::chain::render_chain(
                device, graph, imports, target, &gp, size, cmd, fence, tick, warpmap,
                req.world_set.as_deref(), after,
            )?;
            // This pane's return path. The compositor's pointer warp reads it off
            // the `Worker` the world's own instance holds — see `pane::Readback`.
            if let Some(grid) = warp_grid {
                if let Ok(mut map) = readbacks.lock() {
                    map.entry(key.clone()).or_insert_with(Readback::default).warp_grid = grid;
                }
            }
        }
        None => {
            // Not a graph this pass: forget the announcement so switching back to
            // a multipass bundle says so again. Otherwise selecting an `-inline`
            // twin and returning leaves the worker running a graph with nothing
            // in the log to show for it.
            *last_pipeline = 0;
            // And release the intermediates the previous bundle built. The pane
            // itself survives the switch — the parallax still draws through it —
            // so nothing else would ever free them, and `prepare` only releases
            // while building the NEXT graph. Mirrors the compositor-side release
            // in `renderer.core`'s `submit_pass`.
            if graph.release(&device.dev) {
                info!("background worker: pane {size:?} left multipass — graph intermediates released");
            }
            let holder = req
                .module
                .is_none()
                .then(|| ParallaxPass::new(&u, &req.params, req.optimized));
            let pass = match (&req.module, &holder) {
                (Some(m), _) => {
                    compositor_background_two_draw_select::loaded_pass(m, &u, &req.params)
                }
                (None, Some(h)) => h.pass(),
                (None, None) => return Err("parallax pass missing".into()),
            };
            compositor_background_two_worker_render::render::render(
                device, passes, target, &pass.sdr, size, cmd, fence,
            )?;
        }
    }
    // Publish ONLY once the fence signalled — keeps the compositor off a buffer
    // still being written. Pipelined, that wait defers to the next pass.
    //
    // CLAMPED, exactly as `Ring::submitted` clamps it: deferring needs a slot
    // that is neither published nor being written, and at depth two there is
    // none. Honouring `pipeline` there would have the next render wrap straight
    // onto the buffer the compositor is reading. The knob is inert at two, not
    // wrong — same rule, same reason, on both sides of the feature.
    match tb.pipeline && compositor_background_two_worker_buffers::buffers::can_pipeline(pane.slots.len()) {
        true => pane.pending = Some(slot),
        false => {
            device.complete(fence)?;
            pane.slots.publish();
        }
    }
    pane.advance();
    pane.mark_rendered(req.serial, tb.rate.min_interval(pane.refresh), tb.cap(pane.refresh));
    Ok(true)
}
