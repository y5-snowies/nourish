//! Balloon text for the tearing/pacing settings — one line per option,
//! shown under the control it belongs to.
//!
//! Every describe states plainly whether the option TEARS. A flip policy that
//! tears without saying so is the failure mode worth designing against here:
//! the user is choosing between artefacts they may not notice for hours, and
//! cannot connect back to a setting that never mentioned them.

use compositor_model_environment_tearing_rate::rate::{PaceMode, Rate, TearMode};
use compositor_model_environment_tearing_select::select::{Exclusivity, Selector};

pub fn selector_describe(s: Selector) -> &'static str {
    match s {
        Selector::Never => "This section never applies.",
        Selector::Always => "Applies to every frame.",
        Selector::Target => "Applies while a tagged window is on screen.",
        Selector::TargetFocused => "Applies while a tagged window is on screen and focused.",
    }
}

pub fn exclusivity_describe(e: Exclusivity) -> &'static str {
    match e {
        Exclusivity::None => "Every client drives the frame cadence, as normal.",
        Exclusivity::Exclusive => {
            "Only tagged windows drive the cadence. Everything else — cursor, \
             animations, other apps — renders when they do, or at the 30 FPS floor."
        }
        Exclusivity::ExclusiveFocused => {
            "Only the focused tagged window drives the cadence. With nothing \
             focused this disengages and normal scheduling returns."
        }
        Exclusivity::Focused => "Only the focused window drives the cadence, tagged or not.",
        Exclusivity::Visible => {
            "Every on-screen window drives the cadence. Offscreen clients and the \
             compositor's own animations no longer do."
        }
    }
}

pub fn tear_mode_describe(m: TearMode) -> &'static str {
    match m {
        TearMode::Always => "Every flip is immediate. Lowest latency, constant tearing.",
        TearMode::Adaptive => {
            "Tears only when the next refresh is far enough away that waiting for \
             it would cost real latency. Fewer seams, slightly higher latency."
        }
    }
}

pub fn pace_mode_describe(m: PaceMode) -> &'static str {
    match m {
        PaceMode::Fixed => {
            "Holds the configured rate. At or below the refresh rate this never \
             tears (rates that do not divide it evenly snap to it); above the \
             refresh rate, tearing is unavoidable."
        }
        PaceMode::Adaptive => {
            "Tears only when the next refresh is far away. Keeps the frame loop \
             moving faster, at the cost of occasional seams."
        }
    }
}

pub fn rate_describe(r: Rate) -> &'static str {
    match r {
        Rate::Uncapped => {
            "No limit — composite as fast as the frame loop allows. Frames the \
             display never shows still cost GPU time."
        }
        Rate::Multiplier(_) => "Rate follows this monitor's refresh rate.",
        Rate::Fps(_) => "A fixed rate, independent of refresh. Applies per monitor.",
    }
}
