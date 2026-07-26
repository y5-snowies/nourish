//! Per-frame resolution: which section governs, and what it decides.

use compositor_model_environment_tearing_config::config::{Config, Pacing, Tearing};
use compositor_model_environment_tearing_rate::rate::{PaceMode, TearMode};
use compositor_model_environment_tearing_select::select::{Exclusivity, Scene};
use std::time::Duration;

/// Fraction of a refresh interval counting as "retrace imminent"; inside it a
/// sync flip is near-free. Higher = fewer tears, lower throughput.
pub const SYNC_WINDOW: f32 = 0.25;

/// The section governing this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Active {
    Tear(Tearing),
    Pace(Pacing),
    /// Neither selector matched: vsync'd flips, uncapped, no exclusivity.
    Default,
}

/// Tearing first, then pacing, then the compositor's default.
pub fn active(cfg: &Config, scene: Scene) -> Active {
    if cfg.tearing.selector.matches(scene) {
        Active::Tear(cfg.tearing)
    } else if cfg.pacing.selector.matches(scene) {
        Active::Pace(cfg.pacing)
    } else {
        Active::Default
    }
}

impl Active {
    pub fn min_interval(&self, refresh: Duration) -> Option<Duration> {
        match self {
            Self::Tear(t) => t.rate.min_interval(refresh),
            Self::Pace(p) => p.rate.min_interval(refresh),
            Self::Default => None,
        }
    }

    /// Could the policy IN FORCE issue an async flip? Drives plane assignment.
    ///
    /// Deliberately a property of the resolved section, not of the whole config:
    /// judging it from the config would disable hardware planes (and the hardware
    /// cursor with them) permanently the moment any selector is armed, including
    /// on an ordinary desktop that is waiting for a target and never tears.
    pub fn may_tear(&self, refresh: Duration) -> bool {
        match self {
            Self::Default => false,
            Self::Tear(_) => true,
            Self::Pace(p) => match p.mode {
                PaceMode::Adaptive => true,
                PaceMode::Fixed => p.rate.exceeds_refresh(refresh),
            },
        }
    }

    pub fn exclusivity(&self) -> Exclusivity {
        match self {
            Self::Tear(t) => t.exclusivity,
            Self::Pace(p) => p.exclusivity,
            Self::Default => Exclusivity::None,
        }
    }

    /// Should THIS flip be async? `until_vblank` is the time left before the next
    /// retrace (`None` = none observed yet, which prefers the clean frame).
    ///
    /// `PaceMode::Fixed` tears only where tearing stops being a choice. Adaptive
    /// thresholds on a fraction of refresh, NOT on measured composite duration —
    /// that is bistable, since tearing raises the frame rate, which shrinks
    /// per-frame damage, which cheapens composites and eases the test, and the
    /// mode then latches into whichever state a disturbance pushed it toward.
    pub fn tear_now(&self, until_vblank: Option<Duration>, refresh: Duration) -> bool {
        let adaptive = until_vblank.is_some_and(|l| l > refresh.mul_f32(SYNC_WINDOW));
        match self {
            Self::Default => false,
            Self::Tear(t) => matches!(t.mode, TearMode::Always) || adaptive,
            Self::Pace(p) => match p.mode {
                PaceMode::Fixed => p.rate.exceeds_refresh(refresh),
                PaceMode::Adaptive => adaptive,
            },
        }
    }
}
