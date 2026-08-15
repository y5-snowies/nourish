use smithay::utils::{Logical, Rectangle};
use compositor_support_action_camera_find_window::WindowEntry;
use compositor_support_action_camera_find_axes::DirAxes;
use compositor_support_action_camera_find_band::{BandState, cycling_primary_start};
use compositor_support_action_camera_find_passes::BasePass;
use compositor_support_action_camera_find_groups::{all_group_pass, extra_group_pass, screen_group_pass};

/// Mutates band and primary_start according to the pass.
pub fn apply_base_pass(
    pass: BasePass,
    band: &mut BandState,
    primary_start: &mut f64,
    primary_start_default: f64,
    origin: &WindowEntry,
    outputs: &[Rectangle<f64, Logical>],
    windows: &[WindowEntry],
    axes: &DirAxes,
) {
    use BasePass::*;
    match pass {
        CyclingBase | CyclingScreen | CyclingScreenStretch | CyclingExtra | CyclingExtraStretch
        | CyclingAll | CyclingAllStretch => {
            *primary_start = cycling_primary_start(band, windows, axes, primary_start_default);
            return;
        }
        _ => {
            *primary_start = primary_start_default;
        }
    }
    match pass {
        Base => {}
        ScreenHigh | ScreenLow | ScreenStretch => {
            screen_group_pass(pass, band, origin, outputs, axes);
        }
        ExtraHigh | ExtraLow | ExtraStretch => {
            extra_group_pass(pass, band, origin, outputs, axes);
        }
        AllHigh | AllLow | AllStretch => {
            all_group_pass(pass, band, windows, axes);
        }
        CyclingBase | CyclingScreen | CyclingScreenStretch | CyclingExtra | CyclingExtraStretch
        | CyclingAll | CyclingAllStretch => unreachable!(),
    }
}
