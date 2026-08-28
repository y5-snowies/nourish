//! The two policy sections and the live global.
//!
//! This crate, `tearing.select` and `tearing.rate` are the persisted SHAPE of the
//! setting — what `preferences.json` holds — hence their home in `compositor.model`
//! with the rest of what the installer and developer tool also read. Resolution
//! lives in `y5.graphic/graphic.tearing/tearing.resolve`, UI strings in the
//! settings surface, runtime state in `support.smithay/smithay.state/state.tearing`.
//!
//! Tearing outranks pacing, and they are mutually exclusive in TIME rather than
//! per-window: at any instant exactly one section governs the output, or neither
//! and the compositor's default applies. That ordering is why there is only ever
//! one exclusivity gate in force — one redraw loop, one answer to "who drives".

use compositor_model_environment_tearing_rate::rate::{PaceMode, Rate, TearMode};
use compositor_model_environment_tearing_select::select::{Exclusivity, Selector};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Tearing {
    pub selector: Selector,
    pub mode: TearMode,
    pub rate: Rate,
    pub exclusivity: Exclusivity,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Pacing {
    pub selector: Selector,
    pub mode: PaceMode,
    pub rate: Rate,
    pub exclusivity: Exclusivity,
}

/// How a client becomes a "target" without speaking `wp_tearing_control_v1`.
/// Resolved once per window, at map time — the heuristics behind it read `/proc`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Tagging {
    /// Tag Steam-launched applications. Steam's own UI is excluded.
    pub steam: bool,
}

/// `Y5_TEARING=1` is deliberately NOT a toggle: it is the user saying so on a
/// specific process, and there is nothing to second-guess.
impl Default for Tagging {
    fn default() -> Self { Self { steam: true } }
}

/// When the render loop may skip a frame nothing changed in. See
/// [`Config::preemptive`] for what each value costs.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Preemptive {
    /// Skip whenever damage is empty. The loop parks on an idle desktop.
    Off,
    /// Never skip while a section is in force. The shipped value.
    #[default]
    Engaged,
    /// Never skip, in force or not.
    Always,
}

impl Preemptive {
    pub const ALL: [Preemptive; 3] = [Self::Off, Self::Engaged, Self::Always];
    /// `governed` is whether a tearing or pacing section resolved this frame.
    pub fn forces(self, governed: bool) -> bool {
        match self {
            Self::Off => false,
            Self::Engaged => governed,
            Self::Always => true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Config {
    pub tearing: Tearing,
    pub pacing: Pacing,
    /// `serde(default)` because `flip` is already written to preferences.json
    /// without this key. A missing FIELD inside a present object is an error, not
    /// a default — and that error fails the whole document, resetting every
    /// unrelated preference.
    #[serde(default)]
    pub tag: Tagging,
    /// Floor for the exclusivity watchdog: the SLOWEST the compositor may run
    /// while a gate is engaged. Not a cap — a rescue rate, and not optional:
    /// see [`FLOOR_MIN_FPS`]. Anything slower normalizes up to it.
    #[serde(default = "floor_default")]
    pub floor: Rate,
    /// Pre-emptive rendering: present every frame the loop reaches, instead of
    /// only when the damage tracker found something changed. Shared by both
    /// sections, like [`floor`] — there is one redraw loop, and this decides
    /// whether it may ever skip.
    ///
    /// Normally a frame in which nothing moved is reported empty, `queue_frame`
    /// refuses it outright (`FrameError::EmptyFrame`), and — because the render
    /// loop re-arms its redraw latch only after a NON-EMPTY result — the loop
    /// parks until something wakes it. That is the saving.
    ///
    /// What this does NOT do is fake damage to defeat that. Forcing full-output
    /// damage (`reset_buffer_ages`) or drawing undamaged elements
    /// (`DRAW_ALL_ELEMENTS`) would make every frame a full-screen recomposite, so
    /// the numbers would describe a compositor nobody runs. Instead it sets
    /// `FrameFlags::FORCE_PRESENT`, which only changes whether the prepared frame may be
    /// reported EMPTY: buffer ages, damage history, the render work and the shape of
    /// the atomic commit are all untouched — so an async (tearing) flip stays the
    /// single-plane FB swap the kernel requires. The flip always has something to
    /// queue, the latch always re-arms, and the loop free-runs at its real cost.
    ///
    /// [`Preemptive::Engaged`] by default: while a section IS in force the loop
    /// never parks, and on an ordinary desktop — where pacing never engages and
    /// tearing waits for a tagged, focused target — it behaves exactly as it did
    /// before this setting existed. That is what makes the default free: the
    /// frames it adds are only ever added where a cadence policy is already
    /// driving, and there a frame ready ahead of the flip is the whole point.
    ///
    /// [`Preemptive::Always`] extends it to the ungoverned case as well, so the
    /// loop never parks at all. Flips are vblank-locked with no section in force,
    /// so this is bounded by the panel rather than free-running — one composite
    /// per refresh on a screen doing nothing. It also restores what the
    /// compositor did before element identities were stabilised, when a
    /// per-frame-rebuilt element re-damaged itself and kept the loop alive by
    /// accident: useful when something updates without reporting damage, and the
    /// setting to reach for when a surface looks stale rather than slow.
    ///
    /// `serde(default = ...)` rather than plain `serde(default)`: preferences.json
    /// predates the key, and a bare default would give every existing install
    /// `Off` — the opposite of the shipped value.
    /// NOT SETTABLE. `serde(skip)`, so it is neither read from nor written to
    /// preferences.json, and the Settings panel no longer offers it.
    ///
    /// It is pinned to [`PREEMPTIVE_DEFAULT`] — `Always` — because the loop
    /// parking on an empty frame turned out to be a HANG, not an optimisation: a
    /// bundle's band is damage the engine cannot see, and once the band element is
    /// occluded there is nothing left to re-arm the redraw latch. That is not a
    /// trade-off to expose until the occlusion side is fixed, because one of the
    /// three values freezes the session.
    ///
    /// The enum, [`Preemptive::forces`] and [`Preemptive::ALL`] are all kept: this
    /// is a knob withdrawn from users, not deleted from the code, and flipping
    /// `PREEMPTIVE_DEFAULT` is how you test the parking behaviour again.
    #[serde(skip, default = "preemptive_default")]
    pub preemptive: Preemptive,
}

/// One times refresh: a stalled target drops the desktop back to exactly the rate
/// it would run at with no policy engaged at all, on whatever panel it is on —
/// rather than to a fixed number that is generous on 60Hz and punitive on 240.
///
/// It does not compete with a healthy target either: the grace threshold is
/// `2 x measured cadence` clamped up to this, so anything drawing at or above
/// refresh never reaches it.
pub const FLOOR_DEFAULT: Rate = Rate::Multiplier(1.0);
fn floor_default() -> Rate { FLOOR_DEFAULT }

/// ALWAYS, pinned. An active bundle's band is a damage source smithay cannot
/// always see (an opaque window covering the output occludes it), and a frame
/// wrongly reported empty parks the render loop for good — so the loop never
/// parks, by decree. Flip here to test the parking behaviour; nothing else
/// selects it.
pub const PREEMPTIVE_DEFAULT: Preemptive = Preemptive::Always;
fn preemptive_default() -> Preemptive { PREEMPTIVE_DEFAULT }

/// The floor has no "off". While a gate is engaged the rescue frames are the only
/// thing still driving the loop: the cursor, the compositor's own UI and — the
/// part that actually deadlocks — the frame callbacks the admitted client needs
/// before it may commit again. `Uncapped` there is not a rate, it is a hang, so
/// it normalizes up to [`FLOOR_DEFAULT`] and this is the slowest selectable rate.
pub const FLOOR_MIN_FPS: f32 = 15.0;
/// `1 / FLOOR_MIN_FPS`. Written out: const float division is not available here.
pub const FLOOR_MAX_INTERVAL: Duration = Duration::from_nanos(66_666_667);

/// Normalize a FLOOR rate. Unlike a cap it cannot be uncapped, and `Multiplier`
/// is left alone — it resolves against a mode this layer cannot see, so its clamp
/// belongs to [`Config::floor_interval`].
pub fn normalized_floor(r: Rate) -> Rate {
    match r.normalized() {
        Rate::Fps(f) => Rate::Fps(f.max(FLOOR_MIN_FPS)),
        Rate::Multiplier(m) => Rate::Multiplier(m),
        Rate::Uncapped => FLOOR_DEFAULT,
    }
}

/// Tearing is armed for a tagged client that is BOTH visible and focused, and
/// takes exclusive control of the cadence while it is. Scoping to focus is what
/// makes it safe as a default: two tagged clients cannot fight over the cadence,
/// and losing focus disengages exclusivity outright rather than leaving the
/// desktop paced by a window you are no longer using.
pub const TEARING_DEFAULT: Tearing = Tearing {
    selector: Selector::TargetFocused,
    mode: TearMode::Always,
    rate: Rate::Uncapped,
    exclusivity: Exclusivity::ExclusiveFocused,
};
pub const PACING_DEFAULT: Pacing = Pacing {
    selector: Selector::Never,
    mode: PaceMode::Fixed,
    rate: Rate::Uncapped,
    exclusivity: Exclusivity::None,
};

impl Default for Tearing {
    fn default() -> Self { TEARING_DEFAULT }
}
impl Default for Pacing {
    fn default() -> Self { PACING_DEFAULT }
}
impl Default for Config {
    fn default() -> Self {
        Self {
            tearing: TEARING_DEFAULT,
            pacing: PACING_DEFAULT,
            tag: Tagging { steam: true },
            floor: FLOOR_DEFAULT,
            preemptive: PREEMPTIVE_DEFAULT,
        }
    }
}

impl Config {
    pub fn normalized(mut self) -> Self {
        self.tearing.rate = self.tearing.rate.normalized();
        self.pacing.rate = self.pacing.rate.normalized();
        self.floor = normalized_floor(self.floor);
        self
    }

    /// The floor as a concrete interval for `refresh` — never absent, and never
    /// slower than [`FLOOR_MAX_INTERVAL`]. `Multiplier` is only bounded here,
    /// where the mode is known: `0.1x` on a 60Hz panel would otherwise resolve to
    /// a 166ms rescue, well past the point the desktop stops being usable.
    pub fn floor_interval(&self, refresh: Duration) -> Duration {
        self.floor
            .min_interval(refresh)
            .unwrap_or(FLOOR_MAX_INTERVAL)
            .min(FLOOR_MAX_INTERVAL)
    }
}

static CONFIG: std::sync::RwLock<Config> =
    std::sync::RwLock::new(Config {
        tearing: TEARING_DEFAULT, pacing: PACING_DEFAULT,
        tag: Tagging { steam: true }, floor: FLOOR_DEFAULT,
        preemptive: PREEMPTIVE_DEFAULT,
    });

pub fn get() -> Config { CONFIG.read().map(|c| *c).unwrap_or_default() }
pub fn set(c: Config) { if let Ok(mut w) = CONFIG.write() { *w = c.normalized(); } }
