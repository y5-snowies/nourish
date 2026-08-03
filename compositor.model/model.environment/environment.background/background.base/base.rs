//! Background triple buffering: the off-thread background worker's settings.
//!
//! The shader renders on its own thread and `VkDevice` into three rotating
//! dmabufs; the compositor samples the newest finished one. That keeps a heavy
//! background out of the compositor's single command buffer, which it blocks on
//! every frame, inline, on the thread that also dispatches input.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use std::time::Duration;
pub use compositor_model_environment_tearing_rate::rate::Rate;
/// What paces [`TripleBufferBackground::rate`]. Shared with the UI triple buffering, so
/// the two settings cannot drift on what "one per frame" means.
pub use compositor_model_environment_tearing_rate::rate::Cadence;

/// `#[serde(default)]` at the STRUCT level: a `preferences.json` written before a
/// field existed must still parse, or the whole document fails and every
/// unrelated setting in it is replaced by defaults on the next save.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(default)]
pub struct TripleBufferBackground {
    pub enabled: bool,
    /// Renders per second, relative to the monitor's refresh or absolute.
    pub rate: Rate,
    /// What [`Self::rate`] is counted against.
    pub cadence: Cadence,
    /// What [`Self::ceiling`] is counted against. SEPARATE from `cadence` on
    /// purpose: the natural setting is a rate counted in vblanks (phase-locked to
    /// the compositor, no timer to drift) under a cap expressed against the panel
    /// (a bound that means the same thing on any host). One field could not say
    /// that — it forced the cap into whichever space the rate used.
    pub ceiling_cadence: Cadence,
    /// Hard ceiling, same form as `rate`, always a WALL-CLOCK minimum interval.
    /// THE LAST WORD, applied whatever `rate` and `cadence` resolve to.
    ///
    /// Load-bearing, not a safety valve: under [`Cadence::Vblank`] the counter is
    /// scene builds, not retraces, so on a host that tears even `Multiplier(1.0)`
    /// is bounded by the composite rate rather than the panel — which is to say
    /// not bounded at all. This is what bounds it.
    pub ceiling: Rate,
    /// Overlap recording the next frame with the GPU running the current one.
    pub pipeline: bool,
    /// Buffers in the ring, PER PANE. Two saves a fullscreen dmabuf per pane and
    /// cannot pipeline; three is what pipelining needs. Clamped by the worker.
    pub slots: u8,
    /// Editor mode: `true` picks from the preset ladder, `false` exposes the
    /// knobs above. Presentational, but it belongs to this setting, not beside it.
    pub preset: bool,
}
/// Clamp one rate. NOT [`Rate::normalized`], whose 10fps floor suits compositing
/// — a background at 1-5fps is legitimate and must not be raised.
fn clamp(r: Rate) -> Rate {
    match r {
        Rate::Multiplier(m) if m.is_finite() && m > 0.0 => Rate::Multiplier(m.clamp(0.005, 4.0)),
        Rate::Fps(f) if f.is_finite() && f > 0.0 => Rate::Fps(f.clamp(0.2, 480.0)),
        _ => Rate::Uncapped,
    }
}

/// The recommended knob sets; `background.preset` names them. Here so `Default`
/// can reach them without depending back on that crate.
///
/// A ladder, not alternatives. Every rung pairs a rate counted PER COMPOSITE with
/// a cap counted PER REFRESH: the rate then rides the compositor's own cadence,
/// phase-locked and with no wall-clock timer to drift against the retrace, while
/// the cap stays tied to the display so a tearing or nested host cannot overshoot
/// what the panel can show.
/// Optimized's rate, with the headroom to actually sustain it.
///
/// Identical to [`OPTIMIZED`] except for the two knobs that cost resources rather
/// than change intent: three buffers and pipelining. That pairing is the point —
/// unpipelined, the worker serializes record -> submit -> wait -> publish and
/// keeps up only while CPU plus GPU fit in one interval; pipelined, the two
/// overlap and only the LONGER has to fit. Same asked-for rate, more headroom to
/// deliver it, at one more fullscreen dmabuf per pane and a frame of latency.
///
/// Reach for it when a heavy shader on a fast panel starts missing flips.
pub const MAXIMUM: TripleBufferBackground = TripleBufferBackground {
    enabled: true, preset: true, rate: Rate::Multiplier(1.0), ceiling: Rate::Multiplier(1.0),
    cadence: Cadence::Vblank, ceiling_cadence: Cadence::Timer, pipeline: true, slots: 3,
};
/// The DEFAULT. A fresh frame for every flip, and nothing spent beyond it.
///
/// Counted PER REFRESH, so `rate` 1.0x means the panel's rate whatever the
/// compositor is doing — the flip is what we are trying to hit, and on a tearing
/// or nested host composites run far above it. `ceiling` 1.0x then stops the
/// producer ever rendering faster than the panel can show.
///
/// Two buffers and NO pipelining. That is a deliberate trade against the third
/// slot: unpipelined, the worker serializes record -> submit -> wait -> publish,
/// so it keeps up only while CPU plus GPU fit inside one interval — but it costs
/// no extra frame of latency and one less fullscreen dmabuf PER PANE, which on
/// multi-monitor is the largest memory item here. Pipelining with three is the
/// setting to reach for if a heavy shader starts missing the cadence.
pub const OPTIMIZED: TripleBufferBackground = TripleBufferBackground {
    enabled: true, preset: true, rate: Rate::Multiplier(1.0), ceiling: Rate::Multiplier(1.0),
    cadence: Cadence::Vblank, ceiling_cadence: Cadence::Timer, pipeline: false, slots: 2,
};
/// Half the panel — 30fps on 60Hz. A background is slow, low-contrast ambience,
/// so half rate is rarely visible on one while costing half the GPU.
///
/// Two buffers, like every rung: none of these pipeline, so a third slot would
/// hold a fullscreen dmabuf per pane and buy nothing.
pub const EFFICIENT: TripleBufferBackground = TripleBufferBackground {
    enabled: true, preset: true, rate: Rate::Multiplier(0.5), ceiling: Rate::Multiplier(0.5),
    cadence: Cadence::Vblank, ceiling_cadence: Cadence::Timer, pipeline: false, slots: 2,
};
/// A quarter of the panel — 15fps on 60Hz. For a Pi, or anything on battery,
/// where the shader is a real share of the budget. A fixed low FPS is still
/// reachable by hand in Manual; as a rung it made the ladder jump too far.
pub const POWER_SAVER: TripleBufferBackground = TripleBufferBackground {
    enabled: true, preset: true, rate: Rate::Multiplier(0.25), ceiling: Rate::Multiplier(0.25),
    cadence: Cadence::Vblank, ceiling_cadence: Cadence::Timer, pipeline: false, slots: 2,
};

impl Default for TripleBufferBackground {
    fn default() -> Self {
        OPTIMIZED
    }
}

impl TripleBufferBackground {
    /// Minimum spacing between renders on a `refresh` panel, from `ceiling`.
    pub fn cap(&self, refresh: Duration) -> Option<Duration> {
        self.ceiling.min_interval(refresh)
    }

    /// Clamp both rates to sane bounds.
    pub fn normalized(mut self) -> Self {
        self.rate = clamp(self.rate);
        self.ceiling = clamp(self.ceiling);
        self
    }

    /// Whether the off-thread worker should actually run.
    ///
    /// `enabled` AND Vulkan. The worker renders into dmabufs the compositor
    /// samples natively; on GLES nothing reads them, so honouring `enabled`
    /// alone would spawn a thread, a second `VkInstance`/`VkDevice` and a
    /// fullscreen buffer ring per pane whose output no code path can ever
    /// consume. GLES stays on the inline shader, which is what it has always
    /// done.
    pub fn engaged(&self) -> bool {
        self.enabled && compositor_model_stats_registry_counter::compositor_prefers_dmabuf()
    }
}

/// The process-global live copy, re-read by the worker on every pass — which is
/// what makes rate, cadence, ceiling and pipelining apply without a restart.
/// Only `enabled` needs one, since it decides whether the worker exists.
///
/// Here rather than in `model.stats`: this is a setting, and the UI triple
/// buffering already kept its live copy next to its own type.
fn live() -> &'static RwLock<TripleBufferBackground> {
    static SLOT: RwLock<TripleBufferBackground> = RwLock::new(OPTIMIZED);
    &SLOT
}

/// Woken whenever the setting changes — see [`set`].
type Waker = Arc<dyn Fn() + Send + Sync>;

fn waker() -> &'static RwLock<Option<Waker>> {
    static SLOT: std::sync::OnceLock<RwLock<Option<Waker>>> = std::sync::OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

/// Worker: register a wake for settings changes.
///
/// Without it the worker learns about a change only when it next happens to run:
/// it is parked on a render deadline or a backstop, and neither a ping nor an ack
/// fires for a settings save. `slots` is part of the key the worker keys its
/// buffers AND its failed-allocation gate on, so a depth change is a
/// REALLOCATION — and waiting out a timer for one is exactly the coupling the
/// event-driven park was built to remove.
pub fn set_change_waker(f: Waker) {
    *waker().write().unwrap_or_else(|e| e.into_inner()) = Some(f);
}

/// Publish the setting. Called once on load and again on every settings save.
/// `background.two`'s `update()` has no preference access, so this global is the
/// seam it reads through.
///
/// Wakes the worker, and only when something actually moved: a save republishes
/// unconditionally, and the settings window saves on every keystroke in a text
/// field.
pub fn set(value: TripleBufferBackground) {
    let changed = {
        let mut slot = live().write().unwrap_or_else(|e| e.into_inner());
        let changed = *slot != value;
        *slot = value;
        changed
    };
    if !changed {
        return;
    }
    let w = waker().read().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(f) = w {
        f();
    }
}

pub fn get() -> TripleBufferBackground {
    *live().read().unwrap_or_else(|e| e.into_inner())
}
