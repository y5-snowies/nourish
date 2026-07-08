//! Per-surface mip-chain generation for the world anti-aliasing trilinear / anisotropic
//! modes.
//!
//! Client & iced buffers are single-mip, `SAMPLED`-only imports, so we cannot
//! `vkCmdBlitImage` FROM them (that needs TRANSFER_SRC) and cannot add transfer
//! usage to the dmabuf import without risking import failure per-modifier.
//! Instead each trilinear/aniso surface is RENDERED (sampled through the AA
//! pipeline) into an owned mip-0, then the chain is generated with blits, and
//! the AA composite samples that mipped copy with the trilinear/aniso sampler.
//!
//! Damage is handled upstream by smithay: undamaged elements aren't redrawn, so
//! they never reach `render_texture_from_to` and their mips aren't regenerated
//! (on the partial-damage path). The round-robin scratch images are reused
//! across frames; `MAX_SURFACES` bounds mip VRAM (excess surfaces fall back to
//! the plain path for the frame).

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use compositor_kernel_vulkan_pipeline_composite_base::composite::{AaComposite, AaPush};

/// One owned, mipped copy of a source surface.
pub struct MipImage {
    image: vk::Image,
    memory: vk::DeviceMemory,
    /// View over all mip levels — sampled by the composite.
    pub view: vk::ImageView,
    /// Level-0-only view — the color attachment we render the source into.
    mip0_view: vk::ImageView,
    width: u32,
    height: u32,
    mip_levels: u32,
    format: vk::Format,
}

impl MipImage {
    fn destroy(&self, dev: &VulkanDevice) {
        unsafe {
            dev.device.destroy_image_view(self.view, None);
            dev.device.destroy_image_view(self.mip0_view, None);
            dev.device.destroy_image(self.image, None);
            dev.device.free_memory(self.memory, None);
        }
    }
}

/// A pool of reusable mipped images, indexed round-robin per frame.
#[derive(Default)]
pub struct MipGen {
    images: Vec<MipImage>,
    next: usize,
}

fn mip_count(w: u32, h: u32) -> u32 {
    // floor(log2(max(w,h))) + 1
    32 - w.max(h).max(1).leading_zeros()
}

fn device_local(mem: &vk::PhysicalDeviceMemoryProperties, bits: u32) -> Option<u32> {
    (0..mem.memory_type_count).find(|&i| {
        bits & (1 << i) != 0
            && mem.memory_types[i as usize]
                .property_flags
                .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
    })
}

fn subrange(base: u32, count: u32) -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: base,
        level_count: count,
        base_array_layer: 0,
        layer_count: 1,
    }
}

#[allow(clippy::too_many_arguments)]
fn barrier(
    dev: &VulkanDevice,
    cmd: vk::CommandBuffer,
    image: vk::Image,
    range: vk::ImageSubresourceRange,
    old: vk::ImageLayout,
    new: vk::ImageLayout,
    src_stage: vk::PipelineStageFlags2,
    dst_stage: vk::PipelineStageFlags2,
    src_access: vk::AccessFlags2,
    dst_access: vk::AccessFlags2,
) {
    let b = vk::ImageMemoryBarrier2::default()
        .src_stage_mask(src_stage)
        .dst_stage_mask(dst_stage)
        .src_access_mask(src_access)
        .dst_access_mask(dst_access)
        .old_layout(old)
        .new_layout(new)
        .image(image)
        .subresource_range(range);
    unsafe {
        dev.device.cmd_pipeline_barrier2(
            cmd,
            &vk::DependencyInfo::default().image_memory_barriers(std::slice::from_ref(&b)),
        );
    }
}

impl MipGen {
    /// Cap on per-frame mipped surfaces — bounds mip VRAM. Surfaces beyond this
    /// fall back to the plain composite path (no AA) for the frame.
    const MAX_SURFACES: usize = 32;

    /// Reset the round-robin index — call once per frame before `acquire`.
    pub fn begin_frame(&mut self) {
        self.next = 0;
    }

    /// No mip images currently allocated.
    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }

    /// Get (creating/reusing) the next mipped image sized to `(w,h)` in `format`.
    /// Returns its index for `record`/`view_of`, or `None` when the per-frame
    /// surface cap is hit (the caller then falls back to the plain path for that
    /// surface — a bound on mip VRAM regardless of how many world surfaces are
    /// on screen).
    pub fn acquire(
        &mut self,
        dev: &VulkanDevice,
        mem: &vk::PhysicalDeviceMemoryProperties,
        format: vk::Format,
        w: u32,
        h: u32,
    ) -> Result<Option<usize>, vk::Result> {
        if self.next >= Self::MAX_SURFACES {
            return Ok(None);
        }
        let (w, h) = (w.max(1), h.max(1));
        let idx = self.next;
        self.next += 1;
        let stale = match self.images.get(idx) {
            Some(m) => m.width != w || m.height != h || m.format != format,
            None => true,
        };
        if stale {
            let fresh = Self::create(dev, mem, format, w, h)?;
            if idx < self.images.len() {
                let old = std::mem::replace(&mut self.images[idx], fresh);
                old.destroy(dev);
            } else {
                self.images.push(fresh);
            }
        }
        Ok(Some(idx))
    }

    pub fn view_of(&self, idx: usize) -> vk::ImageView {
        self.images[idx].view
    }

    fn create(
        dev: &VulkanDevice,
        mem: &vk::PhysicalDeviceMemoryProperties,
        format: vk::Format,
        w: u32,
        h: u32,
    ) -> Result<MipImage, vk::Result> {
        let device = &dev.device;
        let levels = mip_count(w, h);
        let image = unsafe {
            device.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(format)
                    .extent(vk::Extent3D {
                        width: w,
                        height: h,
                        depth: 1,
                    })
                    .mip_levels(levels)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(
                        vk::ImageUsageFlags::COLOR_ATTACHMENT
                            | vk::ImageUsageFlags::SAMPLED
                            | vk::ImageUsageFlags::TRANSFER_SRC
                            | vk::ImageUsageFlags::TRANSFER_DST,
                    )
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .initial_layout(vk::ImageLayout::UNDEFINED),
                None,
            )?
        };
        let req = unsafe { device.get_image_memory_requirements(image) };
        let mem_idx = device_local(mem, req.memory_type_bits).ok_or(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY)?;
        let memory = unsafe {
            device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(mem_idx),
                None,
            )?
        };
        unsafe { device.bind_image_memory(image, memory, 0)? };
        let mk_view = |base, count| unsafe {
            device.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format)
                    .subresource_range(subrange(base, count)),
                None,
            )
        };
        let view = mk_view(0, levels)?;
        let mip0_view = mk_view(0, 1)?;
        Ok(MipImage {
            image,
            memory,
            view,
            mip0_view,
            width: w,
            height: h,
            mip_levels: levels,
            format,
        })
    }

    /// Record (into `cmd`, OUTSIDE any render pass): fill mip-0 by drawing the
    /// source (bound in `fill_set`) through the AA pipeline, then blit the chain
    /// down. Leaves every level in SHADER_READ_ONLY_OPTIMAL for the composite.
    pub fn record(
        &self,
        dev: &VulkanDevice,
        cmd: vk::CommandBuffer,
        aa: &AaComposite,
        fill_set: vk::DescriptorSet,
        idx: usize,
    ) {
        let m = &self.images[idx];
        let device = &dev.device;

        // mip0 → COLOR_ATTACHMENT, then render the (full) source into it.
        barrier(
            dev, cmd, m.image, subrange(0, 1),
            vk::ImageLayout::UNDEFINED, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::PipelineStageFlags2::TOP_OF_PIPE, vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
            vk::AccessFlags2::empty(), vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
        );
        let attach = vk::RenderingAttachmentInfo::default()
            .image_view(m.mip0_view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(vk::ClearValue {
                color: vk::ClearColorValue { float32: [0.0; 4] },
            });
        let area = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D { width: m.width, height: m.height },
        };
        let rendering = vk::RenderingInfo::default()
            .render_area(area)
            .layer_count(1)
            .color_attachments(std::slice::from_ref(&attach));
        unsafe {
            device.cmd_begin_rendering(cmd, &rendering);
            device.cmd_set_viewport(cmd, 0, &[vk::Viewport {
                x: 0.0, y: 0.0, width: m.width as f32, height: m.height as f32,
                min_depth: 0.0, max_depth: 1.0,
            }]);
            device.cmd_set_scissor(cmd, 0, &[area]);
        }
        // Full-screen copy: dst = whole NDC quad, src = whole texture, taps=1,
        // classic mode (params2.x = 0) — a plain bilinear blit into mip 0.
        aa.draw(dev, cmd, fill_set, AaPush {
            dst: [-1.0, -1.0, 2.0, 2.0],
            src: [0.0, 0.0, 1.0, 1.0],
            color: [1.0, 1.0, 1.0, 1.0],
            params: [1.0, 1.0, 0.0, 0.0],
            params2: [0.0, 0.0, 0.0, 0.0],
        });
        unsafe { device.cmd_end_rendering(cmd) };

        // mip0 → TRANSFER_SRC; mips 1.. → TRANSFER_DST.
        barrier(
            dev, cmd, m.image, subrange(0, 1),
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL, vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT, vk::PipelineStageFlags2::ALL_TRANSFER,
            vk::AccessFlags2::COLOR_ATTACHMENT_WRITE, vk::AccessFlags2::TRANSFER_READ,
        );
        if m.mip_levels > 1 {
            barrier(
                dev, cmd, m.image, subrange(1, m.mip_levels - 1),
                vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::PipelineStageFlags2::TOP_OF_PIPE, vk::PipelineStageFlags2::ALL_TRANSFER,
                vk::AccessFlags2::empty(), vk::AccessFlags2::TRANSFER_WRITE,
            );
        }

        // Blit each level from the one above, halving the extent (min 1).
        let (mut sw, mut sh) = (m.width as i32, m.height as i32);
        for level in 1..m.mip_levels {
            let (dw, dh) = ((sw / 2).max(1), (sh / 2).max(1));
            let layers = |mip| vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: mip,
                base_array_layer: 0,
                layer_count: 1,
            };
            let region = vk::ImageBlit {
                src_subresource: layers(level - 1),
                src_offsets: [
                    vk::Offset3D { x: 0, y: 0, z: 0 },
                    vk::Offset3D { x: sw, y: sh, z: 1 },
                ],
                dst_subresource: layers(level),
                dst_offsets: [
                    vk::Offset3D { x: 0, y: 0, z: 0 },
                    vk::Offset3D { x: dw, y: dh, z: 1 },
                ],
            };
            unsafe {
                device.cmd_blit_image(
                    cmd,
                    m.image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    m.image, vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    std::slice::from_ref(&region),
                    vk::Filter::LINEAR,
                );
            }
            // This level becomes the source for the next blit.
            barrier(
                dev, cmd, m.image, subrange(level, 1),
                vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::PipelineStageFlags2::ALL_TRANSFER, vk::PipelineStageFlags2::ALL_TRANSFER,
                vk::AccessFlags2::TRANSFER_WRITE, vk::AccessFlags2::TRANSFER_READ,
            );
            sw = dw;
            sh = dh;
        }

        // All levels are TRANSFER_SRC now → hand them to the fragment shader.
        barrier(
            dev, cmd, m.image, subrange(0, m.mip_levels),
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::PipelineStageFlags2::ALL_TRANSFER, vk::PipelineStageFlags2::FRAGMENT_SHADER,
            vk::AccessFlags2::TRANSFER_READ, vk::AccessFlags2::SHADER_SAMPLED_READ,
        );
    }

    pub fn destroy(&mut self, dev: &VulkanDevice) {
        for m in self.images.drain(..) {
            m.destroy(dev);
        }
    }
}
