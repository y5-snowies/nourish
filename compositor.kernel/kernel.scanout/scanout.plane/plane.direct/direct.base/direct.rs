//! Direct-scanout policy surface: the toggle the frame executor consults.
//! Delegates the flag construction to `plane.assign` (single source).
//! Default: enabled (FrameFlags::DEFAULT). The `no_direct_scanout` mode token
//! (primary node) opts out → primary-plane-only compositing, for GPUs whose
//! overlay/cursor planes only accept tiled buffers (e.g. NVIDIA block-linear),
//! where a non-tiled per-plane buffer scans out garbled.

use compositor_developer_environment_config_mode::mode::ModeFlags;
use compositor_kernel_scanout_plane_assign_base::assign::{frame_flags, PlanePolicy};
use smithay::backend::drm::compositor::FrameFlags;

/// The compositor's current direct-scanout policy. Enabled unless the primary
/// node's `no_direct_scanout` mode token is set.
pub fn enabled() -> bool {
    !compositor_developer_environment_config_router::router::primary_mode()
        .contains(ModeFlags::NO_DIRECT_SCANOUT)
}

pub fn flags() -> FrameFlags {
    frame_flags(PlanePolicy {
        allow_direct_scanout: enabled(),
    })
}
