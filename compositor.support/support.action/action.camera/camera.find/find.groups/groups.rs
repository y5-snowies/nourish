use smithay::utils::{Logical, Rectangle};
use compositor_support_action_camera_find_window::WindowEntry;
use compositor_support_action_camera_find_axes::DirAxes;
use compositor_support_action_camera_find_band::{
    BandState, all_window_edges, output_perpendicular_size, resolve_high_low, screen_edges_for_band,
};

use compositor_support_action_camera_find_passes::BasePass;

pub fn screen_group_pass(
    pass: BasePass,
    band: &mut BandState,
    origin: &WindowEntry,
    outputs: &[Rectangle<f64, Logical>],
    axes: &DirAxes,
) {
    let (lo, hi) = screen_edges_for_band(band, origin, outputs, axes);
    if pass == BasePass::ScreenHigh {
        let (_h_t, _l_t, htl) = resolve_high_low(band, lo, hi);
        band.high_is_top_or_left = htl;
        if htl { band.secondary_low = band.secondary_low.min(lo); }
        else    { band.secondary_high = band.secondary_high.max(hi); }
    } else if pass == BasePass::ScreenLow {
        if band.high_is_top_or_left { band.secondary_high = band.secondary_high.max(hi); }
        else                         { band.secondary_low = band.secondary_low.min(lo); }
    } else {
        // ScreenStretch
        let (_h_t, _l_t, htl) = resolve_high_low(band, lo, hi);
        band.high_is_top_or_left = htl;
        band.secondary_low = band.secondary_low.min(lo);
        band.secondary_high = band.secondary_high.max(hi);
    }
}

pub fn extra_group_pass(
    pass: BasePass,
    band: &mut BandState,
    origin: &WindowEntry,
    outputs: &[Rectangle<f64, Logical>],
    axes: &DirAxes,
) {
    let mon = output_perpendicular_size(origin, outputs, axes);
    let target_low = band.secondary_low - mon;
    let target_high = band.secondary_high + mon;
    if pass == BasePass::ExtraHigh {
        let (_h_t, _l_t, htl) = resolve_high_low(band, target_low, target_high);
        band.high_is_top_or_left = htl;
        if htl { band.secondary_low = target_low; }
        else    { band.secondary_high = target_high; }
    } else if pass == BasePass::ExtraLow {
        if band.high_is_top_or_left { band.secondary_high = target_high; }
        else                         { band.secondary_low = target_low; }
    } else {
        // ExtraStretch
        let (_h_t, _l_t, htl) = resolve_high_low(band, target_low, target_high);
        band.high_is_top_or_left = htl;
        band.secondary_low = target_low;
        band.secondary_high = target_high;
    }
}

pub fn all_group_pass(
    pass: BasePass,
    band: &mut BandState,
    windows: &[WindowEntry],
    axes: &DirAxes,
) {
    let (lo, hi) = all_window_edges(windows, axes);
    if pass == BasePass::AllHigh {
        let (_h_t, _l_t, htl) = resolve_high_low(band, lo, hi);
        band.high_is_top_or_left = htl;
        if htl { band.secondary_low = band.secondary_low.min(lo); }
        else    { band.secondary_high = band.secondary_high.max(hi); }
    } else if pass == BasePass::AllLow {
        if band.high_is_top_or_left { band.secondary_high = band.secondary_high.max(hi); }
        else                         { band.secondary_low = band.secondary_low.min(lo); }
    } else {
        // AllStretch
        let (lo, hi) = all_window_edges(windows, axes);
        let (_h_t, _l_t, htl) = resolve_high_low(band, lo, hi);
        band.high_is_top_or_left = htl;
        band.secondary_low = band.secondary_low.min(lo);
        band.secondary_high = band.secondary_high.max(hi);
    }
}
