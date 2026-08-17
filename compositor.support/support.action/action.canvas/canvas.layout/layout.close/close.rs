use compositor_support_action_canvas_layout_rect::Rect;
use compositor_support_action_canvas_layout_minsize::MinSize;
use compositor_support_action_canvas_layout_side::{AxisAlign, Side, clamp_min};
use compositor_support_action_canvas_layout_converge::{EdgeSel, converge_close};

pub fn align_close<W>(
    rects: &mut [(W, Rect)],
    primary_idx: Option<usize>,
    h_mode: AxisAlign,
    v_mode: AxisAlign,
    min_size: MinSize,
) {
    let movable: Vec<usize> = (0..rects.len())
        .filter(|i| Some(*i) != primary_idx)
        .collect();
    if movable.len() < 2 {
        return;
    }

    let h_target_min = converge_close(rects, &movable, EdgeSel::Left);
    let h_target_max = converge_close(rects, &movable, EdgeSel::Right);
    let h_target_cx = converge_close(rects, &movable, EdgeSel::CenterX);
    let v_target_min = converge_close(rects, &movable, EdgeSel::Top);
    let v_target_max = converge_close(rects, &movable, EdgeSel::Bottom);
    let v_target_cy = converge_close(rects, &movable, EdgeSel::CenterY);

    for &i in &movable {
        let r = &mut rects[i].1;
        match h_mode {
            AxisAlign::None => {}
            AxisAlign::Edge { side: Side::Min, .. } => { r.x = h_target_min; }
            AxisAlign::Edge { side: Side::Max, .. } => { r.x = h_target_max - r.w; }
            AxisAlign::DualEdgeStretch => {
                r.x = h_target_min;
                r.w = (h_target_max - h_target_min).max(0.0);
            }
            AxisAlign::Center => { r.x = h_target_cx - r.w * 0.5; }
            AxisAlign::CenterAndEdgeStretch(_) => unreachable!(),
        }
        match v_mode {
            AxisAlign::None => {}
            AxisAlign::Edge { side: Side::Min, .. } => { r.y = v_target_min; }
            AxisAlign::Edge { side: Side::Max, .. } => { r.y = v_target_max - r.h; }
            AxisAlign::DualEdgeStretch => {
                r.y = v_target_min;
                r.h = (v_target_max - v_target_min).max(0.0);
            }
            AxisAlign::Center => { r.y = v_target_cy - r.h * 0.5; }
            AxisAlign::CenterAndEdgeStretch(_) => unreachable!(),
        }
        clamp_min(r, min_size);
    }
}
