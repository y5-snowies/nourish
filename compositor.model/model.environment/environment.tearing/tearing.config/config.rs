//! The two policy sections and the live global.
//!
//! This crate, `tearing.select` and `tearing.rate` are the persisted SHAPE of the
//! setting — what `preferences.json` holds — which is why they sit in
//! `compositor.developer` with the rest of the settings the installer and the
//! developer tool also read. Nothing else of the domain belongs here: resolution
//! lives in `y5.graphic/graphic.tearing/tearing.resolve` and the UI strings
//! beside it, the runtime state in `support.smithay/smithay.state/state.tearing`.
//!
//! Tearing outranks pacing, and they are mutually exclusive in TIME rather than
//! per-window: at any instant exactly one section governs the output, or neither
//! and the compositor's default applies. That ordering is why there is only ever
//! one exclusivity gate in force — one redraw loop, one answer to "who drives".

use compositor_model_environment_tearing_rate::rate::{PaceMode, Rate, TearMode};
use compositor_model_environment_tearing_select::select::{Exclusivity, Selector};
use serde::{Deserialize, Serialize};

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

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub struct Config {
    pub tearing: Tearing,
    pub pacing: Pacing,
    /// `serde(default)` because `flip` is already written to preferences.json
    /// without this key. A missing FIELD inside a present object is an error, not
    /// a default — and that error fails the whole document, resetting every
    /// unrelated preference.
    #[serde(default)]
    pub tag: Tagging,
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

impl Config {
    pub fn normalized(mut self) -> Self {
        self.tearing.rate = self.tearing.rate.normalized();
        self.pacing.rate = self.pacing.rate.normalized();
        self
    }
}

static CONFIG: std::sync::RwLock<Config> =
    std::sync::RwLock::new(Config { tearing: TEARING_DEFAULT, pacing: PACING_DEFAULT, tag: Tagging { steam: true } });

pub fn get() -> Config { CONFIG.read().map(|c| *c).unwrap_or_default() }
pub fn set(c: Config) { if let Ok(mut w) = CONFIG.write() { *w = c.normalized(); } }
