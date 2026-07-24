use std::time::Instant;
use smithay::wayland::fractional_scale::FractionalScaleManagerState;
pub use compositor_support_smithay_state_fractional_config::{DebounceCycle, FractionalScaleConfig};
use compositor_support_smithay_state_fractional_debounce::run_tick;

/// What a surface was last told. `scale` is the value the CLIENT currently holds;
/// `idle` is how the surface was CLASSIFIED on the last pass. Keeping the two apart
/// is what makes idle -> visible an observed transition rather than something
/// inferred from the number, so the value itself carries no hidden meaning.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Published {
    pub scale: f64,
    pub idle: bool,
}

impl Published {
    /// Mapped but invisible on every pane: `1.0`, the protocol identity scale, so
    /// the client can release its hi-res buffers.
    pub const IDLE: Self = Self { scale: 1.0, idle: true };

    /// Visible on at least one pane, at its sharpest pane's lattice scale.
    pub fn visible(scale: f64) -> Self {
        Self { scale, idle: false }
    }

    /// The idle state a later visible pass un-idles FROM.
    pub fn is_idle(&self) -> bool {
        self.idle
    }
}

pub struct Fractional {
    pub state: FractionalScaleManagerState,
    pub cfg: FractionalScaleConfig,
    /// Fingerprint of the pending batch as of the last tick.
    pub last_observed: Option<u64>,
    pub cycle: Option<DebounceCycle>,
    /// The cycle elapsed and the batch is waiting only on `min_interval`.
    pub armed: bool,
    pub last_emit_at: Option<Instant>,
    /// Sharpest scale currently held by a visible surface — the seed handed to a
    /// surface at creation, before it is mapped into any pane.
    pub last_emitted_scale: Option<f64>,
}

impl Fractional {
    pub fn set_config(&mut self, cfg: FractionalScaleConfig) {
        self.cfg = cfg;
    }

    /// Debounce the pending batch; `true` means submit all of it now.
    pub fn tick(&mut self, pending: Option<u64>) -> bool {
        let r = run_tick(&self.cfg, self.last_observed, self.cycle, self.armed, self.last_emit_at, pending);
        self.last_observed = r.last_observed;
        self.cycle = r.cycle;
        self.armed = r.armed;
        self.last_emit_at = r.last_emit_at;
        r.fire
    }

    pub fn last_emitted(&self) -> Option<f64> {
        self.last_emitted_scale
    }
}
