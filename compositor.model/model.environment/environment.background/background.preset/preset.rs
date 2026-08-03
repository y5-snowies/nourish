//! Recommended knob combinations for background triple buffering.

use compositor_model_environment_background_base::base::{self, TripleBufferBackground};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
/// A ladder from "no visible difference" down to "barely moving", NOT a set of
/// alternatives. Each step buys GPU time by lowering how often the background
/// re-renders — nothing else changes, and the background never loses resolution
/// or detail at any step.
pub enum Preset {
    /// Optimized's rate with the headroom to sustain it: three buffers and
    /// pipelining. Same intent, more resources — for a heavy shader on a fast
    /// panel that misses flips without them.
    Maximum,
    /// A fresh frame for every flip, and nothing spent beyond it. The DEFAULT:
    /// panel rate, hard-capped at panel rate, on two buffers and unpipelined —
    /// efficient in the sense of wasting nothing, not of doing less.
    Optimized,
    /// Half the panel — 30fps on 60Hz. Rarely visible on slow ambience, and half
    /// the GPU time.
    Efficient,
    /// A quarter of the panel — 15fps on 60Hz. For a Pi with a heavy shader, or
    /// anything on battery. Same rungs as the UI ladder, deliberately: the two
    /// settings sit next to each other and a user should not have to learn two
    /// scales.
    PowerSaver,
}

impl Preset {
    /// In ladder order, smoothest first.
    pub const ALL: [Preset; 4] = [Self::Maximum, Self::Optimized, Self::Efficient, Self::PowerSaver];

    /// This preset's knobs, carrying `base`'s `enabled` and `preset` unchanged —
    /// picking a preset changes what the background does, not how it is edited.
    pub fn apply(self, base: TripleBufferBackground) -> TripleBufferBackground {
        let mut v = match self {
            Self::Maximum => base::MAXIMUM,
            Self::Optimized => base::OPTIMIZED,
            Self::Efficient => base::EFFICIENT,
            Self::PowerSaver => base::POWER_SAVER,
        };
        v.enabled = base.enabled;
        v.preset = base.preset;
        v
    }

    /// The preset `t` exactly matches, if any — for showing which one is active.
    pub fn of(t: &TripleBufferBackground) -> Option<Preset> {
        Self::ALL.into_iter().find(|p| p.apply(*t) == *t)
    }
}
