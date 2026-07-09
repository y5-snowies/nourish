//! Direct-scanout policy surface: the toggle the frame executor consults.
//! Delegates the flag construction to `plane.assign` (single source).
//! Default: enabled (FrameFlags::DEFAULT). The `gpu_no_direct_scanout`
//! experimental flag opts out → primary-plane-only compositing, for GPUs whose
//! overlay/cursor planes only accept tiled buffers (e.g. NVIDIA block-linear),
//! where a non-tiled per-plane buffer scans out garbled.

use compositor_developer_environment_experimental_base::base::{self, GpuFlags};
use compositor_kernel_scanout_plane_assign_base::assign::{frame_flags, PlanePolicy};
use smithay::backend::drm::compositor::FrameFlags;

/// The compositor's current direct-scanout policy. Enabled unless the
/// `gpu_no_direct_scanout` experimental flag is set.
pub fn enabled() -> bool {
    !base::get().contains(GpuFlags::NO_DIRECT_SCANOUT)
}

pub fn flags() -> FrameFlags {
    frame_flags(PlanePolicy {
        allow_direct_scanout: enabled(),
    })
}
