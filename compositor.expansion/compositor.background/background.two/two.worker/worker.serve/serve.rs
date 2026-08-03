//! The worker thread body: paces over the live panes at its own rate (see
//! `worker.signal`).

use compositor_background_two_draw_vulkan::vulkan::ParallaxPass;
use compositor_background_two_worker_device::device::Device;
use compositor_background_two_worker_pace::pace;
use compositor_background_two_worker_pane::pane::{Pane, Registry};
use compositor_background_two_worker_pipeline::pipeline::Passes;
use compositor_background_two_worker_signal::signal::{DrawRequest, Signal};
use compositor_background_two_worker_target::target::Config;
use compositor_background_two_worker_key::key;
use compositor_kernel_graphic_bridge_publish_attempt::attempt::Attempt;
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

/// BACKSTOP ONLY. Both real ways a pane disappears are reported explicitly — a
/// monitor removal arrives through `Signal::drop_output`, a viewport collapse
/// through the region count on every request — so this timeout should never be
/// what frees anything. It exists for the case neither covers: a caller that
/// stops drawing a pane without going through either, where the alternative is
/// leaking a fullscreen dmabuf per slot for the session.
///
/// Long, because being late costs memory and being early costs a full ring
/// reallocation on a pane that was only briefly not drawn.
const RETIRE: Duration = Duration::from_secs(30);

/// What determines whether a pane's buffers can be allocated at all. A failure
/// is remembered against this, and re-attempted only when it changes — see
/// `publish.attempt` for why a timed retry would be the wrong shape.
type Alloc = ((u32, u32), usize);

pub fn run(signal: Arc<Signal>, registry: Registry, tx: mpsc::Sender<Result<(), String>>) {
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
        fourcc: compositor_background_two_worker_format::format::select(&device.phd),
    };
    if tx.send(Ok(())).is_err() {
        return;
    }
    let (mut passes, mut panes) = (Passes::new(), HashMap::<u64, Pane>::new());
    // Per-pane record of the allocation that failed, so an identical request is
    // never re-issued. Without it a standing refusal — out of GPU memory, a size
    // the driver will not take — is retried on every pass of a loop whose idle
    // nap is a millisecond: a thousand failed allocations and log lines a second.
    let mut failed = HashMap::<u64, Attempt<Alloc>>::new();
    // When each absent pane was last seen live, for the backstop sweep below.
    let mut absent = HashMap::<u64, Instant>::new();
    let start = Instant::now();
    // Our cursor into the kernel's output-removal mailbox.
    let mut retired_cursor = compositor_kernel_graphic_bridge_publish_retire::retire::retired_epoch();
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
        let seen: HashSet<u64> = live.iter().map(|(k, _)| *k).collect();
        // A pane collapsed away by a viewport change, retired now rather than on
        // the backstop.
        //
        // ONLY ASK AN OUTPUT ABOUT ITS OWN PANES. An output absent from `live`
        // has told us nothing — it simply is not drawing this pass, which happens
        // constantly: the picker owns the frame while the world behind it does
        // not draw, a monitor's pass is skipped, an overlay takes over. Treating
        // absence as "collapsed" freed a whole ring every time the active pass
        // changed, and the compositor then had no published frame to sample, so
        // the background VANISHED for the frames until the worker refilled it —
        // which is a hole where a backdrop should be, not a slow backdrop.
        //
        // Absence is what the removal mailbox and the timeout backstop are for.
        let collapsed = |k: u64| {
            if seen.contains(&k) {
                return false;
            }
            let mut this_output = live
                .iter()
                .filter(|(o, _)| key::key_output(*o) == key::key_output(k))
                .peekable();
            // Output not drawing at all: not our call to make.
            if this_output.peek().is_none() {
                return false;
            }
            this_output.all(|(_, r)| key::key_region(k) >= r.regions)
        };
        for (pane, req) in live.iter() {
            let (pane, req) = (*pane, req.clone());
            let request: Alloc = (req.size, slots);
            let gate = failed.entry(pane).or_default();
            if !gate.worth_trying(&request) {
                blocked += 1;
                continue;
            }
            match serve_pane(&device, &mut passes, &mut panes, &registry, &cfg, &tb, pane, req, start) {
                Ok(did) => {
                    drew |= did;
                    failed.entry(pane).or_default().succeeded();
                }
                Err(e) => {
                    if failed.entry(pane).or_default().failed(request) {
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
                let prefix = key::output_prefix(&output);
                let gone: Vec<u64> =
                    panes.keys().copied().filter(|k| key::key_output(*k) == prefix).collect();
                free(&device, &mut panes, &registry, &mut absent, &mut failed, &gone, "output removed");
            }
        }
        let gone: Vec<u64> = panes.keys().copied().filter(|k| collapsed(*k)).collect();
        free(&device, &mut panes, &registry, &mut absent, &mut failed, &gone, "region collapsed");
        retire(&device, &mut panes, &registry, &mut absent, &mut failed, &seen);
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
            // `for_ack` is false, and that is the whole point. An ack fires once
            // per pane per composite — on a host running thousands of composites
            // a second that woke this thread thousands of times a second, and
            // every wake is a full pass: lock, clone the live map, read the
            // settings, build the seen set, sweep, lock again. It also contends
            // the signal mutex with the compositor twice per composite. Nothing
            // is gained: the pane is not due yet, so the pass can only decide to
            // park again. An earlier comment here claimed an ack should "cut it
            // short", which had it backwards — cutting a render deadline short
            // is exactly what the deadline exists to prevent.
            Some(n) => signal.park(seq, n, false),
            None if drew => {}
            None if stalled => signal.park(seq, STALLED_BACKSTOP, false),
            // Due, but held by the ack handshake. This is THE case that used to
            // poll; it now sleeps until the compositor takes the frame.
            None => signal.park(seq, ACK_BACKSTOP, true),
        }
    }
    passes.destroy(&device.dev);
    for (_, p) in panes {
        device.free_ring(&p.cmds, &p.fences);
        for t in p.targets { t.destroy(&device.dev); }
    }
}

/// The BACKSTOP sweep: panes absent from the live set for longer than
/// [`RETIRE`]. Explicit removal and region collapse have already run by here, so
/// anything this catches is a caller that stopped drawing without saying so.
fn retire(
    device: &Device,
    panes: &mut HashMap<u64, Pane>,
    registry: &Registry,
    absent: &mut HashMap<u64, Instant>,
    failed: &mut HashMap<u64, Attempt<Alloc>>,
    seen: &HashSet<u64>,
) {
    let now = Instant::now();
    absent.retain(|k, _| !seen.contains(k));
    let stale: Vec<u64> = panes
        .keys()
        .copied()
        .filter(|k| !seen.contains(k))
        .filter(|k| now.duration_since(*absent.entry(*k).or_insert(now)) >= RETIRE)
        .collect();
    free(device, panes, registry, absent, failed, &stale, "not drawn");
}

/// Release `keys`. The registry entry goes FIRST, so the compositor cannot pick
/// up a handle to memory about to be freed; it then draws no background for that
/// key, which is already the pre-first-frame behaviour.
fn free(
    device: &Device,
    panes: &mut HashMap<u64, Pane>,
    registry: &Registry,
    absent: &mut HashMap<u64, Instant>,
    failed: &mut HashMap<u64, Attempt<Alloc>>,
    keys: &[u64],
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
    for k in keys {
        absent.remove(k);
        failed.remove(k);
        if let Some(p) = panes.remove(k) {
            info!("background worker: pane {k} freed ({}x{}, {why})", p.size.0, p.size.1);
            device.free_ring(&p.cmds, &p.fences);
            for t in p.targets {
                t.destroy(&device.dev);
            }
        }
    }
}

/// `Ok(true)` when a frame was rendered and published.
#[allow(clippy::too_many_arguments)]
fn serve_pane(
    device: &Device, passes: &mut Passes, panes: &mut HashMap<u64, Pane>, registry: &Registry,
    cfg: &Config, tb: &TripleBufferBackground, key: u64, req: DrawRequest, start: Instant,
) -> Result<bool, String> {
    compositor_background_two_worker_ensure::ensure::ensure(
        panes, registry, device, cfg, key, req.size,
        compositor_background_two_worker_buffers::buffers::slots(tb.slots),
    )?;
    let pane = panes.get_mut(&key).ok_or("pane vanished")?;
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
    u.time = start.elapsed().as_secs_f32();
    let target = &pane.targets[pane.cursor];
    let holder = req
        .module
        .is_none()
        .then(|| ParallaxPass::new(&u, &req.params, req.optimized));
    let pass = match (&req.module, &holder) {
        (Some(m), _) => compositor_background_two_draw_select::loaded_pass(m, &u, &req.params),
        (None, Some(h)) => h.pass(),
        (None, None) => return Err("parallax pass missing".into()),
    };
    // The pane's OWN command slot, which is its buffer slot. No global ring, so
    // this never waits on another monitor's frame.
    let slot = pane.cursor;
    let (cmd, fence) = (pane.cmds[slot], pane.fences[slot]);
    compositor_background_two_worker_render::render::render(
        device, passes, target, &pass.sdr, pane.size, cmd, fence,
    )?;
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
