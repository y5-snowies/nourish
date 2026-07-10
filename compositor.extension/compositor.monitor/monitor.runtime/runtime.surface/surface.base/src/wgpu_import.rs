//! Import a Smithay `Dmabuf` as a `wgpu::Texture`.
//!
//! Uses the Vulkan HAL escape hatch (`Device::as_hal::<wgpu::hal::api::Vulkan>`)
//! to call `texture_from_dmabuf_fd`. Single-plane only (the WGPU-HAL API
//! doesn't support multi-plane today). ARGB8888 LINEAR from gbm is always
//! single-plane, so we're fine.

use std::os::fd::AsRawFd;

use smithay::backend::allocator::Buffer;
use smithay::backend::allocator::dmabuf::Dmabuf;
use wgpu::hal::TextureDescriptor as HalTextureDescriptor;
use wgpu::hal::{MemoryFlags};
use wgpu::TextureUses;
use crate::error::WgpuImportError;
use crate::wgpu_context::WgpuVulkanContext;

/// Canonical format for our DMABUF round-trip.
///
/// gbm's `Argb8888` Fourcc maps to BGRA in API endianness, so this is what
/// WGPU sees. sRGB so iced_wgpu's text rendering looks right.
pub const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;

/// Usage flags applied to imported textures. Render attachment for Iced
/// drawing into, texture binding for sampling (rare, but cheap to include),
/// copy source for debugging snapshots.
pub const TEXTURE_USAGE: wgpu::TextureUsages = wgpu::TextureUsages::RENDER_ATTACHMENT
    .union(wgpu::TextureUsages::TEXTURE_BINDING)
    .union(wgpu::TextureUsages::COPY_SRC);

/// Usage flags for a dmabuf imported purely as a copy DESTINATION — the LINEAR
/// scanout buffer the untiling blit writes into (see `document/GPU_UNTILE_BLIT.md`).
/// `COPY_DST` is a strictly weaker requirement than `RENDER_ATTACHMENT`: it is
/// what lets the render GPU accept a foreign (scanout-card) LINEAR dmabuf even on
/// hardware that refuses LINEAR as a color target (NVIDIA proprietary observed).
pub const TEXTURE_USAGE_TRANSFER_DST: wgpu::TextureUsages =
    wgpu::TextureUsages::COPY_DST.union(wgpu::TextureUsages::COPY_SRC);

/// Import a Smithay `Dmabuf` as a `wgpu::Texture`.
///
/// The returned texture is usable as a render attachment. It shares GPU
/// memory with the dmabuf — what Iced renders here will be visible to a
/// GLES import of the same dmabuf.
pub fn import_dmabuf_to_wgpu(
    ctx: &WgpuVulkanContext,
    dmabuf: &Dmabuf,
) -> Result<wgpu::Texture, WgpuImportError> {
    import_dmabuf_to_wgpu_with(
        ctx,
        dmabuf,
        TextureUses::COLOR_TARGET | TextureUses::RESOURCE,
        TEXTURE_USAGE,
    )
}

/// Import a Smithay `Dmabuf` as a copy-DESTINATION `wgpu::Texture`.
///
/// Same import machinery as [`import_dmabuf_to_wgpu`] but with `COPY_DST` (HAL
/// `TransferDst`) usage instead of color-attachment. Used for the untiling blit's
/// LINEAR output buffer, which lives on the scanout card and is written by the
/// render GPU via `copy_texture_to_texture` — never rendered into directly.
pub fn import_dmabuf_to_wgpu_transfer_dst(
    ctx: &WgpuVulkanContext,
    dmabuf: &Dmabuf,
) -> Result<wgpu::Texture, WgpuImportError> {
    import_dmabuf_to_wgpu_with(
        ctx,
        dmabuf,
        TextureUses::COPY_DST | TextureUses::COPY_SRC,
        TEXTURE_USAGE_TRANSFER_DST,
    )
}

/// Shared import body: the HAL escape hatch + `create_texture_from_hal`, with the
/// caller choosing the HAL `TextureUses` and the wgpu `TextureUsages`.
fn import_dmabuf_to_wgpu_with(
    ctx: &WgpuVulkanContext,
    dmabuf: &Dmabuf,
    hal_uses: TextureUses,
    wgpu_usage: wgpu::TextureUsages,
) -> Result<wgpu::Texture, WgpuImportError> {
    let size = dmabuf.size();
    let fd = dmabuf.handles().next().ok_or(WgpuImportError::NoFd)?;
    let stride = dmabuf.strides().next().ok_or(WgpuImportError::NoStride)?;
    let offset = dmabuf.offsets().next().ok_or(WgpuImportError::NoOffset)?;
    let modifier: u64 = dmabuf.format().modifier.into();

    trace!(
        "Importing dmabuf into wgpu: {}x{}, fd={}, stride={}, offset={}, modifier={:#x}",
        size.w,
        size.h,
        fd.as_raw_fd(),
        stride,
        offset,
        modifier,
    );

    // Re-borrow and dup; we hand WGPU an owned fd it can keep for the
    // lifetime of the texture.
    let fd_owned = dmabuf
        .handles()
        .next()
        .ok_or(WgpuImportError::NoFd)?
        .try_clone_to_owned()
        .map_err(WgpuImportError::FdDup)?;

    let hal_desc = HalTextureDescriptor {
        label: Some("y5_iced_dmabuf_imported"),
        size: wgpu::Extent3d {
            width: size.w as u32,
            height: size.h as u32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TEXTURE_FORMAT,
        usage: hal_uses,
        memory_flags: MemoryFlags::empty(),
        view_formats: vec![],
    };

    let hal_texture = unsafe {
        let hal_device_guard = ctx.device.as_hal::<wgpu::hal::api::Vulkan>();
        let hal_device = hal_device_guard
            .as_ref()
            .ok_or(WgpuImportError::NotVulkanBackend)?;

        if dmabuf.num_planes() == 1 {
            hal_device
                .texture_from_dmabuf_fd(fd_owned, &hal_desc, modifier, stride as u64, offset as u64)
                .map_err(WgpuImportError::HalImport)?
        } else {
            // Multi-plane (AMD DCC / Intel CCS): all planes live in the single BO
            // we allocated, so import fd[0] with every plane's (offset, stride).
            let planes: Vec<(u64, u64)> = dmabuf
                .offsets()
                .zip(dmabuf.strides())
                .map(|(o, s)| (o as u64, s as u64))
                .collect();
            hal_device
                .texture_from_dmabuf_fd_planar(fd_owned, &hal_desc, modifier, &planes)
                .map_err(WgpuImportError::HalImport)?
        }
    };

    let wgpu_desc = wgpu::TextureDescriptor {
        label: Some("y5_iced_dmabuf_imported"),
        size: wgpu::Extent3d {
            width: size.w as u32,
            height: size.h as u32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TEXTURE_FORMAT,
        usage: wgpu_usage,
        view_formats: &[],
    };

    let wgpu_texture = unsafe {
        ctx.device
            .create_texture_from_hal::<wgpu::hal::api::Vulkan>(hal_texture, &wgpu_desc)
    };

    trace!("wgpu import successful: {}x{} texture", size.w, size.h);
    Ok(wgpu_texture)
}
