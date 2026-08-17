use compositor_support_action_canvas_layout_rect::Rect;

pub const CLOSE_MAX_ITERS: usize = 64;

pub const CLOSE_SNAP_PX: f64 = 10.0;

pub const CLOSE_ALPHA: f64 = 0.5;

#[derive(Copy, Clone)]
pub enum EdgeSel {
    Left,
    Right,
    Top,
    Bottom,
    CenterX,
    CenterY,
}

pub fn edge_value(r: &Rect, sel: EdgeSel) -> f64 {
    match sel {
        EdgeSel::Left => r.left(),
        EdgeSel::Right => r.right(),
        EdgeSel::Top => r.top(),
        EdgeSel::Bottom => r.bottom(),
        EdgeSel::CenterX => r.center_x(),
        EdgeSel::CenterY => r.center_y(),
    }
}

pub fn median_of(vals: &[f64]) -> f64 {
    let mut v: Vec<f64> = vals.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) * 0.5
    }
}

pub fn converge_close<W>(rects: &[(W, Rect)], movable: &[usize], sel: EdgeSel) -> f64 {
    let mut vals: Vec<f64> = movable
        .iter()
        .map(|&i| edge_value(&rects[i].1, sel))
        .collect();
    if vals.is_empty() {
        return 0.0;
    }
    if vals.len() == 1 {
        return vals[0];
    }
    for _ in 0..CLOSE_MAX_ITERS {
        let (vmin, vmax) = vals
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
                (a.min(v), b.max(v))
            });
        if vmax - vmin < CLOSE_SNAP_PX {
            return median_of(&vals);
        }
        let median = median_of(&vals);
        for v in vals.iter_mut() {
            *v += CLOSE_ALPHA * (median - *v);
        }
    }
    median_of(&vals)
}
