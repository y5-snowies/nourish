use std::collections::HashMap;
use std::hash::Hash;
use compositor_support_action_canvas_layout_rect::{Rect, rect_eq};
use compositor_support_action_canvas_layout_minsize::MinSize;
use compositor_support_action_canvas_layout_flags::LayoutFlags;
use compositor_support_action_canvas_layout_axis::Axis;
use compositor_support_action_canvas_layout_variant::{DistributeVariant, distribute_variant_h, distribute_variant_v};
use compositor_support_action_canvas_layout_align::align;
use compositor_support_action_canvas_layout_distrib::distribute_axis;

/// Compute new rectangles for a set of windows according to layout flags.
/// Returns only windows whose rect changed. Order: distribute-h, distribute-v, align.
pub fn layout<W>(
    windows: Vec<(W, Rect)>,
    primary: Option<(W, Rect)>,
    flags: LayoutFlags,
    min_size: MinSize,
) -> HashMap<W, Rect>
where
    W: Eq + Hash + Clone,
{
    use LayoutFlags as F;
    if windows.is_empty() { return HashMap::new(); }

    let h_variant = distribute_variant_h(flags);
    let v_variant = distribute_variant_v(flags);
    if primary.is_some()
        && (matches!(h_variant, DistributeVariant::Axis | DistributeVariant::AxisBounded)
            || matches!(v_variant, DistributeVariant::Axis | DistributeVariant::AxisBounded))
    {
        return HashMap::new();
    }

    let input: Vec<(W, Rect)> = windows.clone();
    let mut current: Vec<(W, Rect)> = windows;

    let primary_ref: Option<&W> = primary.as_ref().map(|(w, _)| w);
    let primary_rect: Option<Rect> = primary.as_ref().map(|(_, r)| *r);

    if let (Some(pw), Some(pr)) = (primary_ref, primary_rect) {
        for (w, r) in current.iter_mut() {
            if w == pw { *r = pr; break; }
        }
    }

    if flags.contains(F::DISTRIBUTE_HORIZONTALLY) {
        distribute_axis(&mut current, primary_ref, primary_rect, Axis::X, h_variant, min_size);
    }
    if flags.contains(F::DISTRIBUTE_VERTICALLY) {
        distribute_axis(&mut current, primary_ref, primary_rect, Axis::Y, v_variant, min_size);
    }
    if flags.contains(F::ALIGN) {
        align(&mut current, primary_ref, primary_rect, flags, min_size);
    }

    let mut out = HashMap::new();
    let original: HashMap<&W, &Rect> = input.iter().map(|(w, r)| (w, r)).collect();
    for (w, new_rect) in current.into_iter() {
        if Some(&w) == primary_ref { continue; }
        if let Some(orig) = original.get(&w) {
            if !rect_eq(orig, &new_rect) { out.insert(w, new_rect); }
        }
    }
    out
}
