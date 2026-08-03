//! Composite-rate ceiling and the per-section flip modes. UI strings live in
//! `configurator.settings/settings.surface/surface.tearlabel`.

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Rate {
    Uncapped,
    /// Multiple of the output's refresh (`2.0` on 60Hz = 120/s). Follows the panel.
    Multiplier(f32),
    /// Absolute frames per second. Per-monitor, since refresh is.
    Fps(f32),
}

impl Rate {
    /// Minimum spacing between composites; `None` = uncapped.
    pub fn min_interval(self, refresh: Duration) -> Option<Duration> {
        match self {
            Self::Multiplier(m) if m > 0.0 => Some(refresh.div_f32(m)),
            Self::Fps(f) if f > 0.0 => Some(Duration::from_secs_f32(1.0 / f)),
            _ => None,
        }
    }

    /// Does holding this rate require flipping mid-scanout? True exactly when the
    /// target exceeds what the panel can present, which is where tearing stops
    /// being a choice: you cannot show more distinct frames per second than the
    /// display has retraces without flipping between them.
    pub fn exceeds_refresh(self, refresh: Duration) -> bool {
        self.min_interval(refresh).is_none_or(|min| min < refresh)
    }

    pub fn normalized(self) -> Self {
        match self {
            Self::Multiplier(m) if m.is_finite() && m > 0.0 => Self::Multiplier(m.clamp(0.1, 16.0)),
            Self::Fps(f) if f.is_finite() && f > 0.0 => Self::Fps(f.clamp(10.0, 1000.0)),
            _ => Self::Uncapped,
        }
    }
}

/// How flips are issued while the TEARING section is in force.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TearMode {
    #[default]
    Always,
    Adaptive,
}

impl TearMode {
    pub const ALL: [TearMode; 2] = [Self::Always, Self::Adaptive];
}

/// How flips are issued while the PACING section is in force — the conservative
/// tier, so no `Always` here: that is what the tearing section is for.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PaceMode {
    #[default]
    Fixed,
    Adaptive,
}

impl PaceMode {
    pub const ALL: [PaceMode; 2] = [Self::Fixed, Self::Adaptive];
}

/// What a [`Rate`] is counted against.
///
/// The distinction is invisible at 60fps and decisive above it: a compositor that
/// tears or runs nested builds scenes far faster than the panel retraces, so
/// "one per scene build" and "one per refresh" stop meaning the same thing.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Cadence {
    /// Count the compositor's SCENE BUILDS. On the native path that is one per
    /// retrace, so the producer stays phase-locked behind the frame that shows it.
    /// On a tearing or nested host it is the composite rate, which can be far
    /// above the panel — `Multiplier(1.0)` then means one per composite, NOT 60/s.
    #[default]
    Vblank,
    /// Sleep a WALL-CLOCK interval derived from the monitor's mode refresh.
    /// `Multiplier(1.0)` means the panel's rate whatever the composite rate is, so
    /// this is the one to pick when the host runs far above the display. On real
    /// hardware it drifts against the retrace instead of phase-locking to it.
    Timer,
}

impl Cadence {
    pub const ALL: [Cadence; 2] = [Self::Vblank, Self::Timer];
}
