//! Import a `Dmabuf` into smithay's GLES renderer.

use smithay::backend::allocator::Buffer;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{ImportDma, Texture};

use compositor_support_bevy_core_fault_base::GlesImportError;
use compositor_model_debug_instance_record::info;

pub fn import_dmabuf_to_gles(
    renderer: &mut GlesRenderer,
    dmabuf: &Dmabuf,
) -> Result<GlesTexture, GlesImportError> {
    info!(
        "gles dmabuf import: size={:?}, format={:?}",
        dmabuf.size(),
        dmabuf.format(),
    );

    let texture = renderer
        .import_dmabuf(dmabuf, None)
        .map_err(GlesImportError::ImportFailed)?;

    info!("gles dmabuf import successful: {:?}", texture.size());
    Ok(texture)
}
