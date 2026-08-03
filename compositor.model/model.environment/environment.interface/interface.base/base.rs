//! UI triple buffering: the ring the iced and bevy surfaces publish through.
//!
//! Both subsystems already render on their own `wgpu::Device`, but into a SINGLE
//! dmabuf that the compositor samples in the same frame it was written — the
//! runtimes are ticked from scene assembly, on the thread that also dispatches
//! input. The write fence sits on that dmabuf's `dma_resv`, so the compositor's
//! composite waits on the UI's GPU work in the kernel whether or not anyone asked
//! it to. A ring with publish-on-complete breaks the coupling structurally: the
//! compositor only ever samples a buffer whose write has already finished.
//!
//! Deliberately slimmer than the background's `TripleBuffer`. That one carries a
//! rate/cadence ladder because a shader animates continuously and its cadence is
//! a real choice. iced renders only when its UI is dirty, so pacing it would be
//! inert; bevy's cadence is its own `App` schedule, not ours to set from here.

pub use compositor_model_environment_tearing_rate::rate::{Cadence, Rate};
use serde::{Deserialize, Serialize};
use std::sync::RwLock;
use std::time::Duration;

/// Ring bounds.
///
/// TWO is viable and costs one fullscreen buffer less, but it cannot pipeline:
/// `capacity` is `len - 2`, so at two the publish is always synchronous whatever
/// `pipeline` says. It also wraps straight back onto the slot the compositor's
/// previous composite may still be reading — which is a WAIT on the worker
/// (implicit sync), never corruption and never compositor latency. On a
/// single-queue GPU that wait is near-unreachable: the composite was submitted
/// before our render, so it has retired by the time we unblock. On a multi-queue
/// desktop GPU it can land, and costs the worker throughput.
///
/// THREE is what pipelining needs — one slot pinned by the reader, one by the
/// writer, one to have in flight — and widens the reuse gap so the wrap-onto-read
/// case cannot arise at all.
///
/// Nothing above three: a fourth only lets the GPU fall a further frame behind,
/// which is latency rather than throughput.
pub const MIN_SLOTS: u8 = 2;
pub const MAX_SLOTS: u8 = 3;

/// Fallback refresh until the render loop publishes the real one.
///
/// 30Hz, not the panel-typical 60: an unknown refresh is a guess, and every
/// consumer turns it into a MINIMUM INTERVAL. Guessing high shortens that
/// interval and lets a producer run at twice the rate it was told to until the
/// real value lands; guessing low only makes it briefly lazy. Err slow.
const DEFAULT_REFRESH: Duration = Duration::from_micros(33_333);

/// `#[serde(default)]` at the STRUCT level, not just per field: a
/// `preferences.json` written before a field existed must still parse, or the
/// whole document fails and every unrelated setting in it is silently replaced
/// by defaults on the next save. Adding a field here must never cost the user
/// their file.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(default)]
pub struct TripleBufferUI {
    /// Off means one buffer and today's behaviour exactly, so the path stays
    /// opt-in until it has run on real hardware.
    ///
    /// NEEDS A RESTART, unlike the knobs below: it also decides whether bevy's
    /// worker thread exists at all, and instances are bound to the backend they
    /// were created on. Toggling it live only collapses or grows the ring.
    pub enabled: bool,
    /// Buffers in the ring: one published, one being drawn, and a third to absorb
    /// a frame whose GPU work has not finished by the time the next one starts.
    pub slots: u8,
    /// Publish on a POLLED completion instead of blocking for it, so recording the
    /// next frame overlaps the GPU running the current one. Without it the publish
    /// still waits — just off the composite rather than inside it.
    pub pipeline: bool,
    /// How often bevy's scenes run. Same `Rate` the background uses, counted
    /// against [`Self::cadence`].
    ///
    /// Bevy only, and deliberately: iced rasterizes only when its UI is dirty, so
    /// pacing it would cost responsiveness and save almost nothing. Bevy has no
    /// such gate — it runs a full ECS + render-graph frame on every tick whether
    /// or not the scene changed — which makes this the one real lever on a GPU
    /// with a single hardware queue, where the compositor's own frame queues
    /// behind whatever bevy submitted.
    ///
    /// This paces the whole tick, not just the raster: bevy's render graph runs
    /// inside `App::update`, so scene animation slows in step. That is what a
    /// rate cap means here, the same as the background's.
    pub rate: Rate,
    /// Whether [`Self::rate`] counts composites or wall-clock refresh.
    pub cadence: Cadence,
    /// The same, for [`Self::ceiling`]. SEPARATE from `cadence`: the
    /// natural setting is a rate counted in vblanks under a cap expressed against
    /// the panel, and one field could not say that.
    pub ceiling_cadence: Cadence,
    /// Hard ceiling on the bevy rate, same form as [`Self::rate`], always a
    /// WALL-CLOCK minimum interval. THE LAST WORD, applied whatever the rate and
    /// cadence resolve to.
    ///
    /// Load-bearing, not a safety valve: under [`Cadence::Vblank`] the counter is
    /// composites, not retraces, so on a host that tears even `Multiplier(1.0)` is
    /// bounded by the composite rate rather than the panel — which is to say not
    /// bounded at all. This is what bounds it.
    pub ceiling: Rate,
    /// Editor mode: `true` picks from the preset ladder, `false` exposes the knobs
    /// above. Presentational, but it belongs to this setting, not beside it.
    pub preset: bool,
}

/// The recommended knob sets; `interface.preset` names them. Here so `Default`
/// can reach them without depending back on that crate.
///
/// A ladder: [`OPTIMIZED`] is a fresh frame for every flip and nothing beyond it;
/// each step below trades that away for GPU time.
///
/// Every rung pairs a rate counted PER COMPOSITE with a cap counted PER REFRESH:
/// the rate rides the compositor's own cadence with no wall-clock timer to drift
/// against it, while the cap stays tied to the display.
///
/// Two buffers throughout, and none of them pipeline. Unpipelined, the worker
/// serializes record -> submit -> wait -> publish and keeps up only while CPU
/// plus GPU fit inside one interval — but it costs no extra frame of latency on
/// a surface the user is pointing at, and one less fullscreen buffer. Three plus
/// pipelining is there in Manual for a scene that starts missing the cadence.
/// Optimized's rate, with the headroom to actually sustain it: three buffers and
/// pipelining. Same intent, more resources — one more fullscreen buffer and a
/// frame of latency — for a scene that misses flips without them.
pub const MAXIMUM: TripleBufferUI = TripleBufferUI {
    enabled: true, preset: true, slots: 3, pipeline: true,
    rate: Rate::Multiplier(1.0), ceiling: Rate::Multiplier(1.0),
    cadence: Cadence::Vblank, ceiling_cadence: Cadence::Timer,
};
pub const OPTIMIZED: TripleBufferUI = TripleBufferUI {
    enabled: true, preset: true, slots: 2, pipeline: false,
    rate: Rate::Multiplier(1.0), ceiling: Rate::Multiplier(1.0),
    cadence: Cadence::Vblank, ceiling_cadence: Cadence::Timer,
};
/// Half the panel — 30fps on 60Hz. Rarely visible on a slowly-turning 3D scene,
/// and half the GPU time.
pub const EFFICIENT: TripleBufferUI = TripleBufferUI {
    enabled: true, preset: true, slots: 2, pipeline: false,
    rate: Rate::Multiplier(0.5), ceiling: Rate::Multiplier(0.5),
    cadence: Cadence::Vblank, ceiling_cadence: Cadence::Timer,
};
pub const POWER_SAVING: TripleBufferUI = TripleBufferUI {
    enabled: true, preset: true, slots: 2, pipeline: false,
    rate: Rate::Multiplier(0.25), ceiling: Rate::Multiplier(0.25),
    cadence: Cadence::Vblank, ceiling_cadence: Cadence::Timer,
};

pub const DEFAULT: TripleBufferUI = OPTIMIZED;

impl Default for TripleBufferUI {
    fn default() -> Self {
        DEFAULT
    }
}

impl TripleBufferUI {
    /// Clamp the ring depth into range. Applied on load, so a hand-edited
    /// `preferences.json` cannot ask for a 0-slot or a 200-slot ring.
    pub fn normalized(mut self) -> Self {
        self.slots = self.slots.clamp(MIN_SLOTS, MAX_SLOTS);
        self.rate = self.rate.normalized();
        self.ceiling = self.ceiling.normalized();
        self
    }

    /// Minimum wall-clock spacing between bevy ticks, from the ceiling. The last
    /// word over whatever the cadence allowed.
    pub fn cap(&self, refresh: Duration) -> Option<Duration> {
        self.ceiling.min_interval(refresh)
    }

    /// Composites between ticks for `rate` under [`Cadence::Vblank`]; at least
    /// one. A `Multiplier` needs no refresh — composites ARE the unit it counts.
    ///
    /// Free-standing rather than a method on the rate field, because both gates
    /// use it: the rate and the cap each carry their own cadence.
    pub fn every(rate: Rate, refresh: Duration) -> u64 {
        match rate {
            Rate::Uncapped => 1,
            Rate::Multiplier(m) if m > 0.0 => (1.0 / m).round().max(1.0) as u64,
            _ => match rate.min_interval(refresh) {
                Some(i) => (i.as_secs_f64() / refresh.as_secs_f64()).round().max(1.0) as u64,
                None => 1,
            },
        }
    }

    /// Minimum wall-clock spacing between bevy ticks, from the rate.
    pub fn interval(&self, refresh: Duration) -> Option<Duration> {
        self.rate.min_interval(refresh)
    }

    /// Whether the ring path should actually run: `enabled` AND Vulkan.
    ///
    /// GLES is excluded outright. The worker half is already Vulkan-only — its
    /// slots carry no `GlesTexture`, because building one needs `&mut
    /// GlesRenderer`, the one thing that cannot leave the compositor thread —
    /// and running only the ring half there would spend a fullscreen dmabuf per
    /// surface per slot on a path whose whole point is decoupling from a
    /// submission GLES does not make. GLES keeps today's single buffer.
    pub fn engaged(&self) -> bool {
        self.enabled && compositor_model_stats_registry_counter::compositor_prefers_dmabuf()
    }

    /// Buffers to actually allocate. Not engaged collapses to one whatever
    /// `slots` holds, so toggling off — or running on GLES — restores the
    /// single-buffer path without discarding the depth the user picked.
    pub fn depth(&self) -> usize {
        match self.engaged() {
            true => self.slots.clamp(MIN_SLOTS, MAX_SLOTS) as usize,
            false => 1,
        }
    }
}

fn live() -> &'static RwLock<TripleBufferUI> {
    static SLOT: RwLock<TripleBufferUI> = RwLock::new(DEFAULT);
    &SLOT
}

/// Publish the setting. Called once on load and again on every settings save —
/// this is what makes the knobs live, since the surfaces re-read it each frame.
/// Same seam as `graphics.base`: the render path cannot reach preferences.
pub fn set(value: TripleBufferUI) {
    *live().write().unwrap_or_else(|e| e.into_inner()) = value;
}

pub fn get() -> TripleBufferUI {
    *live().read().unwrap_or_else(|e| e.into_inner())
}

fn live_refresh() -> &'static RwLock<Duration> {
    static SLOT: RwLock<Duration> = RwLock::new(DEFAULT_REFRESH);
    &SLOT
}

/// Render loop: publish the FASTEST mapped output's refresh, once per scene
/// build (`Orchestrator::fastest_refresh`).
///
/// One value rather than per-monitor, unlike the background's per-pane refresh:
/// the worker has no output of its own to ask. The value must therefore not
/// depend on WHICH output is drawing — `scene` runs once per output, so
/// publishing the drawing one made this alternate every frame on a mixed-refresh
/// desktop.
///
/// FASTEST because a bevy instance is composited on every mapped output (its
/// elements are not gated per output), so pacing it to the slowest panel would
/// stutter on the fast one. The slow panel loses nothing: the compositor already
/// composites it at its own rate, which bounds what it can show regardless.
/// Only meaningful under `Cadence::Timer` and for `Rate::Fps` — a `Multiplier`
/// under `Cadence::Vblank` counts composites and never consults this.
pub fn set_refresh(d: Duration) {
    if !d.is_zero() {
        *live_refresh().write().unwrap_or_else(|e| e.into_inner()) = d;
    }
}

pub fn refresh() -> Duration {
    *live_refresh().read().unwrap_or_else(|e| e.into_inner())
}
