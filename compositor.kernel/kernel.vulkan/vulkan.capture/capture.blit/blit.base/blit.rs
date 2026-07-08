//! The capture cache + the per-frame blit.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use compositor_kernel_vulkan_renderer_error_base::VulkanError;
use smithay::backend::allocator::Buffer;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::utils::{Physical, Rectangle};
use std::os::unix::io::AsRawFd;
use std::time::Instant;

/// A capture-target dmabuf imported as a TRANSFER_DST image (no view — a
/// TRANSFER_DST-only image cannot have one).
struct CaptureImport {
    image: vk::Image,
    memory: vk::DeviceMemory,
}

/// Per-target identity used to detect when the capture set changes between
/// frames. The registry entry dmabufs are stable across frames, so an unchanged
/// `(plane-0 inode, width, height)` sequence means "reuse the cached images".
///
/// The identity is the dma-buf **inode**, NOT the raw fd: fd *numbers* are
/// recycled by the kernel once closed, so a freed capture entry and a later,
/// freshly-allocated one can carry the *same* fd number at the same resolution.
/// Keying on the fd then mistakes the new buffer for the cached old one and
/// blits into a stale image, leaving the new entry blank (the "first capture
/// works, later ones intermittently don't" bug). Each `dma_buf` export has its
/// own inode in the dmabuf pseudo-fs, stable for the buffer's lifetime and
/// distinct across distinct buffers, so it disambiguates fd reuse. Two fds that
/// dup the same buffer share the inode — exactly the "reuse the import" case.
type Key = (u64, u32, u32);

fn key_of(dmabuf: &Dmabuf) -> Key {
    let size = dmabuf.size();
    let ino = dmabuf
        .handles()
        .next()
        .and_then(|f| inode_of(f.as_raw_fd()))
        // No fd (shouldn't happen) or fstat failed: fall back to a value that
        // never matches a real inode, forcing a re-import rather than risking a
        // stale-cache alias.
        .unwrap_or(u64::MAX);
    (ino, size.w as u32, size.h as u32)
}

/// The dma-buf's inode via `fstat`. `None` on failure (treated as "no stable
/// identity" by [`key_of`], forcing a fresh import).
fn inode_of(fd: i32) -> Option<u64> {
    // SAFETY: `fstat` only reads metadata for `fd`; the zeroed `stat` is fully
    // written by the call on success. `fd` is a live borrowed dma-buf fd.
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        if libc::fstat(fd, &mut st) == 0 {
            Some(st.st_ino as u64)
        } else {
            None
        }
    }
}

/// A capture target: the destination dmabuf plus an optional source sub-rect
/// (physical/output pixels) to copy from the composed scene. `None` copies the
/// whole scene.
pub type CaptureTarget = (Dmabuf, Option<Rectangle<i32, Physical>>);

/// Caches the capture-target dmabufs imported as TRANSFER_DST images, freeing
/// and re-importing whenever the target set changes (the leak fix).
#[derive(Default)]
pub struct CaptureCache {
    keys: Vec<Key>,
    imports: Vec<CaptureImport>,
    last_warn: Option<Instant>,
}

impl CaptureCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Synchronise the cache to `targets`. Returns `true` if the set changed and
    /// the images were (re)imported this call (so the caller transitions them
    /// UNDEFINED→GENERAL); `false` if the cached images were reused (already
    /// GENERAL). On a change, the previous images are destroyed first.
    fn sync(&mut self, dev: &VulkanDevice, targets: &[CaptureTarget]) -> Result<bool, VulkanError> {
        let new_keys: Vec<Key> = targets.iter().map(|(d, _)| key_of(d)).collect();
        if new_keys == self.keys && self.imports.len() == targets.len() {
            return Ok(false);
        }
        // The set changed — free the previous images, then import the new set.
        self.free(dev);
        for (dmabuf, _src) in targets {
            let (image, memory, _view, _fmt, _w, _h) =
                compositor_kernel_vulkan_memory_target_base::target::import_target(
                    dev,
                    dmabuf,
                    vk::ImageUsageFlags::TRANSFER_DST,
                    false,
                )?;
            self.imports.push(CaptureImport { image, memory });
        }
        self.keys = new_keys;
        Ok(true)
    }

    fn free(&mut self, dev: &VulkanDevice) {
        unsafe {
            for c in self.imports.drain(..) {
                dev.device.destroy_image(c.image, None);
                dev.device.free_memory(c.memory, None);
            }
        }
        self.keys.clear();
    }

    /// Destroy all cached images (call from the renderer's `Drop`, after
    /// `device_wait_idle`).
    pub fn destroy(&mut self, dev: &VulkanDevice) {
        self.free(dev);
    }

    /// Throttled (once/min) capture-failure warning.
    fn warn(&mut self, msg: String) {
        if self.last_warn.is_none_or(|t| t.elapsed().as_secs() >= 60) {
            warn!("post-scene capture copy failed ({msg}); lock snapshot may be blank (throttled: once/min)");
            self.last_warn = Some(Instant::now());
        }
    }
}

/// Copy the just-composed scene `src_image` (left in GENERAL layout by
/// `record_composition`, GPU work already drained on the synchronous path) into
/// each capture-target dmabuf via `vkCmdBlitImage`. Failures are logged
/// (throttled) and non-fatal — a missed capture only blanks the lock-screen
/// snapshot, never the display.
pub fn blit_into_targets(
    dev: &VulkanDevice,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    cache: &mut CaptureCache,
    src_image: vk::Image,
    extent: (u32, u32),
    targets: &[CaptureTarget],
) {
    if targets.is_empty() {
        return;
    }
    let fresh = match cache.sync(dev, targets) {
        Ok(f) => f,
        Err(e) => {
            cache.warn(format!("import: {e}"));
            return;
        }
    };
    // Pair each imported destination image with its source sub-rect (region
    // capture) or the full scene extent (full capture), and its own
    // destination size (the entry dmabuf's size).
    let dsts: Vec<DstBlit> = cache
        .imports
        .iter()
        .zip(targets.iter())
        .map(|(c, (dmabuf, src))| {
            let dst_size = dmabuf.size();
            let (sx, sy, sw, sh) = match src {
                Some(r) => (r.loc.x, r.loc.y, r.size.w, r.size.h),
                None => (0, 0, extent.0 as i32, extent.1 as i32),
            };
            DstBlit {
                image: c.image,
                fresh,
                src_offset: (sx, sy),
                src_end: (sx + sw, sy + sh),
                dst_end: (dst_size.w, dst_size.h),
            }
        })
        .collect();
    if dsts.is_empty() {
        return;
    }
    if let Err(e) = record_capture_blits(dev, command_pool, queue, src_image, &dsts) {
        cache.warn(format!("blit: {e}"));
    }
}

/// Per-destination blit geometry: the imported target image, whether it was
/// freshly imported this frame (UNDEFINED→GENERAL), the source sub-rect within
/// the composed scene, and the destination extent.
struct DstBlit {
    image: vk::Image,
    fresh: bool,
    src_offset: (i32, i32),
    src_end: (i32, i32),
    dst_end: (i32, i32),
}

/// One command buffer: transition fresh targets to GENERAL, blit the scene into
/// each, then a release barrier for the external (GLES/wgpu) reader. Submitted
/// synchronously (the capture entry dmabuf is read by bevy next frame, so we
/// drain before returning).
fn record_capture_blits(
    dev: &VulkanDevice,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    src_image: vk::Image,
    dsts: &[DstBlit],
) -> Result<(), VulkanError> {
    let device = &dev.device;
    let info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let cmd = unsafe { device.allocate_command_buffers(&info)? }[0];
    let sub = vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };
    let layers = vk::ImageSubresourceLayers {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
    };
    unsafe {
        device.begin_command_buffer(
            cmd,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )?;

        // Acquire barriers: each destination to GENERAL (TRANSFER_DST is valid in
        // GENERAL). Fresh imports start UNDEFINED; cached ones are already GENERAL
        // but still need a WAR/WAW dependency before we write.
        let acquires: Vec<vk::ImageMemoryBarrier2> = dsts
            .iter()
            .map(|d| {
                vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                    .dst_stage_mask(vk::PipelineStageFlags2::ALL_TRANSFER)
                    .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                    .old_layout(if d.fresh {
                        vk::ImageLayout::UNDEFINED
                    } else {
                        vk::ImageLayout::GENERAL
                    })
                    .new_layout(vk::ImageLayout::GENERAL)
                    .image(d.image)
                    .subresource_range(sub)
            })
            .collect();
        device.cmd_pipeline_barrier2(
            cmd,
            &vk::DependencyInfo::default().image_memory_barriers(&acquires),
        );

        // Each destination blits its own source sub-rect (region capture) or the
        // full scene (full capture) into its own extent — `vkCmdBlitImage`
        // scales src→dst as needed.
        for d in dsts {
            let region = vk::ImageBlit {
                src_subresource: layers,
                src_offsets: [
                    vk::Offset3D {
                        x: d.src_offset.0,
                        y: d.src_offset.1,
                        z: 0,
                    },
                    vk::Offset3D {
                        x: d.src_end.0,
                        y: d.src_end.1,
                        z: 1,
                    },
                ],
                dst_subresource: layers,
                dst_offsets: [
                    vk::Offset3D { x: 0, y: 0, z: 0 },
                    vk::Offset3D {
                        x: d.dst_end.0,
                        y: d.dst_end.1,
                        z: 1,
                    },
                ],
            };
            device.cmd_blit_image(
                cmd,
                src_image,
                vk::ImageLayout::GENERAL,
                d.image,
                vk::ImageLayout::GENERAL,
                &[region],
                vk::Filter::LINEAR,
            );
        }

        // Release barrier: make the blit writes available to the external
        // (GLES/wgpu) reader of the same dmabuf memory.
        let releases: Vec<vk::ImageMemoryBarrier2> = dsts
            .iter()
            .map(|d| {
                vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::ALL_TRANSFER)
                    .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                    .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                    .dst_access_mask(vk::AccessFlags2::MEMORY_READ)
                    .old_layout(vk::ImageLayout::GENERAL)
                    .new_layout(vk::ImageLayout::GENERAL)
                    .image(d.image)
                    .subresource_range(sub)
            })
            .collect();
        device.cmd_pipeline_barrier2(
            cmd,
            &vk::DependencyInfo::default().image_memory_barriers(&releases),
        );

        device.end_command_buffer(cmd)?;
        let cmd_info = vk::CommandBufferSubmitInfo::default().command_buffer(cmd);
        let submit = vk::SubmitInfo2::default().command_buffer_infos(std::slice::from_ref(&cmd_info));
        device
            .queue_submit2(queue, &[submit], vk::Fence::null())
            .map_err(|e| VulkanError::Vk(format!("capture submit: {e}")))?;
        device.device_wait_idle()?;
        device.free_command_buffers(command_pool, &[cmd]);
    }
    Ok(())
}
