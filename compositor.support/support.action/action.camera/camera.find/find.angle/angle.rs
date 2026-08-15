/// Angle in degrees, measured clockwise from "right" (positive x-axis).
/// 0° = Right, 90° = Down, 180° = Left, 270° = Up.
/// Range is [0, 360); callers should normalize but the implementation
/// normalizes defensively too.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Angle(pub f64);

/// Snap granularity for diagonal directions. The snap is only consulted by
/// code paths that need a discrete choice (corner anchors, padding edges,
/// primary-axis split). Projections always use the raw angle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snap {
    /// Snap to the nearest of {0, 90, 180, 270} = the 4 cardinals.
    Cardinal,
    /// Snap to the nearest of 8 octants (every 45°).
    Octant,
    /// Snap to the nearest of 16 (every 22.5°). The discrete-decision sites
    /// still bucket to 8 (because we only have 8 corners/edges), but you can
    /// thread Sixteenth through if a future site needs finer granularity.
    Sixteenth,
}

/// Internal 8-way bucket derived from a Diagonal's (angle, snap). Used only
/// for discrete decisions that have one of 8 natural cases (corner anchors,
/// padding edges). Projections do NOT go through this — they use raw angle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Octant {
    Right,
    DownRight,
    Down,
    DownLeft,
    Left,
    UpLeft,
    Up,
    UpRight,
}

impl Octant {
    /// (x_component, y_component) in screen coords, each in {-1, 0, +1}.
    /// Negative y = upward (screen-y grows down).
    pub fn components(self) -> (i8, i8) {
        match self {
            Octant::Right => (1, 0),
            Octant::DownRight => (1, 1),
            Octant::Down => (0, 1),
            Octant::DownLeft => (-1, 1),
            Octant::Left => (-1, 0),
            Octant::UpLeft => (-1, -1),
            Octant::Up => (0, -1),
            Octant::UpRight => (1, -1),
        }
    }
}
