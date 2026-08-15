use smithay::utils::{Logical, Rectangle};
use compositor_support_action_camera_find_window::{WindowEntry, WindowId, cmp_f64};
use compositor_support_action_camera_find_flags::WindowFinderFlags;
use compositor_support_action_camera_find_axes::DirAxes;
use compositor_support_action_camera_find_origin::{
    distance_sq_to_viewport_center, is_window_visible, visible_area_fraction,
};

use compositor_support_action_camera_find_band::overlap_area;

pub fn pick_origin(
    flags: WindowFinderFlags,
    focused: Option<WindowId>,
    windows: &[WindowEntry],
    outputs: &[Rectangle<f64, Logical>],
    axes: &DirAxes,
) -> Option<WindowId> {
    use WindowFinderFlags as F;
    if flags.contains(F::ORIGIN_FOCUSED_VISIBLE) {
        if let Some(id) = focused {
            if let Some(w) = windows.iter().find(|w| w.id == id) {
                if is_window_visible(&w.rect, outputs) { return Some(id); }
            }
        }
    }
    if flags.contains(F::ORIGIN_FOCUSED) {
        if let Some(id) = focused {
            if windows.iter().any(|w| w.id == id) { return Some(id); }
        }
    }
    if flags.contains(F::ORIGIN_VISIBLE) {
        if let Some(w) = windows
            .iter()
            .filter(|w| is_window_visible(&w.rect, outputs))
            .min_by(|a, b| {
                cmp_f64(axes.secondary_low(&a.rect), axes.secondary_low(&b.rect))
                    .then_with(|| cmp_f64(axes.primary_back(&a.rect), axes.primary_back(&b.rect)))
            })
        {
            return Some(w.id);
        }
    }
    let want_area = flags.contains(F::ORIGIN_MOST_VISIBLE_AREA);
    let want_center = flags.contains(F::ORIGIN_MOST_CENTERED);
    if want_area || want_center {
        if let Some(w) = windows
            .iter()
            .filter(|w| is_window_visible(&w.rect, outputs))
            .max_by(|a, b| {
                use std::cmp::Ordering;
                let mut cmp = Ordering::Equal;
                if want_area {
                    let af = visible_area_fraction(&a.rect, outputs);
                    let bf = visible_area_fraction(&b.rect, outputs);
                    cmp = cmp_f64(af, bf);
                }
                if cmp == Ordering::Equal && want_center {
                    let ad = distance_sq_to_viewport_center(&a.rect, outputs);
                    let bd = distance_sq_to_viewport_center(&b.rect, outputs);
                    cmp = cmp_f64(bd, ad);
                }
                if cmp == Ordering::Equal && want_area {
                    let aa: f64 = outputs.iter().map(|o| overlap_area(&a.rect, o)).sum();
                    let ba: f64 = outputs.iter().map(|o| overlap_area(&b.rect, o)).sum();
                    cmp = cmp_f64(aa, ba);
                }
                cmp
            })
        {
            return Some(w.id);
        }
    }
    None
}
