use bitflags::bitflags;
use smithay::utils::{Logical, Point};

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct CameraPlacementFlags: u64 {
        // ── Pan strategies (proposers) ──────────────────────────────────────
        const PAN_CENTER          = 1 << 0;
        const PAN_FIT             = 1 << 1;
        const PAN_CORNER          = 1 << 2;

        // ── Zoom strategies (proposers) ─────────────────────────────────────
        const ZOOM_OUT_TO_FIT     = 1 << 3;
        const ZOOM_IN_TO_FIT      = 1 << 4;

        // ── Zoom axis modifiers (combine with ZOOM_*_TO_FIT) ───────────────
        const ZOOM_FIT_HORIZONTAL = 1 << 5;
        const ZOOM_FIT_VERTICAL   = 1 << 6;

        // ── Dominance enablers ──────────────────────────────────────────────
        const PAN_DOMINANCE       = 1 << 7;
        const ZOOM_DOMINANCE      = 1 << 8;

        // ── Pan dominance goals (only meaningful with PAN_DOMINANCE) ───────
        const PAN_GOAL_MIN_MOVEMENT   = 1 << 9;
        const PAN_GOAL_MAX_VISIBILITY = 1 << 10;
        const PAN_GOAL_NO_CUTOFF      = 1 << 11;
        const PAN_GOAL_NO_OVERSHOOT   = 1 << 12;

        // ── Zoom dominance goals (only meaningful with ZOOM_DOMINANCE) ─────
        const ZOOM_GOAL_MIN_CHANGE    = 1 << 13;
        const ZOOM_GOAL_FILL_VIEWPORT = 1 << 14;
        const ZOOM_GOAL_NO_CROP       = 1 << 15;

        // ── Padding ─────────────────────────────────────────────────────────
        /// Apply default padding (5% of min(screen dim), clamped to [100, 500])
        /// to the target only if the chosen zoom permits it without overflow.
        const PAD_DEFAULT             = 1 << 16;
    }
}

/// Result of `compute_placement`. The caller applies or animates toward this state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacementResult {
    pub position: Point<f64, Logical>,
    pub zoom: f64,
}
