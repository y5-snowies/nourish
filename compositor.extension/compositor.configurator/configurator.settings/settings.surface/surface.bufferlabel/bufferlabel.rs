//! Short control labels for the two triple-buffering settings. The model crates
//! carry the ladders themselves (`background.preset`, `interface.preset`); the
//! words shown for each rung are presentation and live here, next to the panel
//! that draws them — same split as `surface.tearlabel`.

use compositor_model_environment_background_preset::preset::Preset as BackgroundPreset;
use compositor_model_environment_interface_preset::preset::Preset as InterfacePreset;

pub fn background_preset_label(p: BackgroundPreset) -> &'static str {
    match p {
        BackgroundPreset::Maximum => "Maximum",
        BackgroundPreset::Optimized => "Optimized",
        BackgroundPreset::Efficient => "Efficient",
        BackgroundPreset::PowerSaver => "Power saver",
    }
}

pub fn interface_preset_label(p: InterfacePreset) -> &'static str {
    match p {
        InterfacePreset::Maximum => "Maximum",
        InterfacePreset::Optimized => "Optimized",
        InterfacePreset::Efficient => "Efficient",
        InterfacePreset::PowerSaving => "Power saving",
    }
}
