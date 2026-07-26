//! Short control labels for the tearing/pacing settings. The balloon text that
//! goes under each control lives in `tearing.describe`.

use compositor_model_environment_tearing_rate::rate::{PaceMode, Rate, TearMode};
use compositor_model_environment_tearing_select::select::{Exclusivity, Selector};

pub fn selector_label(s: Selector) -> &'static str {
    match s {
        Selector::Never => "Never",
        Selector::Always => "Always",
        Selector::Target => "Target visible",
        Selector::TargetFocused => "Target focused",
    }
}

pub fn exclusivity_label(e: Exclusivity) -> &'static str {
    match e {
        Exclusivity::None => "None",
        Exclusivity::Exclusive => "Exclusive",
        Exclusivity::ExclusiveFocused => "Exclusive focused",
        Exclusivity::Focused => "Focused",
        Exclusivity::Visible => "Visible",
    }
}

pub fn tear_mode_label(m: TearMode) -> &'static str {
    match m {
        TearMode::Always => "Always",
        TearMode::Adaptive => "Adaptive",
    }
}

pub fn pace_mode_label(m: PaceMode) -> &'static str {
    match m {
        PaceMode::Fixed => "Fixed",
        PaceMode::Adaptive => "Adaptive",
    }
}

pub fn rate_label(r: Rate) -> String {
    match r {
        Rate::Uncapped => "Uncapped".to_string(),
        Rate::Multiplier(m) => format!("{m:.1}x refresh"),
        Rate::Fps(f) => format!("{f:.0} FPS"),
    }
}

