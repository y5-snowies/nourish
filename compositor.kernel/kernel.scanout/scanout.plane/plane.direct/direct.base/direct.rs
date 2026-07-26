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
//! `may_tear` is a property of the CONFIG, not of the frame: it changes only
//! when settings do, so planes never churn frame to frame.

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
