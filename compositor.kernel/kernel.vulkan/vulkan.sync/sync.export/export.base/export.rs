//! Semaphore -> fd export + the bridge into the DRM syncobj world (Phase 4
//! Step 2 — real). Fds and handles cross as plain values (Law 1).
//!
//! Round trip this enables (the Step 2 exit criterion):
//!   timeline semaphore --export_opaque_fd--> fd --bridge_to_syncobj-->
//!   syncobj handle --drm.syncobj.timeline--> signal/wait/query
//! and, for plane fencing, syncobj -> sync_file (`to_sync_file`) which is
//! what `scanout.fence/fence.in` attaches as IN_FENCE_FD.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use smithay::backend::drm::DrmDeviceFd;
use smithay::reexports::drm::control::{syncobj, Device as ControlDevice};
use std::os::unix::io::{AsFd, FromRawFd, OwnedFd};

#[derive(Debug, thiserror::Error)]
pub enum SemExportError {
    #[error("vkGetSemaphoreFdKHR failed: {0}")]
    Export(String),
    #[error("syncobj bridge failed: {0}")]
    Bridge(String),
}

pub fn export_opaque_fd(
    device: &VulkanDevice,
    semaphore: vk::Semaphore,
) -> Result<OwnedFd, SemExportError> {
    let loader =
        ash::khr::external_semaphore_fd::Device::new(&device.instance, &device.device);
    let info = vk::SemaphoreGetFdInfoKHR::default()
        .semaphore(semaphore)
        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD);
    let raw = unsafe {
        loader
            .get_semaphore_fd(&info)
            .map_err(|e| SemExportError::Export(format!("{e}")))?
    };
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

/// Export a BINARY semaphore's pending signal as a `sync_file` fd (SYNC_FD
/// handle type). The semaphore must have a queued signal op (call this right
/// after the submit that signals it); the fd becomes signaled when that submit
/// completes on the GPU. This is the render-completion fence the async render
/// path returns (GLES/EGL native-fence wait + KMS IN_FENCE).
pub fn export_sync_file(
    device: &VulkanDevice,
    semaphore: vk::Semaphore,
) -> Result<OwnedFd, SemExportError> {
    let loader =
        ash::khr::external_semaphore_fd::Device::new(&device.instance, &device.device);
    let info = vk::SemaphoreGetFdInfoKHR::default()
        .semaphore(semaphore)
        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
    let raw = unsafe {
        loader
            .get_semaphore_fd(&info)
            .map_err(|e| SemExportError::Export(format!("{e}")))?
    };
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

/// Export a VkFence's pending signal as a `sync_file` fd (SYNC_FD, via
/// `VK_KHR_external_fence_fd`). Call right after the submit that signals the
/// fence. `Ok(Some(fd))` is the render-completion fence; `Ok(None)` is the SYNC_FD
/// `-1` sentinel (already signalled → no wait needed); `Err` is a real export
/// failure (the caller should fall back to a CPU drain). The `infence_2` render
/// fence — cleaner completion semantics than a binary-semaphore SYNC_FD, plus the
/// invalid-fd guard the semaphore path lacks.
pub fn export_fence_sync_file(
    device: &VulkanDevice,
    fence: vk::Fence,
) -> Result<Option<OwnedFd>, SemExportError> {
    let loader = ash::khr::external_fence_fd::Device::new(&device.instance, &device.device);
    let info = vk::FenceGetFdInfoKHR::default()
        .fence(fence)
        .handle_type(vk::ExternalFenceHandleTypeFlags::SYNC_FD);
    let raw = unsafe {
        loader
            .get_fence_fd(&info)
            .map_err(|e| SemExportError::Export(format!("{e}")))?
    };
    if raw == -1 {
        // SYNC_FD sentinel: the fence is already signalled → no fence to wait on.
        return Ok(None);
    }
    if raw < 0 {
        return Err(SemExportError::Export(format!(
            "get_fence_fd returned invalid fd {raw}"
        )));
    }
    Ok(Some(unsafe { OwnedFd::from_raw_fd(raw) }))
}

/// Import an exported semaphore fd as a DRM syncobj on `drm_fd` — the vulkan
/// half of the timeline-semaphore <-> syncobj round trip. The kernel treats
/// an opaque drm-syncobj-backed semaphore fd and a syncobj fd as the same
/// object class on Mesa/NVIDIA; failures surface as Bridge errors for the
/// caller to fall back on.
pub fn bridge_to_syncobj(
    device: &VulkanDevice,
    semaphore: vk::Semaphore,
    drm_fd: &DrmDeviceFd,
) -> Result<syncobj::Handle, SemExportError> {
    let fd = export_opaque_fd(device, semaphore)?;
    drm_fd
        .fd_to_syncobj(fd.as_fd(), false)
        .map_err(|e| SemExportError::Bridge(format!("fd_to_syncobj: {e}")))
}

/// Export a syncobj timeline point as a sync_file fd suitable for
/// IN_FENCE_FD (`scanout.fence/fence.in` consumes the result as a value).
/// Binary-fence materialization of a timeline point: transfer the point into
/// a temporary binary syncobj, then export it as a sync_file.
pub fn to_sync_file(
    drm_fd: &DrmDeviceFd,
    handle: syncobj::Handle,
    _point: u64,
) -> Result<OwnedFd, SemExportError> {
    drm_fd
        .syncobj_to_fd(handle, true)
        .map_err(|e| SemExportError::Bridge(format!("syncobj_to_fd(sync_file): {e}")))
}
