use smithay::utils::{Logical, Rectangle};
pub use compositor_support_action_camera_find_direction::Direction;

/// Maps semantic axes (primary = ray travel, secondary = band) to coordinates.
pub struct DirAxes {
    pub dir: Direction,
}

impl DirAxes {
    /// Forward edge of rect along travel direction.
    pub fn primary_forward(&self, r: &Rectangle<f64, Logical>) -> f64 {
        match self.dir {
            Direction::Right => r.loc.x + r.size.w,
            Direction::Left => -r.loc.x,
            Direction::Down => r.loc.y + r.size.h,
            Direction::Up => -r.loc.y,
            Direction::Diagonal(..) => {
                let (ux, uy) = self.dir.unit_vec();
                let x = if ux >= 0.0 { r.loc.x + r.size.w } else { r.loc.x };
                let y = if uy >= 0.0 { r.loc.y + r.size.h } else { r.loc.y };
                ux * x + uy * y
            }
        }
    }

    /// Trailing edge of rect along travel direction.
    pub fn primary_back(&self, r: &Rectangle<f64, Logical>) -> f64 {
        match self.dir {
            Direction::Right => r.loc.x,
            Direction::Left => -(r.loc.x + r.size.w),
            Direction::Down => r.loc.y,
            Direction::Up => -(r.loc.y + r.size.h),
            Direction::Diagonal(..) => {
                let (ux, uy) = self.dir.unit_vec();
                let x = if ux >= 0.0 { r.loc.x } else { r.loc.x + r.size.w };
                let y = if uy >= 0.0 { r.loc.y } else { r.loc.y + r.size.h };
                ux * x + uy * y
            }
        }
    }

    /// Lower edge along the perpendicular axis (top for horizontal, left for vertical).
    pub fn secondary_low(&self, r: &Rectangle<f64, Logical>) -> f64 {
        match self.dir {
            Direction::Right | Direction::Left => r.loc.y,
            Direction::Up | Direction::Down => r.loc.x,
            Direction::Diagonal(..) => {
                let (ux, uy) = self.dir.unit_vec();
                let (px, py) = (-uy, ux);
                let xs = [r.loc.x, r.loc.x + r.size.w];
                let ys = [r.loc.y, r.loc.y + r.size.h];
                let mut lo = f64::INFINITY;
                for &x in &xs {
                    for &y in &ys {
                        let s = px * x + py * y;
                        if s < lo { lo = s; }
                    }
                }
                lo
            }
        }
    }

    pub fn secondary_high(&self, r: &Rectangle<f64, Logical>) -> f64 {
        match self.dir {
            Direction::Right | Direction::Left => r.loc.y + r.size.h,
            Direction::Up | Direction::Down => r.loc.x + r.size.w,
            Direction::Diagonal(..) => {
                let (ux, uy) = self.dir.unit_vec();
                let (px, py) = (-uy, ux);
                let xs = [r.loc.x, r.loc.x + r.size.w];
                let ys = [r.loc.y, r.loc.y + r.size.h];
                let mut hi = f64::NEG_INFINITY;
                for &x in &xs {
                    for &y in &ys {
                        let s = px * x + py * y;
                        if s > hi { hi = s; }
                    }
                }
                hi
            }
        }
    }

    /// Length of the rect along the primary axis.
    pub fn primary_len(&self, r: &Rectangle<f64, Logical>) -> f64 {
        match self.dir {
            Direction::Right | Direction::Left => r.size.w,
            Direction::Up | Direction::Down => r.size.h,
            Direction::Diagonal(..) => {
                let (ux, uy) = self.dir.unit_vec();
                ux.abs() * r.size.w + uy.abs() * r.size.h
            }
        }
    }
}
