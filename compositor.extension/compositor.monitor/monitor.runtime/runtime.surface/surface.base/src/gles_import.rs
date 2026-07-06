//! Import a Smithay `Dmabuf` as a `GlesTexture`.
//!
//! Straightforward wrapper over `GlesRenderer::import_dmabuf`. The renderer
//! must have the EGL dmabuf-import extensions, which any modern Mesa/NVIDIA
//! driver provides.

use smithay::backend::allocator::Buffer;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::Texture;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};

use crate::error::GlesImportError;

/// Import a `Dmabuf` into a `GlesRenderer` as a sampleable `GlesTexture`.
///
/// The texture is internally Arc-like; cloning is cheap. The underlying GPU
/// resource lives as long as the dmabuf does (i.e., as long as the
/// `AllocatedDmabuf` owner is alive).
pub fn import_dmabuf_to_gles(
    renderer: &mut GlesRenderer,
    dmabuf: &Dmabuf,
) -> Result<GlesTexture, GlesImportError> {
    trace!(
        "Importing dmabuf into GlesRenderer: size={:?}, format={:?}",
        dmabuf.size(),
        dmabuf.format(),
    );

    let texture = renderer
        .import_dmabuf(dmabuf, None)
        .map_err(GlesImportError::ImportFailed)?;

    trace!(
        "GLES import successful: GlesTexture size {:?}",
        texture.size()
    );

    Ok(texture)
}
