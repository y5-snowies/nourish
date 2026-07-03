//! dmabuf -> VkImage (client buffer import) — the vulkan implementation of the
//! contract import capability. Single-memory (non-disjoint) plane layout is the
//! always-on path; DISJOINT multi-plane import (Intel CCS-style aux plane on a
//! separate fd) is gated behind the `MULTIPLANE_SUPPORT` master knob.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::{VulkanDevice, MULTIPLANE_SUPPORT};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::Buffer;
use smithay::backend::vulkan::PhysicalDevice;
use std::os::unix::io::{AsRawFd, BorrowedFd, IntoRawFd, RawFd};

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("unsupported fourcc for the vulkan path: {0:?}")]
    UnsupportedFormat(smithay::backend::allocator::Fourcc),
    #[error("disjoint multi-plane import not populated (single-memory path only)")]
    Disjoint,
    #[error("vulkan call failed: {0}")]
    Vk(String),
}

pub struct ImportedImage {
    pub image: vk::Image,
    pub memory: vk::DeviceMemory,
    /// Extra per-plane allocations for a DISJOINT import (empty otherwise).
    pub extra_memory: Vec<vk::DeviceMemory>,
    pub view: vk::ImageView,
    pub format: vk::Format,
    pub size: (u32, u32),
}

/// Contract validation: a full import attempt followed by teardown.
pub fn validate(device: &VulkanDevice, phd: &PhysicalDevice, dmabuf: &Dmabuf) -> bool {
    match import(device, phd, dmabuf) {
        Ok(img) => {
            destroy(device, img);
            true
        }
        Err(e) => {
            trace!("vulkan dmabuf validation failed: {e}");
            false
        }
    }
}

pub fn import(
    device: &VulkanDevice,
    _phd: &PhysicalDevice,
    dmabuf: &Dmabuf,
) -> Result<ImportedImage, ImportError> {
    let fourcc = dmabuf.format().code;
    let modifier = dmabuf.format().modifier;
    let format = compositor_kernel_vulkan_format_query_base::query::vk_format(fourcc)
        .ok_or(ImportError::UnsupportedFormat(fourcc))?;
    let size = dmabuf.size();
    let (width, height) = (size.w as u32, size.h as u32);

    // Per-plane layouts from the dmabuf description.
    let offsets: Vec<u32> = dmabuf.offsets().collect();
    let strides: Vec<u32> = dmabuf.strides().collect();
    let fds: Vec<BorrowedFd<'_>> = dmabuf.handles().collect();
    if fds.is_empty() {
        return Err(ImportError::Vk("dmabuf has no planes".into()));
    }
    // Disjoint = planes living in genuinely DISTINCT memory objects (e.g. Intel
    // CCS with the aux plane on a separate BO). Keyed by (st_dev, st_ino), NOT the
    // raw fd number: gbm dups the fd per plane, so an AMD DCC single-BO buffer
    // (metadata plane at an offset in the SAME memory) has different raw fds but
    // one memory object — it MUST take the non-disjoint bind_single path.
    let disjoint = is_disjoint(&fds);
    if disjoint && !MULTIPLANE_SUPPORT {
        return Err(ImportError::Disjoint);
    }

    let plane_layouts: Vec<vk::SubresourceLayout> = offsets
        .iter()
        .zip(strides.iter())
        .map(|(offset, stride)| vk::SubresourceLayout {
            offset: *offset as u64,
            size: 0, // must be 0 for VK_EXT_image_drm_format_modifier
            row_pitch: *stride as u64,
            array_pitch: 0,
            depth_pitch: 0,
        })
        .collect();

    let mut modifier_info = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
        .drm_format_modifier(Into::<u64>::into(modifier))
        .plane_layouts(&plane_layouts);
    let mut external_info = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);

    // DISJOINT images bind one allocation per memory plane.
    let create_flags = if disjoint {
        vk::ImageCreateFlags::DISJOINT
    } else {
        vk::ImageCreateFlags::empty()
    };
    let image_info = vk::ImageCreateInfo::default()
        .flags(create_flags)
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut modifier_info)
        .push_next(&mut external_info);

    let dev = &device.device;
    let image = unsafe {
        dev.create_image(&image_info, None)
            .map_err(|e| ImportError::Vk(format!("create_image: {e}")))?
    };
    let fd_loader = ash::khr::external_memory_fd::Device::new(&device.instance, dev);

    let (memory, extra_memory) = {
        let bound = if disjoint {
            bind_disjoint(dev, &fd_loader, image, &fds)
        } else {
            bind_single(dev, &fd_loader, image, &fds).map(|m| (m, Vec::new()))
        };
        match bound {
            Ok(m) => m,
            Err(e) => {
                unsafe { dev.destroy_image(image, None) };
                return Err(e);
            }
        }
    };

    // Opaque (X-prefixed) formats have no real alpha — the X byte is undefined,
    // so clients often leave it 0. Vulkan maps Xrgb8888 -> B8G8R8A8_UNORM (which
    // HAS alpha), so sampling that 0 makes the window blend out transparent.
    // Force alpha to 1 via a view swizzle for X-formats; ARGB/ABGR keep alpha.
    use smithay::backend::allocator::Fourcc;
    let opaque = matches!(
        fourcc,
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
    let view_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .components(components)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });
    let view = unsafe {
        dev.create_image_view(&view_info, None).map_err(|e| {
            dev.free_memory(memory, None);
            free_mems(dev, &extra_memory);
            dev.destroy_image(image, None);
            ImportError::Vk(format!("create_image_view: {e}"))
        })?
    };

    Ok(ImportedImage {
        image,
        memory,
        extra_memory,
        view,
        format,
        size: (width, height),
    })
}

/// Single allocation imported from the first fd, bound at offset 0. Covers the
/// single-plane case and single-fd multi-plane layouts.
fn bind_single(
    dev: &ash::Device,
    fd_loader: &ash::khr::external_memory_fd::Device,
    image: vk::Image,
    fds: &[BorrowedFd<'_>],
) -> Result<vk::DeviceMemory, ImportError> {
    let requirements = unsafe { dev.get_image_memory_requirements(image) };
    let owned = fds[0]
        .try_clone_to_owned()
        .map_err(|e| ImportError::Vk(format!("fd dup: {e}")))?;
    let memory_type_index =
        pick_memory_type(fd_loader, owned.as_raw_fd(), requirements.memory_type_bits)?;
    let mut import_info = vk::ImportMemoryFdInfoKHR::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
        .fd(owned.into_raw_fd());
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index)
        .push_next(&mut import_info)
        .push_next(&mut dedicated);
    let memory = unsafe {
        dev.allocate_memory(&alloc_info, None)
            .map_err(|e| ImportError::Vk(format!("allocate_memory: {e}")))?
    };
    unsafe {
        dev.bind_image_memory(image, memory, 0).map_err(|e| {
            dev.free_memory(memory, None);
            ImportError::Vk(format!("bind_image_memory: {e}"))
        })?;
    }
    Ok(memory)
}

/// DISJOINT bind: one allocation per memory plane (each from its own fd), bound
/// via vkBindImageMemory2 with per-plane aspects. Returns (plane0, [plane1..]).
fn bind_disjoint(
    dev: &ash::Device,
    fd_loader: &ash::khr::external_memory_fd::Device,
    image: vk::Image,
    fds: &[BorrowedFd<'_>],
) -> Result<(vk::DeviceMemory, Vec<vk::DeviceMemory>), ImportError> {
    let mut memories: Vec<vk::DeviceMemory> = Vec::with_capacity(fds.len());
    for (i, fd) in fds.iter().enumerate() {
        let aspect = plane_aspect(i).ok_or(ImportError::Disjoint)?;
        let mut plane_req = vk::ImagePlaneMemoryRequirementsInfo::default().plane_aspect(aspect);
        let req_info = vk::ImageMemoryRequirementsInfo2::default()
            .image(image)
            .push_next(&mut plane_req);
        let mut req2 = vk::MemoryRequirements2::default();
        unsafe { dev.get_image_memory_requirements2(&req_info, &mut req2) };
        let reqs = req2.memory_requirements;

        let result = (|| {
            let owned = fd
                .try_clone_to_owned()
                .map_err(|e| ImportError::Vk(format!("fd dup: {e}")))?;
            let memory_type_index =
                pick_memory_type(fd_loader, owned.as_raw_fd(), reqs.memory_type_bits)?;
            let mut import_info = vk::ImportMemoryFdInfoKHR::default()
                .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
                .fd(owned.into_raw_fd());
            let alloc_info = vk::MemoryAllocateInfo::default()
                .allocation_size(reqs.size)
                .memory_type_index(memory_type_index)
                .push_next(&mut import_info);
            unsafe { dev.allocate_memory(&alloc_info, None) }
                .map_err(|e| ImportError::Vk(format!("allocate_memory: {e}")))
        })();
        match result {
            Ok(mem) => memories.push(mem),
            Err(e) => {
                free_mems(dev, &memories);
                return Err(e);
            }
        }
    }

    let mut plane_infos: Vec<vk::BindImagePlaneMemoryInfo> = (0..fds.len())
        .map(|i| vk::BindImagePlaneMemoryInfo::default().plane_aspect(plane_aspect(i).unwrap()))
        .collect();
    let binds: Vec<vk::BindImageMemoryInfo> = memories
        .iter()
        .zip(plane_infos.iter_mut())
        .map(|(mem, plane)| {
            vk::BindImageMemoryInfo::default()
                .image(image)
                .memory(*mem)
                .memory_offset(0)
                .push_next(plane)
        })
        .collect();
    if let Err(e) = unsafe { dev.bind_image_memory2(&binds) } {
        free_mems(dev, &memories);
        return Err(ImportError::Vk(format!("bind_image_memory2: {e}")));
    }

    let first = memories.remove(0);
    Ok((first, memories))
}

/// Pick a memory type valid for both the image's requirement bits and the
/// dmabuf fd (per VkMemoryFdPropertiesKHR). Selecting an arbitrary type binds
/// incompatible memory that faults the GPU when later sampled.
fn pick_memory_type(
    fd_loader: &ash::khr::external_memory_fd::Device,
    fd: RawFd,
    image_type_bits: u32,
) -> Result<u32, ImportError> {
    let mut fd_props = vk::MemoryFdPropertiesKHR::default();
    unsafe {
        fd_loader
            .get_memory_fd_properties(
                vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
                fd,
                &mut fd_props,
            )
            .map_err(|e| ImportError::Vk(format!("get_memory_fd_properties: {e}")))?;
    }
    let compatible = image_type_bits & fd_props.memory_type_bits;
    if compatible == 0 {
        return Err(ImportError::Vk(
            "no memory type compatible with both the image and the dmabuf fd".into(),
        ));
    }
    Ok(compatible.trailing_zeros())
}

/// Memory identity of a plane fd: `(st_dev, st_ino)`. dup'd fds of one BO share it,
/// so single-BO multi-plane (AMD DCC) resolves to a single object. On fstat failure
/// fall back to a per-fd pseudo-id so distinct fds are treated as distinct.
fn mem_id(fd: &BorrowedFd<'_>) -> (u64, u64) {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } == 0 {
        (st.st_dev as u64, st.st_ino as u64)
    } else {
        (u64::MAX, fd.as_raw_fd() as u64)
    }
}

/// True iff the plane fds span more than one underlying memory object.
fn is_disjoint(fds: &[BorrowedFd<'_>]) -> bool {
    if fds.len() < 2 {
        return false;
    }
    let first = mem_id(&fds[0]);
    fds.iter().any(|fd| mem_id(fd) != first)
}

fn plane_aspect(i: usize) -> Option<vk::ImageAspectFlags> {
    Some(match i {
        0 => vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
        1 => vk::ImageAspectFlags::MEMORY_PLANE_1_EXT,
        2 => vk::ImageAspectFlags::MEMORY_PLANE_2_EXT,
        3 => vk::ImageAspectFlags::MEMORY_PLANE_3_EXT,
        _ => return None,
    })
}

fn free_mems(dev: &ash::Device, mems: &[vk::DeviceMemory]) {
    unsafe {
        for m in mems {
            dev.free_memory(*m, None);
        }
    }
}

pub fn destroy(device: &VulkanDevice, img: ImportedImage) {
    unsafe {
        device.device.destroy_image_view(img.view, None);
        device.device.destroy_image(img.image, None);
        device.device.free_memory(img.memory, None);
        free_mems(&device.device, &img.extra_memory);
    }
}
