//! `import_shm_buffer` with per-surface texture reuse.
//!
//! Mirrors the vendored GLES renderer: the imported SHM texture is cached in the
//! surface's `data_map` (via `texture.cache`), keyed by `ContextId`. A surface
//! that re-commits the same-sized buffer (the common case for an animating app)
//! reuses its `VkImage` and re-uploads only the damaged region (`update_memory`)
//! instead of allocating a fresh device-local image every frame — the dominant
//! source of the Vulkan path's excess memory churn vs. GLES.

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::{ImportMem, Texture};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::utils::{Buffer as BufferCoord, Rectangle, Size};
use smithay::wayland::compositor::SurfaceData;
use smithay::wayland::shm::with_buffer_contents;

use crate::error::VulkanError;
use crate::renderer::VulkanRenderer;
use crate::texture::VulkanTexture;

/// Read the SHM pool and repack rows into a tightly-packed RGBA buffer (stride
/// may exceed `width*4`); the returned buffer has row length = `width`.
fn read_packed(
    buffer: &WlBuffer,
) -> Result<(Vec<u8>, Fourcc, i32, i32), VulkanError> {
    with_buffer_contents(buffer, |ptr, len, data| {
        let fourcc = match data.format {
            wl_shm::Format::Argb8888 => Fourcc::Argb8888,
            wl_shm::Format::Xrgb8888 => Fourcc::Xrgb8888,
            // Must match `ImportMem::mem_formats()` — those are advertised to
            // clients over wl_shm, so any format we list there can land here.
            wl_shm::Format::Abgr8888 => Fourcc::Abgr8888,
            wl_shm::Format::Xbgr8888 => Fourcc::Xbgr8888,
            other => return Err(VulkanError::Import(format!("unsupported shm format {other:?}"))),
        };
        let width = data.width.max(0) as usize;
        let height = data.height.max(0) as usize;
        let stride = data.stride.max(0) as usize;
        let offset = data.offset.max(0) as usize;
        let row_bytes = width * 4;
        let mut pixels = Vec::with_capacity(row_bytes * height);
        for y in 0..height {
            let start = offset + y * stride;
            let end = start + row_bytes;
            if end > len {
                return Err(VulkanError::Import("shm row out of bounds".into()));
            }
            let row = unsafe { std::slice::from_raw_parts(ptr.add(start), row_bytes) };
            pixels.extend_from_slice(row);
        }
        Ok((pixels, fourcc, width as i32, height as i32))
    })
    .map_err(|e| VulkanError::Import(format!("shm access: {e:?}")))?
}

pub(crate) fn import_shm_buffer(
    r: &mut VulkanRenderer,
    buffer: &WlBuffer,
    surface: Option<&SurfaceData>,
    damage: &[Rectangle<i32, BufferCoord>],
) -> Result<VulkanTexture, VulkanError> {
    let (pixels, fourcc, width, height) = read_packed(buffer)?;
    let size = Size::from((width, height));

    let Some(surface) = surface else {
        // No surface to hang a cache on (e.g. a one-off import) — allocate fresh.
        return ImportMem::import_memory(r, &pixels, fourcc, size, false);
    };

    let cache = compositor_kernel_vulkan_texture_cache_base::cache::for_surface(surface);
    let id = r.context_id_value();
    // Reuse only if a cached texture matches this buffer's size AND format AND is
    // shareable exactly when the active bundle currently needs it to be.
    //
    // That last clause is what makes the shareability gate safe to change at
    // runtime. Shareability is fixed when the image is ALLOCATED, so without it a
    // bundle selected mid-session would find every existing surface unexportable
    // and one deselected would leave every surface paying for an export nothing
    // reads — unreliable in both directions. Failing the reuse test instead drops
    // the stale image and reallocates it correctly on the surface's next commit.
    let want_shared = r.wants_shared_shm();
    let cached = cache
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .filter(|t| {
            t.width() == width.max(0) as u32
                && t.height() == height.max(0) as u32
                && t.format() == Some(fourcc)
                && t.shared.is_some() == want_shared
        });

    let mut tex = if let Some(cached) = cached {
        // Re-upload into the existing image: the damaged sub-regions, or the
        // whole image when no damage was provided.
        if damage.is_empty() {
            ImportMem::update_memory(
                r,
                &cached,
                &pixels,
                Rectangle::from_loc_and_size((0, 0), (width, height)),
            )?;
        } else {
            for d in damage {
                ImportMem::update_memory(r, &cached, &pixels, *d)?;
            }
        }
        cached
    } else {
        let new = ImportMem::import_memory(r, &pixels, fourcc, size, false)?;
        cache.lock().unwrap().insert(id, new.clone());
        new
    };

    // HDR-only: tag the texture with the surface's color. SDR does no extra work.
    if r.use_hdr() {
        tex.set_surf(surf_from_surface(surface));
    }
    Ok(tex)
}

/// The per-surface composite `surf` flag from a surface's HDR color tag (set by
/// the color-management protocol). `[transfer, is_hdr, 0, 0]`; SDR by default.
pub(crate) fn surf_from_surface(s: &SurfaceData) -> [f32; 4] {
    match compositor_kernel_graphic_color_surface_base::get(s) {
        Some(h) => [h.transfer as f32, if h.is_hdr { 1.0 } else { 0.0 }, 0.0, 0.0],
        None => [0.0; 4],
    }
}
