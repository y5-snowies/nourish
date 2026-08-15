use std::time::Duration;

/// Tunables for the fractional scale debounce logic.
#[derive(Debug, Clone)]
pub struct FractionalScaleConfig {
    /// Hard floor for the emitted scale.
    pub min_scale: f64,
    /// Hard ceiling.
    pub max_scale: f64,
    pub auto_increment: f64,
    /// Quantization step.
    pub step: f64,
    /// Minimum interval between consecutive emits.
    pub min_interval: Duration,
    /// Short rolling debounce window.
    pub debounce_quiet: Duration,
    /// Maximum total debounce duration before a forced trigger.
    pub debounce_max: Duration,
}

impl Default for FractionalScaleConfig {
    fn default() -> Self {
        Self {
            min_scale: 1.25,
            max_scale: 2.25,
            auto_increment: 0.25,
            step: 0.05,
            min_interval: Duration::from_millis(750),
            debounce_quiet: Duration::from_millis(250),
            debounce_max: Duration::from_millis(750),
        }
    }
}

/// State of an in-progress debounce cycle.
#[derive(Debug, Clone, Copy)]
pub struct DebounceCycle {
    /// Instant the first change of this cycle was observed.
    pub started_at: std::time::Instant,
    /// Instant past which zoom is considered quiet.
    pub quiet_after: std::time::Instant,
}
