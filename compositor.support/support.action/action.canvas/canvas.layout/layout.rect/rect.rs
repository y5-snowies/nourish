/// Axis-aligned rectangle in f64 coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    #[inline]
    pub fn left(&self) -> f64 {
        self.x
    }
    #[inline]
    pub fn right(&self) -> f64 {
        self.x + self.w
    }
    #[inline]
    pub fn top(&self) -> f64 {
        self.y
    }
    #[inline]
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
    #[inline]
    pub fn center_x(&self) -> f64 {
        self.x + self.w * 0.5
    }
    #[inline]
    pub fn center_y(&self) -> f64 {
        self.y + self.h * 0.5
    }

    pub fn bbox_of<'a, I: IntoIterator<Item = &'a Rect>>(rects: I) -> Option<Rect> {
        let mut it = rects.into_iter();
        let first = it.next()?;
        let mut l = first.left();
        let mut r = first.right();
        let mut t = first.top();
        let mut b = first.bottom();
        for rc in it {
            if rc.left() < l { l = rc.left(); }
            if rc.right() > r { r = rc.right(); }
            if rc.top() < t { t = rc.top(); }
            if rc.bottom() > b { b = rc.bottom(); }
        }
        Some(Rect { x: l, y: t, w: r - l, h: b - t })
    }
}

pub const EPSILON: f64 = 0.5;

pub fn rect_eq(a: &Rect, b: &Rect) -> bool {
    (a.x - b.x).abs() < EPSILON
        && (a.y - b.y).abs() < EPSILON
        && (a.w - b.w).abs() < EPSILON
        && (a.h - b.h).abs() < EPSILON
}
