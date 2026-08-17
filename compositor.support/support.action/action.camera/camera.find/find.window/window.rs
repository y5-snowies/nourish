use smithay::utils::{Logical, Rectangle};
use uuid::Uuid;

pub type WindowId = Uuid;

#[derive(Clone, Debug)]
pub struct WindowEntry {
    pub id: WindowId,
    pub rect: Rectangle<f64, Logical>,
}

/// Reserved sentinel ID for a synthetic origin window. Real windows must
/// not use this value; with `u64::MAX` collisions are practically impossible.
pub const SYNTHETIC_ORIGIN_ID: Uuid = Uuid::nil();

/// f64 NaN-safe comparator. Treats NaN as Equal so unwrap-free sorting works.
/// Coordinate data should never produce NaN, so this is purely defensive.
#[inline]
pub fn cmp_f64(a: f64, b: f64) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}
