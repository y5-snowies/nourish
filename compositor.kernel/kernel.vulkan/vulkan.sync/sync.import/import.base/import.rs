//! fd -> semaphore import + the bridge from the DRM syncobj world (Phase 4
//! Step 2 — real). The acquire direction: a client's syncobj acquire point
//! becomes a semaphore the render submission waits on.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use smithay::backend::drm::DrmDeviceFd;
use smithay::reexports::drm::control::{syncobj, Device as ControlDevice};
use std::os::unix::io::{FromRawFd, IntoRawFd, OwnedFd};

#[derive(Debug, thiserror::Error)]
pub enum SemImportError {
    #[error("vkImportSemaphoreFdKHR failed: {0}")]
    Import(String),
    #[error("syncobj bridge failed: {0}")]
    Bridge(String),
}

/// Import an opaque-fd semaphore payload into `semaphore`. The fd is consumed
/// by the driver on success (Vulkan external-semaphore fd semantics).
pub fn import_opaque_fd(
    device: &VulkanDevice,
    semaphore: vk::Semaphore,
    fd: OwnedFd,
) -> Result<(), SemImportError> {
    let loader =
        ash::khr::external_semaphore_fd::Device::new(&device.instance, &device.device);
    let info = vk::ImportSemaphoreFdInfoKHR::default()
        .semaphore(semaphore)
        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD)
        .fd(fd.into_raw_fd());
    unsafe {
        loader
            .import_semaphore_fd(&info)
            .map_err(|e| SemImportError::Import(format!("{e}")))
    }
}

/// Import a client's acquire fence (a `sync_file` fd) into `semaphore` as a
/// TEMPORARY payload (SYNC_FD semantics): the render submission can then add
/// `semaphore` as a wait so it does not sample the client buffer until the
/// client's GPU work has completed. This is the acquire half of explicit sync
/// (`linux-drm-syncobj-v1`). The fd is consumed by the driver on success.
pub fn import_sync_file(
    device: &VulkanDevice,
    semaphore: vk::Semaphore,
    fd: OwnedFd,
) -> Result<(), SemImportError> {
    let loader =
        ash::khr::external_semaphore_fd::Device::new(&device.instance, &device.device);
    // The driver takes ownership of the fd ONLY on success. `into_raw_fd` has already
    // released it here, so the error arm has to take it back or the fd leaks — which, on a
    // path that runs per frame, reaches `EMFILE` and kills the compositor.
    let raw = fd.into_raw_fd();
    let info = vk::ImportSemaphoreFdInfoKHR::default()
        .semaphore(semaphore)
        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD)
        .flags(vk::SemaphoreImportFlags::TEMPORARY)
        .fd(raw);
    unsafe {
        loader.import_semaphore_fd(&info).map_err(|e| {
            drop(OwnedFd::from_raw_fd(raw));
            SemImportError::Import(format!("{e}"))
        })
    }
}

/// Bridge a DRM syncobj into a freshly created (or supplied) semaphore: the
/// syncobj is exported as an fd and imported as the semaphore's payload.
pub fn bridge_from_syncobj(
    device: &VulkanDevice,
    semaphore: vk::Semaphore,
    drm_fd: &DrmDeviceFd,
    handle: syncobj::Handle,
) -> Result<(), SemImportError> {
    let fd = drm_fd
        .syncobj_to_fd(handle, false)
        .map_err(|e| SemImportError::Bridge(format!("syncobj_to_fd: {e}")))?;
    import_opaque_fd(device, semaphore, fd)
}
