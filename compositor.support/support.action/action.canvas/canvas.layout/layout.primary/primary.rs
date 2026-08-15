use std::hash::Hash;
use compositor_support_action_canvas_layout_rect::Rect;
use compositor_support_action_canvas_layout_axis::{Axis, axis_min, axis_len, axis_center, axis_set_min};
use compositor_support_action_canvas_layout_variant::DistributeVariant;
use compositor_support_action_canvas_layout_spacing::pick_primary_spacing;

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Side2 { Before, After }

pub fn classify_side(r: &Rect, primary: &Rect, axis: Axis) -> Side2 {
    let pcenter = axis_center(primary, axis);
    let r_min = axis_min(r, axis);
    let r_max = r_min + axis_len(r, axis);
    if r_max <= pcenter { return Side2::Before; }
    if r_min >= pcenter { return Side2::After; }
    let left_reach = pcenter - r_min;
    let right_reach = r_max - pcenter;
    if left_reach >= right_reach { Side2::Before } else { Side2::After }
}

pub fn distribute_with_primary<W>(
    rects: &mut [(W, Rect)],
    primary_idx: Option<usize>,
    primary_rect: Rect,
    axis: Axis,
    variant: DistributeVariant,
) where W: Eq + Hash {
    let mut before: Vec<usize> = Vec::new();
    let mut after: Vec<usize> = Vec::new();
    for i in 0..rects.len() {
        if Some(i) == primary_idx { continue; }
        match classify_side(&rects[i].1, &primary_rect, axis) {
            Side2::Before => before.push(i),
            Side2::After => after.push(i),
        }
    }
    before.sort_by(|&a, &b| axis_min(&rects[a].1, axis).partial_cmp(&axis_min(&rects[b].1, axis)).unwrap());
    after.sort_by(|&a, &b| axis_min(&rects[a].1, axis).partial_cmp(&axis_min(&rects[b].1, axis)).unwrap());

    let p_min = axis_min(&primary_rect, axis);
    let p_max = p_min + axis_len(&primary_rect, axis);

    let gaps_before: Vec<f64> = {
        let mut gs = Vec::new();
        for w in before.windows(2) {
            let a = &rects[w[0]].1; let b = &rects[w[1]].1;
            gs.push(axis_min(b, axis) - (axis_min(a, axis) + axis_len(a, axis)));
        }
        if let Some(&last) = before.last() {
            let a = &rects[last].1;
            gs.push(p_min - (axis_min(a, axis) + axis_len(a, axis)));
        }
        gs
    };
    let gaps_after: Vec<f64> = {
        let mut gs = Vec::new();
        if let Some(&first) = after.first() {
            let b = &rects[first].1;
            gs.push(axis_min(b, axis) - p_max);
        }
        for w in after.windows(2) {
            let a = &rects[w[0]].1; let b = &rects[w[1]].1;
            gs.push(axis_min(b, axis) - (axis_min(a, axis) + axis_len(a, axis)));
        }
        gs
    };

    let (sp_before, _) = pick_primary_spacing(variant, &gaps_before);
    let (sp_after, _) = pick_primary_spacing(variant, &gaps_after);

    if !before.is_empty() {
        let mut anchor = p_min - sp_before;
        for &i in before.iter().rev() {
            let r = &mut rects[i].1;
            let len = axis_len(r, axis);
            let new_min = anchor - len;
            axis_set_min(r, axis, new_min);
            anchor = new_min - sp_before;
        }
    }
    if !after.is_empty() {
        let mut cursor = p_max + sp_after;
        for &i in &after {
            let r = &mut rects[i].1;
            axis_set_min(r, axis, cursor);
            cursor += axis_len(r, axis) + sp_after;
        }
    }
}
