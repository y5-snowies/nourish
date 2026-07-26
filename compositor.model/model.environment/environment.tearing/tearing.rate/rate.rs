//! Composite-rate ceiling and the per-section flip modes. UI strings live in
//! `y5.graphic/graphic.tearing/tearing.text`.

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
