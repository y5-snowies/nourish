//! Start-of-gesture attenuation for two-finger touchpad scroll.
//!
//! Lives beside the other trackpad-gesture policy (`gesture.state`) rather than on
//! the Orchestrator: it is pure libinput smoothing, and the state root only holds
//! the value.

/// Softens the start of a two-finger window scroll on libinput touchpads.
///
/// libinput releases the accumulated pre-recognition distance in the first event of
/// a two-finger scroll, so a gesture forwarded to a client starts with a lurch even
/// though the steady cadence is fine. [`factor`](Self::factor) ramps the first few
/// events up to full strength; the gesture's stop event (or a >200ms gap) resets it.
/// Only the window/iced scroll path uses this — canvas pan/zoom is handled elsewhere.
#[derive(Default)]
pub struct FingerScrollRamp {
    count: u32,
    last_msec: u32,
}

impl FingerScrollRamp {
    /// Attenuation in `0..=1` for this event's forwarded amount; advances the ramp.
    pub fn factor(&mut self, time_msec: u32) -> f64 {
        // No stop event delivered but a long gap since the last one → fresh gesture.
        if time_msec.wrapping_sub(self.last_msec) > 200 {
            self.count = 0;
        }
        self.last_msec = time_msec;
        let f = match self.count {
            0 => 0.3,
            1 => 0.5,
            2 => 0.7,
            3 => 0.85,
            _ => 1.0,
        };
        self.count = self.count.saturating_add(1);
        f
    }

    /// Terminating (stop) event: the next gesture ramps from the start again.
    pub fn end(&mut self) {
        self.count = 0;
    }
}
