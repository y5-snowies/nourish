//! Recommended knob combinations for UI triple buffering.

use compositor_model_environment_interface_base::base::{self, TripleBufferUI};
use serde::{Deserialize, Serialize};

/// A ladder, NOT a set of alternatives. Each step buys GPU time by lowering how
/// often bevy's scenes re-render — nothing loses resolution or detail at any
/// step, and iced is untouched by either (it already only redraws when its UI
/// changes).
///
/// Shorter than the background's on purpose.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Preset {
    /// Optimized's rate with the headroom to sustain it: three buffers and
    /// pipelining. Same intent, more resources — for a scene that misses flips
    /// without them.
    Maximum,
    /// A fresh frame for every flip, and nothing spent beyond it. The DEFAULT:
    /// panel rate, hard-capped at panel rate, on two buffers and unpipelined.
    Optimized,
    /// Half the panel — 30fps on 60Hz. Rarely visible on a slowly-turning scene,
    /// and half the GPU time.
    Efficient,
    /// A quarter of the panel — 15fps on 60Hz. For a Pi, or anything on battery,
    /// where a spinning globe should cost almost nothing.
    PowerSaving,
}

impl Preset {
    /// In ladder order, smoothest first.
    pub const ALL: [Preset; 4] = [Self::Maximum, Self::Optimized, Self::Efficient, Self::PowerSaving];

    /// This preset's knobs, carrying `base`'s `enabled` and `preset` unchanged —
    /// picking a preset changes what the UI does, not how it is edited.
    pub fn apply(self, base: TripleBufferUI) -> TripleBufferUI {
        let mut v = match self {
            Self::Maximum => base::MAXIMUM,
            Self::Optimized => base::OPTIMIZED,
            Self::Efficient => base::EFFICIENT,
            Self::PowerSaving => base::POWER_SAVING,
        };
        v.enabled = base.enabled;
        v.preset = base.preset;
        v
    }

    /// The preset `s` exactly matches, if any — for showing which one is active.
    pub fn of(s: &TripleBufferUI) -> Option<Preset> {
        Self::ALL.into_iter().find(|p| p.apply(*s) == *s)
    }
}
