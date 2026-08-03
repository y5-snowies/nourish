//! The compositor → worker draw signal.
//!
//! A PING IS NOT A RENDER COMMAND: it publishes a pane's latest state and marks
//! it live. The worker paces itself off `Config::rate`; a ping only kick-starts
//! it when it had parked. If one ping meant one render the worker's rate would BE
//! the compositor's frame rate — thousands of shader passes a second through one
//! hardware queue. Idle falls out for free: a pane goes quiet [`LIVE`] after its
//! last ping, and the worker then parks in [`Signal::live`].

use compositor_background_two_shader_spirv::VulkanModule;
use compositor_orchestration_draw_dispatch_frame::ParallaxUniforms;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// How long a pane stays live after its last ping — only has to clear one frame
/// at a low refresh; when drawing stops, the pane lapses and the worker parks.
pub const LIVE: Duration = Duration::from_millis(250);

/// How often a worker that still owns buffers comes round with nothing live, so
/// its retirement sweep can run. One wakeup a second, and only while it is
/// actually holding memory — an idle worker with nothing allocated parks for
/// good.
pub const SWEEP: Duration = Duration::from_secs(1);

/// Everything the worker needs to render one pane's background frame.
///
/// `uniforms.time` is IGNORED — the worker stamps its own, since its rate does
/// not track the ping rate and a caller-supplied time would drift with load.
#[derive(Clone)]
pub struct DrawRequest {
    pub uniforms: ParallaxUniforms,
    pub params: [f32; 16],
    /// Runtime-loaded shader, or `None` for the built-in. Cheap per request: the
    /// module's SPIR-V is behind an `Arc`.
    pub module: Option<Arc<VulkanModule>>,
    /// The world's "Optimized" flag — selects a SPIR-V variant, not a push value,
    /// so it rides here rather than inside `uniforms`.
    pub optimized: bool,
    /// This pane's physical pixel size. Panes differ in size and — across
    /// monitors — in camera, so each owns buffers at its own resolution.
    pub size: (u32, u32),
    /// Refresh of the monitor this pane is on — `Rate::Multiplier` is relative
    /// to it, and it differs per monitor.
    pub refresh: Duration,
    /// That output's monotonic scene-build counter — a retrace count on the
    /// native path. Carried, not derived by counting pings: a pane lowered during
    /// two outputs' passes would be pinged twice per retrace and halve its divisor.
    pub serial: u64,
    /// How many viewport regions this output currently has.
    ///
    /// The retirement signal for a viewport collapse: every pane of this output
    /// whose region index is `>= regions` no longer exists, and the worker frees
    /// it on the spot rather than waiting for a liveness timeout to notice.
    pub regions: usize,
}

#[derive(Default)]
struct State {
    panes: HashMap<u64, (DrawRequest, Instant)>,
    /// Bumped by anything that could change what the worker would decide, so a
    /// wake that lands between "finished a pass" and "went to sleep" is not
    /// lost. The worker snapshots it BEFORE its pass and hands it back to
    /// [`Signal::park`], which returns at once if it has moved.
    progress: u64,
    /// Whether the worker is parked specifically waiting for the compositor to
    /// take its last published frame. Only then does an ack wake it — otherwise
    /// every pane's `latest()` would wake the worker once per compositor frame
    /// to tell it something it is not waiting for.
    awaiting_ack: bool,
    stop: bool,
}

/// Many producers in principle, one consumer (the worker thread).
#[derive(Default)]
pub struct Signal {
    state: Mutex<State>,
    wake: Condvar,
}

impl Signal {
    pub fn new() -> Self {
        Self::default()
    }

    /// Compositor thread: publish `pane`'s state and mark it live. Not a render
    /// request — it only wakes the worker if it had parked. Never blocks.
    ///
    /// Waking is DELIBERATELY selective. A ping arrives per pane per compositor
    /// frame; waking on every one would put the worker back on the ping rate,
    /// which is the thing this type exists to decouple. It wakes only when
    /// something the worker's decision depends on actually moved:
    ///
    /// * the map was empty — it is parked and has to come back at all;
    /// * the pane is new — there is a buffer set to allocate;
    /// * the pane RESIZED — the one input that can reopen an allocation the
    ///   driver refused, and so the one that ends a stall.
    ///
    /// A ping that only carries a new camera changes nothing about *when* to
    /// render, so it does not wake anyone; the worker will read it on its own
    /// schedule.
    pub fn ping(&self, pane: u64, req: DrawRequest) {
        if let Ok(mut s) = self.state.lock() {
            let was_idle = s.panes.is_empty();
            let resized = s.panes.get(&pane).is_none_or(|(p, _)| p.size != req.size);
            s.panes.insert(pane, (req, Instant::now()));
            if was_idle || resized {
                s.progress = s.progress.wrapping_add(1);
                self.wake.notify_one();
            }
        }
    }

    /// Compositor thread: the last published frame has been taken.
    ///
    /// THE event the worker's `drained()` gate is waiting on. Without it the
    /// worker had to poll for the acknowledgement on a millisecond nap, once per
    /// background frame — a thousand wakeups a second to notice something that
    /// happens on a schedule the compositor already knows. Costs nothing when
    /// nobody is waiting: `awaiting_ack` is false and this is a lock and a
    /// compare.
    pub fn acknowledged(&self) {
        if let Ok(mut s) = self.state.lock() {
            if s.awaiting_ack {
                s.progress = s.progress.wrapping_add(1);
                self.wake.notify_one();
            }
        }
    }

    /// Anything that changes what the worker would decide but is neither a ping
    /// nor an ack — a settings save, which can change the ring depth and so is a
    /// reallocation. Same lost-wakeup guarantee as the rest: the counter moves,
    /// so a park that has not started yet returns immediately.
    pub fn poke(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.progress = s.progress.wrapping_add(1);
            self.wake.notify_one();
        }
    }

    /// Worker thread: the current progress counter, snapshotted BEFORE a pass so
    /// [`park`](Self::park) cannot sleep through a wake that landed during it.
    pub fn progress(&self) -> u64 {
        self.state.lock().map(|s| s.progress).unwrap_or(0)
    }

    /// Worker thread: sleep until something changes, or `backstop` elapses.
    ///
    /// `seq` is the counter from before the pass. If it has already moved this
    /// returns immediately — the lost-wakeup guard, and the reason the counter
    /// exists rather than a bare condvar.
    ///
    /// `for_ack` says the worker is blocked on the compositor taking its last
    /// frame, which is what lets [`acknowledged`](Self::acknowledged) stay quiet
    /// the rest of the time. The backstop is a backstop: every case that gets
    /// here has a real event that should arrive first.
    pub fn park(&self, seq: u64, backstop: Duration, for_ack: bool) {
        let Ok(mut s) = self.state.lock() else { return };
        if s.stop || s.progress != seq {
            return;
        }
        s.awaiting_ack = for_ack;
        let Ok((mut guard, _)) = self.wake.wait_timeout(s, backstop) else { return };
        guard.awaiting_ack = false;
    }

    /// Worker thread: every live pane's state, parking while none are. Entries
    /// are NOT consumed — re-reading them each pass is what lets the worker run
    /// at its own rate rather than the ping rate. `None` = stop.
    /// `holding` says whether the worker still owns buffers. It decides how the
    /// park behaves, and the distinction is load-bearing: with buffers to
    /// account for, the worker must come round even with nothing to draw, or a
    /// monitor unplugged on an idle desktop leaves its ring in GPU memory until
    /// something happens to redraw. With nothing allocated there is no such work,
    /// so it parks indefinitely and an idle machine stays idle.
    pub fn live(&self, holding: bool) -> Option<Vec<(u64, DrawRequest)>> {
        let mut s = self.state.lock().ok()?;
        loop {
            if s.stop {
                return None;
            }
            s.panes.retain(|_, (_, t)| t.elapsed() < LIVE);
            if !s.panes.is_empty() {
                return Some(s.panes.iter().map(|(k, (r, _))| (*k, r.clone())).collect());
            }
            if !holding {
                s = self.wake.wait(s).ok()?;
                continue;
            }
            let (guard, timeout) = self.wake.wait_timeout(s, SWEEP).ok()?;
            s = guard;
            if timeout.timed_out() {
                return Some(Vec::new());
            }
        }
    }

    /// Ask the worker loop to exit, waking it if parked.
    pub fn stop(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.stop = true;
            self.wake.notify_all();
        }
    }
}
