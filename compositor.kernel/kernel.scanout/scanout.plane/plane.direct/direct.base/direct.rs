//! Direct-scanout policy surface: the toggle the frame executor consults.
//! Delegates the flag construction to `plane.assign` (single source).
//!
//! Tearing and direct scanout are MUTUALLY EXCLUSIVE. The kernel only accepts an
//! async page flip for a single-plane, pure-FB-swap atomic commit, so any frame
//! where a buffer got promoted to an overlay — or the cursor moved onto the
//! cursor plane — would carry two planes and lose the async flag. When the live
//! config could tear at all we force `FrameFlags::empty()`, compositing
//! everything (cursor included) into the primary framebuffer, which is the one
//! configuration that can tear. The flip side is the vendored smithay
//! `AtomicDrmSurface::page_flip` / `DrmSurface::set_tearing`.
//!
//! `may_tear` is a property of the RESOLVED POLICY, not of the raw config and
//! not of the individual frame. It therefore tracks the scene: a target window
//! appearing or being panned away retracts and restores plane assignment at
//! runtime, one frame behind. Judging it from the config instead would strip
//! hardware planes — and the hardware cursor with them — permanently, the moment
//! any selector was armed, including on a desktop that never tears.
//!
//! The guarantee runs one way only: a frame that DOES tear always had planes off.
//! The converse does not hold — under either adaptive mode `may_tear` stays true
//! across the whole engagement, so the frames that end up synced still pay for
//! composited planes.

use compositor_kernel_scanout_plane_assign_base::assign::{frame_flags, PlanePolicy};
use smithay::backend::drm::compositor::FrameFlags;

/// Direct scanout is allowed only while nothing in the live config can issue an
/// async flip.
pub fn enabled(may_tear: bool) -> bool {
    !may_tear
}

pub fn flags(may_tear: bool) -> FrameFlags {
    frame_flags(PlanePolicy {
        allow_direct_scanout: enabled(may_tear),
    })
}
