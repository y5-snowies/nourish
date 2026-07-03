//! Import a Smithay `Dmabuf` as a `wgpu::Texture`. Single-plane only —
//! ARGB8888 LINEAR is single-plane, so we're fine.

use std::os::fd::AsRawFd;

use compositor_support_bevy_core_context_base::WgpuVulkanContext;
use compositor_support_bevy_core_fault_base::WgpuImportError;
use compositor_developer_debug_instance_record::info;
use smithay::backend::allocator::Buffer;
use smithay::backend::allocator::dmabuf::Dmabuf;
use wgpu::TextureUses;
use wgpu::hal::{MemoryFlags, TextureDescriptor as HalTextureDescriptor};

/// Canonical format for our DMABUF round-trip. gbm's ARGB8888 maps to BGRA
/// in API endianness, sRGB so Bevy's content looks right.
pub const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;

/// Usage: render attachment (Bevy draws into), texture binding (sampling),
/// copy src (readback snapshots) + copy dst (fill a frozen snapshot via copy).
pub const TEXTURE_USAGE: wgpu::TextureUsages = wgpu::TextureUsages::RENDER_ATTACHMENT
    .union(wgpu::TextureUsages::TEXTURE_BINDING)
    .union(wgpu::TextureUsages::COPY_SRC)
    .union(wgpu::TextureUsages::COPY_DST);

pub fn import_dmabuf_to_wgpu(
    ctx: &WgpuVulkanContext,
    dmabuf: &Dmabuf,
) -> Result<wgpu::Texture, WgpuImportError> {
    let size = dmabuf.size();
    let fd = dmabuf.handles().next().ok_or(WgpuImportError::NoFd)?;
    let stride = dmabuf.strides().next().ok_or(WgpuImportError::NoStride)?;
    let offset = dmabuf.offsets().next().ok_or(WgpuImportError::NoOffset)?;
    let modifier: u64 = dmabuf.format().modifier.into();

    info!(
        "Importing dmabuf into wgpu: {}x{}, fd={}, stride={}, offset={}, modifier={:#x}",
        size.w, size.h, fd.as_raw_fd(), stride, offset, modifier,
    );

    let fd_owned = dmabuf
        .handles()
        .next()
        .ok_or(WgpuImportError::NoFd)?
        .try_clone_to_owned()
        .map_err(WgpuImportError::FdDup)?;

    let hal_desc = HalTextureDescriptor {
        label: Some("y5_bevy_dmabuf_imported"),
        size: wgpu::Extent3d {
            width: size.w as u32,
            height: size.h as u32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TEXTURE_FORMAT,
        usage: TextureUses::COLOR_TARGET | TextureUses::RESOURCE | TextureUses::COPY_SRC | TextureUses::COPY_DST,
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
        label: Some("y5_bevy_dmabuf_imported"),
        size: wgpu::Extent3d {
            width: size.w as u32,
            height: size.h as u32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TEXTURE_FORMAT,
        usage: TEXTURE_USAGE,
        view_formats: &[],
    };

    let wgpu_texture = unsafe {
        ctx.device
            .create_texture_from_hal::<wgpu::hal::api::Vulkan>(hal_texture, &wgpu_desc)
    };

    info!("wgpu import successful: {}x{} texture", size.w, size.h);
    Ok(wgpu_texture)
}
