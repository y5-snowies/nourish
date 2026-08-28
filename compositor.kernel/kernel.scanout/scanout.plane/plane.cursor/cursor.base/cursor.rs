//! Hardware cursor plane — Phase 4 Step 4: the crate now owns its own
//! allocator capability (DRM dumb buffers), removing the GBM dependency from
//! the cursor path. Plane ATTACHMENT remains delegated until the native
//! commit builder (under delegation, smithay's DrmOutputManager drives the
//! cursor plane from the GBM device it was handed; this allocator is what the
//! vulkan-only build uses the moment attachment de-delegates).

use smithay::backend::allocator::dumb::{DumbAllocator, DumbBuffer};
use smithay::backend::allocator::{Allocator, Fourcc, Modifier};
use smithay::backend::drm::DrmDeviceFd;

#[derive(Debug, thiserror::Error)]
pub enum CursorError {
    #[error("cursor plane attachment is delegated to the smithay compositor in Phase 1")]
    Delegated,
    #[error("cursor buffer allocation failed: {0}")]
    Alloc(String),
}

/// The cursor allocator: dumb buffers on the scanout device. GBM-free.
pub struct CursorAllocator {
    allocator: DumbAllocator,
}

impl CursorAllocator {
    pub fn new(fd: DrmDeviceFd) -> Self {
        Self {
            allocator: DumbAllocator::new(fd),
        }
    }

    /// Allocate a cursor-plane buffer. The format is an assertion rather than a
    /// negotiation — every KMS driver takes a linear ARGB cursor — but it is
    /// stated in the format layer with the rest, so no crate outside it names a
    /// format.
    pub fn allocate(&mut self, size: (u32, u32)) -> Result<DumbBuffer, CursorError> {
        let (fourcc, mods) = compositor_kernel_graphic_format_answer_base::answer::constant(compositor_kernel_graphic_format_answer_base::answer::Consumer::CursorPlane);
        self.allocator
            .create_buffer(size.0, size.1, fourcc, &mods)
            .map_err(|e| CursorError::Alloc(format!("{e}")))
    }
}

/// Future surface: set a cursor image on the pipe's cursor plane (requires
/// the native commit builder).
pub fn set_cursor(_buffer: &DumbBuffer, _hotspot: (i32, i32)) -> Result<(), CursorError> {
    Err(CursorError::Delegated)
}
