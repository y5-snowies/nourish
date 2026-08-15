use std::hash::Hash;
use compositor_support_action_canvas_layout_rect::Rect;
use compositor_support_action_canvas_layout_minsize::MinSize;
use compositor_support_action_canvas_layout_flags::LayoutFlags;
use compositor_support_action_canvas_layout_axis::Axis;
use compositor_support_action_canvas_layout_side::{
    AxisAlign, resolve_axis_mode, apply_axis, clamp_min,
};

use compositor_support_action_canvas_layout_close::align_close;

pub fn align<W>(
    rects: &mut [(W, Rect)],
    primary: Option<&W>,
    primary_rect: Option<Rect>,
    flags: LayoutFlags,
    min_size: MinSize,
) where
    W: Eq + Hash,
{
    use LayoutFlags as F;
    if rects.len() <= 1 && primary.is_none() { return; }

    let close_mode = flags.contains(F::ALIGN_CLOSE) && primary.is_none();
    let primary_idx: Option<usize> = primary.and_then(|p| rects.iter().position(|(w, _)| w == p));

    let target_default: Rect = if let Some(pr) = primary_rect {
        pr
    } else {
        match Rect::bbox_of(rects.iter().map(|(_, r)| r)) {
            Some(b) => b,
            None => return,
        }
    };

    let h_left = flags.contains(F::ALIGN_LEFT);
    let h_right = flags.contains(F::ALIGN_RIGHT);
    let h_center = flags.contains(F::ALIGN_CENTER_HORIZONTAL);
    let v_top = flags.contains(F::ALIGN_TOP);
    let v_bottom = flags.contains(F::ALIGN_BOTTOM);
    let v_center = flags.contains(F::ALIGN_CENTER_VERTICAL);

    let h_mode = resolve_axis_mode(
        h_left, h_right, h_center,
        flags.contains(F::ALIGN_STRETCH_LEFT),
        flags.contains(F::ALIGN_STRETCH_RIGHT),
    );
    let v_mode = resolve_axis_mode(
        v_top, v_bottom, v_center,
        flags.contains(F::ALIGN_STRETCH_TOP),
        flags.contains(F::ALIGN_STRETCH_BOTTOM),
    );

    let no_specific_h = matches!(h_mode, AxisAlign::None);
    let no_specific_v = matches!(v_mode, AxisAlign::None);
    let bare_align = no_specific_h && no_specific_v;

    for i in 0..rects.len() {
        if Some(i) == primary_idx { continue; }
        let target = target_default;
        let r = &mut rects[i].1;
        if bare_align {
            r.x = target.center_x() - r.w * 0.5;
            r.y = target.center_y() - r.h * 0.5;
            continue;
        }
        apply_axis(r, &target, Axis::X, h_mode);
        apply_axis(r, &target, Axis::Y, v_mode);
        clamp_min(r, min_size);
    }

    if close_mode && !bare_align {
        align_close(rects, primary_idx, h_mode, v_mode, min_size);
    }
}
