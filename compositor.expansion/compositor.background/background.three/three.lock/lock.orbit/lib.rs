//! Debug helper: orbit the camera around the origin in the XZ plane during
//! non-idle phases. Set `CAMERA_ORBIT_SPEED = 0.0` to disable for
//! production. When orbit is enabled, the object's own spin is suppressed
//! (in `apply_to_transform`) so the two motions don't compound.

pub mod orbit;
pub use orbit::*;
