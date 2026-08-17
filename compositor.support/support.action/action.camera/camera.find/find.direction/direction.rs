pub use compositor_support_action_camera_find_angle::{Angle, Octant, Snap};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
    Diagonal(Angle, Snap),
}

impl Direction {
    /// Bucket this direction to one of 8 octants for discrete-case logic.
    /// Cardinals map directly. Diagonals snap per their Snap setting; finer
    /// snaps (Sixteenth) still bucket to 8 since that's what we have cases for.
    pub fn octant(self) -> Octant {
        match self {
            Direction::Right => Octant::Right,
            Direction::Left => Octant::Left,
            Direction::Up => Octant::Up,
            Direction::Down => Octant::Down,
            Direction::Diagonal(Angle(a), snap) => {
                let a = a.rem_euclid(360.0);
                let step = match snap {
                    Snap::Cardinal => 90.0,
                    Snap::Octant => 45.0,
                    Snap::Sixteenth => 45.0,
                };
                let idx = ((a / step).round() as i64).rem_euclid((360.0 / step) as i64);
                match (snap, idx) {
                    (Snap::Cardinal, 0) => Octant::Right,
                    (Snap::Cardinal, 1) => Octant::Down,
                    (Snap::Cardinal, 2) => Octant::Left,
                    (Snap::Cardinal, 3) => Octant::Up,
                    (_, 0) => Octant::Right,
                    (_, 1) => Octant::DownRight,
                    (_, 2) => Octant::Down,
                    (_, 3) => Octant::DownLeft,
                    (_, 4) => Octant::Left,
                    (_, 5) => Octant::UpLeft,
                    (_, 6) => Octant::Up,
                    (_, 7) => Octant::UpRight,
                    _ => Octant::Right,
                }
            }
        }
    }

    /// Raw unit vector along this direction. Cardinals are exact; diagonals
    /// use the raw angle (NOT the snapped octant) so projections are accurate.
    /// Returns (ux, uy) where ux²+uy² = 1.
    pub fn unit_vec(self) -> (f64, f64) {
        match self {
            Direction::Right => (1.0, 0.0),
            Direction::Left => (-1.0, 0.0),
            Direction::Down => (0.0, 1.0),
            Direction::Up => (0.0, -1.0),
            Direction::Diagonal(Angle(a), _snap) => {
                let r = a.to_radians();
                (r.cos(), r.sin())
            }
        }
    }
}
