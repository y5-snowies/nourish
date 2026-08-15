use std::hash::Hash;
use compositor_support_action_canvas_layout_rect::Rect;
use compositor_support_action_canvas_layout_minsize::MinSize;
use compositor_support_action_canvas_layout_axis::{Axis, axis_min, axis_len, axis_set_min};
use compositor_support_action_canvas_layout_variant::DistributeVariant;
use compositor_support_action_canvas_layout_spacing::pick_no_primary_spacing;
use compositor_support_action_canvas_layout_primary::distribute_with_primary;

pub fn distribute_axis<W>(
    rects: &mut [(W, Rect)],
    primary: Option<&W>,
    primary_rect: Option<Rect>,
    axis: Axis,
    variant: DistributeVariant,
    min_size: MinSize,
) where W: Eq + Hash {
    let _ = min_size;
    if rects.len() < 2 && primary_rect.is_none() { return; }
    let primary_idx: Option<usize> = primary.and_then(|p| rects.iter().position(|(w, _)| w == p));
    if let Some(pr) = primary_rect {
        distribute_with_primary(rects, primary_idx, pr, axis, variant);
    } else {
        distribute_without_primary(rects, axis, variant);
    }
}

fn distribute_without_primary<W>(rects: &mut [(W, Rect)], axis: Axis, variant: DistributeVariant) {
    let mut order: Vec<usize> = (0..rects.len()).collect();
    order.sort_by(|&a, &b| axis_min(&rects[a].1, axis).partial_cmp(&axis_min(&rects[b].1, axis)).unwrap());
    let n = order.len();
    if n < 2 { return; }

    let bb_min = order.iter().map(|&i| axis_min(&rects[i].1, axis)).fold(f64::INFINITY, f64::min);
    let bb_max = order.iter().map(|&i| axis_min(&rects[i].1, axis) + axis_len(&rects[i].1, axis)).fold(f64::NEG_INFINITY, f64::max);
    let bb_center = (bb_min + bb_max) * 0.5;
    let total_size: f64 = order.iter().map(|&i| axis_len(&rects[i].1, axis)).sum();

    let gaps: Vec<f64> = order.windows(2).map(|w| {
        let a = &rects[w[0]].1;
        let b = &rects[w[1]].1;
        axis_min(b, axis) - (axis_min(a, axis) + axis_len(a, axis))
    }).collect();

    let (target_spacing, _allow_overlap) =
        pick_no_primary_spacing(variant, &gaps, total_size, bb_min, bb_max, n);

    let new_extent = total_size + (n as f64 - 1.0) * target_spacing;
    let start = match variant {
        DistributeVariant::AxisBounded => bb_min,
        _ => bb_center - new_extent * 0.5,
    };

    let mut cursor = start;
    for &i in &order {
        let r = &mut rects[i].1;
        axis_set_min(r, axis, cursor);
        cursor += axis_len(r, axis) + target_spacing;
    }
}
