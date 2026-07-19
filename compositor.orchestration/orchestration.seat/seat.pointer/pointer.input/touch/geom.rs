//! Touch coordinate helpers: normalized fraction → physical screen point → y5
//! world point, using the same projection the pointer's absolute-motion path uses.
use smithay::utils::{Logical, Physical, Point};
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_core_state_base::{Loop, Transform};

/// Physical screen point for a normalized 0..1 touch fraction.
pub fn physical(_loop: &Loop, nx: f64, ny: f64) -> Point<f64, Physical> {
    let (pw, ph) = _loop.size_ctx_all().screen_size_physical;
    Point::from((nx * pw, ny * ph))
}

/// y5-world (storage) point for a physical screen point, projected through the
/// camera/region of the pane under it.
pub fn world(_loop: &mut Loop, phys: Point<f64, Physical>) -> Point<f64, Logical> {
    let ctx = _loop.pointer_context(phys);
    let t: Transform = (phys, ctx).into();
    t.into_storage_point_f64()
}

/// Normalized 0..1 fraction of a physical screen point (inverse of `physical`).
pub fn fraction_of(_loop: &Loop, phys: Point<f64, Physical>) -> (f64, f64) {
    let (pw, ph) = _loop.size_ctx_all().screen_size_physical;
    (
        if pw > 0.0 { phys.x / pw } else { 0.0 },
        if ph > 0.0 { phys.y / ph } else { 0.0 },
    )
}
