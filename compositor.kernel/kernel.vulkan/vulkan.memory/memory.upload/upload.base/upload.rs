//! CPU → VkImage staging upload (create + in-place region update).

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use compositor_kernel_vulkan_renderer_error_base::VulkanError;
use smithay::backend::allocator::Fourcc;
use smithay::backend::vulkan::PhysicalDevice;
use smithay::utils::{Buffer as BufferCoord, Rectangle, Size};

/// 4 bytes per pixel — the SHM/memory formats this renderer accepts
/// (A/X RGB/BGR 8888) are all 32-bit.
const BPP: usize = 4;

/// An uploaded device-local sampled image + its backing resources.
pub struct UploadedImage {
    pub image: vk::Image,
    pub memory: vk::DeviceMemory,
    pub view: vk::ImageView,
    pub format: vk::Format,
    pub width: u32,
    pub height: u32,
    /// The allocation size, needed by a second device importing this memory: the
    /// importer must allocate exactly this rather than its own requirement.
    pub size: u64,
}

/// A host-visible staging buffer reused across uploads. Grows on demand; never
/// shrinks. Freed via [`StagingBuffer::destroy`] in the renderer's `Drop`.
#[derive(Default)]
pub struct StagingBuffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    capacity: u64,
}

impl StagingBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ensure the staging buffer holds at least `needed` bytes, then copy `data`
    /// (`data.len() <= needed`) into it. Returns the buffer handle to copy from.
    fn stage(
        &mut self,
        dev: &VulkanDevice,
        phd: &PhysicalDevice,
        data: &[u8],
    ) -> Result<vk::Buffer, VulkanError> {
        let needed = data.len() as u64;
        let device = &dev.device;
        if self.capacity < needed {
            // Grow (and drop the old allocation). Round up to reduce churn when a
            // surface's buffer size creeps.
            let new_cap = needed.next_power_of_two().max(4096);
            unsafe {
                if self.buffer != vk::Buffer::null() {
                    device.destroy_buffer(self.buffer, None);
                }
                if self.memory != vk::DeviceMemory::null() {
                    device.free_memory(self.memory, None);
                }
            }
            let buffer = unsafe {
                device.create_buffer(
                    &vk::BufferCreateInfo::default()
                        .size(new_cap)
                        .usage(vk::BufferUsageFlags::TRANSFER_SRC)
                        .sharing_mode(vk::SharingMode::EXCLUSIVE),
                    None,
                )?
            };
            let req = unsafe { device.get_buffer_memory_requirements(buffer) };
            let idx = find_memory_type(
                dev,
                phd,
                req.memory_type_bits,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            )
            .ok_or(VulkanError::Import("no host-visible memory type".into()))?;
            let memory = unsafe {
                device
                    .allocate_memory(
                        &vk::MemoryAllocateInfo::default()
                            .allocation_size(req.size.max(new_cap))
                            .memory_type_index(idx),
                        None,
                    )
                    .inspect_err(|_| device.destroy_buffer(buffer, None))?
            };
            unsafe { device.bind_buffer_memory(buffer, memory, 0)? };
            self.buffer = buffer;
            self.memory = memory;
            self.capacity = new_cap;
        }
        unsafe {
            let ptr = device.map_memory(self.memory, 0, needed, vk::MemoryMapFlags::empty())?
                as *mut u8;
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
            device.unmap_memory(self.memory);
        }
        Ok(self.buffer)
    }

    pub fn destroy(&self, dev: &VulkanDevice) {
        unsafe {
            if self.buffer != vk::Buffer::null() {
                dev.device.destroy_buffer(self.buffer, None);
            }
            if self.memory != vk::DeviceMemory::null() {
                dev.device.free_memory(self.memory, None);
            }
        }
    }
}

fn find_memory_type(
    dev: &VulkanDevice,
    phd: &PhysicalDevice,
    type_bits: u32,
    props: vk::MemoryPropertyFlags,
) -> Option<u32> {
    let mem = unsafe { dev.instance.get_physical_device_memory_properties(phd.handle()) };
    (0..mem.memory_type_count).find(|&i| {
        (type_bits & (1 << i)) != 0 && mem.memory_types[i as usize].property_flags.contains(props)
    })
}

const COLOR_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

/// Run a one-time-submit command buffer synchronously (alloc → record → submit
/// → fence wait → free). The wait is scoped to THIS submission via its own
/// VkFence — a `device_wait_idle` here would also drain the in-flight
/// composite on the native IN_FENCE path (a full pipeline stall per SHM
/// upload).
fn one_time<F: FnOnce(vk::CommandBuffer)>(
    dev: &VulkanDevice,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    record: F,
) -> Result<(), VulkanError> {
    let device = &dev.device;
    let info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let cmd = unsafe { device.allocate_command_buffers(&info)? }[0];
    unsafe {
        device.begin_command_buffer(
            cmd,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )?;
        record(cmd);
        device.end_command_buffer(cmd)?;
        let cmds = [cmd];
        let fence = device.create_fence(&vk::FenceCreateInfo::default(), None)?;
        let submit = vk::SubmitInfo::default().command_buffers(&cmds);
        let result = device
            .queue_submit(queue, &[submit], fence)
            .and_then(|()| device.wait_for_fences(&[fence], true, u64::MAX));
        device.destroy_fence(fence, None);
        device.free_command_buffers(command_pool, &cmds);
        result?;
    }
    Ok(())
}

/// Allocate a device-local SAMPLED image and upload `data` (tightly packed
/// `width*height*4`, no row padding) in full.
#[allow(clippy::too_many_arguments)]
pub fn create_and_upload(
    dev: &VulkanDevice,
    phd: &PhysicalDevice,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    staging: &mut StagingBuffer,
    data: &[u8],
    format: Fourcc,
    size: Size<i32, BufferCoord>,
    // `shareable`: create the image `OPAQUE_FD`-exportable so a second device can
    // sample it. A PARAMETER rather than a flag this crate reads, because the answer
    // changes with the selected bundle and only the caller knows it.
    shareable: bool,
) -> Result<UploadedImage, VulkanError> {
    let vk_format = compositor_kernel_vulkan_format_query_base::query::vk_format(format)
        .ok_or(VulkanError::UnsupportedFormat(format))?;
    let (width, height) = (size.w.max(1) as u32, size.h.max(1) as u32);
    let expected = width as usize * height as usize * BPP;
    if data.len() < expected {
        return Err(VulkanError::Import(format!(
            "shm buffer too small: {} < {expected} ({width}x{height})",
            data.len()
        )));
    }

    let device = &dev.device;

    // Device-local sampled image (TRANSFER_DST for the upload), created SHAREABLE
    // only when the selected bundle asked for window textures.
    //
    // A SHM surface has no dmabuf, so this image is the only handle on its pixels
    // — and the background worker needs to sample it. `OPAQUE_FD` external memory
    // makes that possible between two logical devices on the same physical device,
    // which is exactly this arrangement (`worker.physical` picks the compositor's
    // node, no fallback). Unlike a dmabuf export it needs no DRM format modifier
    // and keeps OPTIMAL tiling.
    //
    // But it is NOT free, which is why `shareable` is asked rather than assumed —
    // see the parameter. Changing it mid-session does NOT strand already-uploaded
    // surfaces: `shm_cache` refuses to reuse a cached texture whose shareability no
    // longer matches, so the next commit reallocates it correctly, in either
    // direction.
    let mut ext_image = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
    let image_info = compositor_kernel_vulkan_memory_external_base::external::image_info(
        vk_format,
        width,
        height,
        shareable.then_some(&mut ext_image),
    );
    let image = unsafe { device.create_image(&image_info, None)? };
    let req = unsafe { device.get_image_memory_requirements(image) };
    let mem_idx = find_memory_type(dev, phd, req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)
        .ok_or(VulkanError::Import("no device-local memory type".into()))?;
    let mut ext_alloc = vk::ExportMemoryAllocateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
    let mut alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(req.size)
        .memory_type_index(mem_idx);
    if shareable {
        alloc_info = alloc_info.push_next(&mut ext_alloc);
    }
    let memory = unsafe {
        device
            .allocate_memory(
                &alloc_info,
                None,
            )
            .inspect_err(|_| {
                device.destroy_image(image, None);
            })?
    };
    unsafe { device.bind_image_memory(image, memory, 0)? };

    // X-formats are opaque (no real alpha) — force alpha to 1 so the window
    // doesn't blend out transparent (see the dmabuf import path).
    let opaque = matches!(
        format,
        Fourcc::Xrgb8888 | Fourcc::Xbgr8888 | Fourcc::Xrgb2101010 | Fourcc::Xbgr2101010
    );
    let components = vk::ComponentMapping {
        r: vk::ComponentSwizzle::IDENTITY,
        g: vk::ComponentSwizzle::IDENTITY,
        b: vk::ComponentSwizzle::IDENTITY,
        a: if opaque {
            vk::ComponentSwizzle::ONE
        } else {
            vk::ComponentSwizzle::IDENTITY
        },
    };
    let view = unsafe {
        device.create_image_view(
            &vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(vk_format)
                .components(components)
                .subresource_range(COLOR_RANGE),
            None,
        )?
    };

    let buffer = staging.stage(dev, phd, &data[..expected])?;

    one_time(dev, command_pool, queue, |cmd| unsafe {
        let to_dst = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE)
            .dst_stage_mask(vk::PipelineStageFlags2::COPY)
            .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .image(image)
            .subresource_range(COLOR_RANGE)];
        device.cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().image_memory_barriers(&to_dst));

        let region = [vk::BufferImageCopy::default()
            .buffer_offset(0)
            .buffer_row_length(0)
            .buffer_image_height(0)
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
            .image_extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })];
        device.cmd_copy_buffer_to_image(cmd, buffer, image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &region);

        let to_read = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COPY)
            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
            .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image(image)
            .subresource_range(COLOR_RANGE)];
        device.cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().image_memory_barriers(&to_read));
    })
    .inspect_err(|_| unsafe {
        device.destroy_image_view(view, None);
        device.destroy_image(image, None);
        device.free_memory(memory, None);
    })?;

    Ok(UploadedImage {
        image,
        memory,
        view,
        format: vk_format,
        width,
        height,
        size: req.size,
    })
}

/// Re-upload a sub-`region` into an EXISTING sampled image (currently in
/// `SHADER_READ_ONLY_OPTIMAL`). `data` is the full-size buffer with row length =
/// `image_width` (the `ImportMem::update_memory` contract); `region` selects the
/// rect to copy. The region's rows are packed tightly into the staging buffer.
#[allow(clippy::too_many_arguments)]
pub fn update_region(
    dev: &VulkanDevice,
    phd: &PhysicalDevice,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    staging: &mut StagingBuffer,
    image: vk::Image,
    image_size: (u32, u32),
    data: &[u8],
    region: Rectangle<i32, BufferCoord>,
) -> Result<(), VulkanError> {
    let (img_w, img_h) = image_size;
    let rx = region.loc.x.max(0) as u32;
    let ry = region.loc.y.max(0) as u32;
    let rw = (region.size.w.max(0) as u32).min(img_w.saturating_sub(rx));
    let rh = (region.size.h.max(0) as u32).min(img_h.saturating_sub(ry));
    if rw == 0 || rh == 0 {
        return Ok(());
    }
    let src_stride = img_w as usize * BPP;
    let row_bytes = rw as usize * BPP;
    // Pack the region rows tightly into a scratch buffer, then stage that.
    let mut packed = Vec::with_capacity(row_bytes * rh as usize);
    for y in 0..rh as usize {
        let start = (ry as usize + y) * src_stride + rx as usize * BPP;
        let end = start + row_bytes;
        if end > data.len() {
            return Err(VulkanError::Import("update_memory: region row out of bounds".into()));
        }
        packed.extend_from_slice(&data[start..end]);
    }
    let buffer = staging.stage(dev, phd, &packed)?;

    let device = &dev.device;
    one_time(dev, command_pool, queue, |cmd| unsafe {
        let to_dst = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
            .src_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
            .dst_stage_mask(vk::PipelineStageFlags2::COPY)
            .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .image(image)
            .subresource_range(COLOR_RANGE)];
        device.cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().image_memory_barriers(&to_dst));

        let copy = [vk::BufferImageCopy::default()
            .buffer_offset(0)
            .buffer_row_length(0)
            .buffer_image_height(0)
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_offset(vk::Offset3D {
                x: rx as i32,
                y: ry as i32,
                z: 0,
            })
            .image_extent(vk::Extent3D {
                width: rw,
                height: rh,
                depth: 1,
            })];
        device.cmd_copy_buffer_to_image(cmd, buffer, image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &copy);

        let to_read = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COPY)
            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
            .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image(image)
            .subresource_range(COLOR_RANGE)];
        device.cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().image_memory_barriers(&to_read));
    })
}
