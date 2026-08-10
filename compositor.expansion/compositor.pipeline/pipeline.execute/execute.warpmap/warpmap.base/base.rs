//! The per-frame, GPU-produced pointer warp map.
//!
//! A warp that ANIMATES cannot be served by a static bake — the bake describes one
//! instant and is wrong every frame after it. Re-baking on the CPU is 16k
//! evaluations per frame, about a millisecond of a sixteen. So the GPU renders the
//! grid as part of the frame, where 16k fragments is nothing, and the result is
//! copied to host memory for the pointer to sample.
//!
//! # Why the read is a frame behind
//!
//! Reading back the image the GPU is still writing means waiting on a fence in the
//! middle of the frame — a stall on the render thread, every frame, to serve a
//! cursor. Instead the copy is recorded into the SAME command buffer as the frame,
//! and the bytes are read at the renderer's existing drain point, where the
//! previous frame is already proven complete. The pointer therefore samples a map
//! one frame old, which at 60 Hz is well inside the latency the pointer already
//! has, and nothing ever blocks.
//!
//! Everything here is owned and nothing is allocated unless a bundle asks for it,
//! so a compositor with no such bundle pays a `None` check per frame.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use compositor_pipeline_execute_effect_base::effect::EffectPass;
use compositor_kernel_vulkan_renderer_error_base::VulkanError;

/// Cells per edge. Must match `shader.map::EDGE` — the consumer indexes with it.
pub const EDGE: u32 = 128;

/// Two `f32` per cell: the source UV the pointer is looking for.
const CELL: u64 = 8;
const FORMAT: vk::Format = vk::Format::R32G32_SFLOAT;

pub struct WarpMap {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    buf: vk::Buffer,
    buf_mem: vk::DeviceMemory,
    pass: EffectPass,
    /// The pipeline id this was built for; a different bundle rebuilds it.
    id: u64,
    /// A copy was recorded into the frame now in flight, so the staging buffer is
    /// worth reading at the next drain.
    pending: bool,
}

fn memory_type(
    mem: &vk::PhysicalDeviceMemoryProperties,
    bits: u32,
    want: vk::MemoryPropertyFlags,
) -> Option<u32> {
    (0..mem.memory_type_count).find(|&i| {
        bits & (1 << i) != 0 && mem.memory_types[i as usize].property_flags.contains(want)
    })
}

fn range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

impl WarpMap {
    /// Build for one bundle's warp shader. `spv` is a fullscreen pass writing the
    /// source UV to `@location(0)`; the engine composes it from the bundle's own
    /// warp module, so this never learns anything shader-specific.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        dev: &VulkanDevice,
        mem: &vk::PhysicalDeviceMemoryProperties,
        cache: vk::PipelineCache,
        id: u64,
        spv: &[u8],
        vert_spv: Option<&[u8]>,
        vert_entry: &str,
        frag_entry: &str,
        push_size: u32,
    ) -> Result<Self, VulkanError> {
        let d = &dev.device;
        let image = unsafe {
            d.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(FORMAT)
                    .extent(vk::Extent3D { width: EDGE, height: EDGE, depth: 1 })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(
                        vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
                    )
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .initial_layout(vk::ImageLayout::UNDEFINED),
                None,
            )
            .map_err(|e| VulkanError::Vk(format!("warp map image: {e}")))?
        };
        let req = unsafe { d.get_image_memory_requirements(image) };
        let idx = memory_type(mem, req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)
            .ok_or_else(|| VulkanError::Vk("warp map: no device-local memory".into()))?;
        let memory = unsafe {
            d.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(idx),
                None,
            )
            .map_err(|e| VulkanError::Vk(format!("warp map memory: {e}")))?
        };
        unsafe { d.bind_image_memory(image, memory, 0) }
            .map_err(|e| VulkanError::Vk(format!("warp map bind: {e}")))?;
        let view = unsafe {
            d.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(FORMAT)
                    .subresource_range(range()),
                None,
            )
            .map_err(|e| VulkanError::Vk(format!("warp map view: {e}")))?
        };

        // Host-visible + COHERENT, so the read needs no explicit invalidate — the
        // buffer is tiny and written once per frame, so the coherent path costs
        // nothing worth managing a range for.
        let size = EDGE as u64 * EDGE as u64 * CELL;
        let buf = unsafe {
            d.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(vk::BufferUsageFlags::TRANSFER_DST)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
            .map_err(|e| VulkanError::Vk(format!("warp map buffer: {e}")))?
        };
        let breq = unsafe { d.get_buffer_memory_requirements(buf) };
        let bidx = memory_type(
            mem,
            breq.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )
        .ok_or_else(|| VulkanError::Vk("warp map: no host-visible memory".into()))?;
        let buf_mem = unsafe {
            d.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(breq.size)
                    .memory_type_index(bidx),
                None,
            )
            .map_err(|e| VulkanError::Vk(format!("warp map buffer memory: {e}")))?
        };
        unsafe { d.bind_buffer_memory(buf, buf_mem, 0) }
            .map_err(|e| VulkanError::Vk(format!("warp map buffer bind: {e}")))?;

        let pass = EffectPass::create(
            dev, cache, FORMAT, 0, spv, vert_spv, vert_entry, frag_entry, push_size, None, None,
        )?;
        Ok(Self { image, memory, view, buf, buf_mem, pass, id, pending: false })
    }

    /// Build, keep or drop the producer to match what `want` asks for.
    ///
    /// ONE implementation for both devices. The compositor runs it when a pipeline
    /// op reaches `submit_frame`; the worker runs it when the same graph executes
    /// on its own device — and a fully-offloaded bundle only ever reaches the
    /// second, which is exactly the case a compositor-only version missed.
    ///
    /// Returns whether the caller's held grid is now INVALID — the producer
    /// changed or went — because a grid outliving its bundle displaces the pointer
    /// by an effect that is no longer on screen.
    pub fn sync(
        dev: &VulkanDevice,
        mem: &vk::PhysicalDeviceMemoryProperties,
        cache: vk::PipelineCache,
        want: Option<&compositor_pipeline_execute_graph_base::graph::GraphPass>,
        slot: &mut Option<WarpMap>,
        where_: &str,
    ) -> Result<bool, VulkanError> {
        let mut invalidated = false;
        match want {
            // Already the right one.
            Some(g) if slot.as_ref().is_some_and(|w| w.id == g.id) => {}
            Some(g) => {
                if let Some(w) = slot.take() {
                    w.destroy(dev);
                }
                invalidated = true;
                info!("warp map: building the {where_} GPU producer for pipeline {:#x}", g.id);
                *slot = Some(Self::create(
                    dev,
                    mem,
                    cache,
                    g.id,
                    &g.spv,
                    g.vert_spv.as_deref(),
                    &g.vert_entry,
                    &g.frag_entry,
                    g.push.len() as u32,
                )?);
            }
            None => {
                if let Some(w) = slot.take() {
                    w.destroy(dev);
                    invalidated = true;
                }
            }
        }
        Ok(invalidated)
    }

    /// Read the grid recorded into the frame that has just been proven complete.
    /// Call at the caller's completion point — see [`Self::take`].
    ///
    /// RETURNED, not published. There are two producers of a warp grid — the
    /// compositor's own map and the worker's, for a fully-offloaded bundle — and
    /// they used to write one process-global slot, racing for it with no way to
    /// tell whose grid was in it. Each caller now puts its own answer where its
    /// own reader will look for it.
    pub fn publish(
        slot: &mut Option<WarpMap>,
        dev: &VulkanDevice,
    ) -> Option<std::sync::Arc<Vec<[f32; 2]>>> {
        slot.as_mut().and_then(|w| w.take(dev)).map(std::sync::Arc::new)
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    /// Record this frame's map: render the grid, then copy it to host memory.
    ///
    /// Both go into the frame's own command buffer, so nothing waits. The bytes
    /// become readable when that frame completes — see [`Self::take`].
    pub fn record(&mut self, dev: &VulkanDevice, cmd: vk::CommandBuffer, push: &[u8]) {
        let d = &dev.device;
        self.transition(
            d,
            cmd,
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::PipelineStageFlags2::TOP_OF_PIPE,
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
            vk::AccessFlags2::empty(),
            vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
        );
        let attach = vk::RenderingAttachmentInfo::default()
            .image_view(self.view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::DONT_CARE)
            .store_op(vk::AttachmentStoreOp::STORE);
        let area = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D { width: EDGE, height: EDGE },
        };
        unsafe {
            d.cmd_begin_rendering(
                cmd,
                &vk::RenderingInfo::default()
                    .render_area(area)
                    .layer_count(1)
                    .color_attachments(std::slice::from_ref(&attach)),
            );
            d.cmd_set_viewport(cmd, 0, &[vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: EDGE as f32,
                height: EDGE as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            }]);
            d.cmd_set_scissor(cmd, 0, &[area]);
        }
        self.pass.draw(dev, cmd, None, None, None, push);
        unsafe { d.cmd_end_rendering(cmd) };
        self.transition(
            d,
            cmd,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags2::COPY,
            vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
            vk::AccessFlags2::TRANSFER_READ,
        );
        let region = vk::BufferImageCopy::default()
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_extent(vk::Extent3D { width: EDGE, height: EDGE, depth: 1 });
        unsafe {
            d.cmd_copy_image_to_buffer(
                cmd,
                self.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                self.buf,
                std::slice::from_ref(&region),
            );
        }
        // Make the copy AVAILABLE to the host.
        //
        // The memory is `HOST_COHERENT` and the read happens after a fence, which
        // is why this works in practice — but coherency covers visibility, not
        // availability, and the spec wants the transfer write released to the host
        // domain explicitly. One barrier on a 128 KB buffer once a frame is not
        // worth relying on driver leniency for.
        let bb = vk::BufferMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COPY)
            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::HOST)
            .dst_access_mask(vk::AccessFlags2::HOST_READ)
            .buffer(self.buf)
            .offset(0)
            .size(vk::WHOLE_SIZE);
        unsafe {
            d.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().buffer_memory_barriers(std::slice::from_ref(&bb)),
            );
        }
        self.pending = true;
    }

    /// Read the map recorded into the frame that has just been proven complete.
    ///
    /// Called at the renderer's drain point, which is the whole reason this never
    /// blocks: completion is already established there, so mapping the buffer is a
    /// memcpy and not a wait. `None` until a frame has actually recorded one.
    pub fn take(&mut self, dev: &VulkanDevice) -> Option<Vec<[f32; 2]>> {
        if !self.pending {
            return None;
        }
        self.pending = false;
        let n = (EDGE * EDGE) as usize;
        let size = n as u64 * CELL;
        let ptr = unsafe {
            dev.device.map_memory(self.buf_mem, 0, size, vk::MemoryMapFlags::empty()).ok()?
        } as *const [f32; 2];
        let mut cells = vec![[0.0f32; 2]; n];
        unsafe {
            std::ptr::copy_nonoverlapping(ptr, cells.as_mut_ptr(), n);
            dev.device.unmap_memory(self.buf_mem);
        }
        Some(cells)
    }

    #[allow(clippy::too_many_arguments)]
    fn transition(
        &self,
        d: &ash::Device,
        cmd: vk::CommandBuffer,
        old: vk::ImageLayout,
        new: vk::ImageLayout,
        ss: vk::PipelineStageFlags2,
        ds: vk::PipelineStageFlags2,
        sa: vk::AccessFlags2,
        da: vk::AccessFlags2,
    ) {
        let b = vk::ImageMemoryBarrier2::default()
            .src_stage_mask(ss)
            .dst_stage_mask(ds)
            .src_access_mask(sa)
            .dst_access_mask(da)
            .old_layout(old)
            .new_layout(new)
            .image(self.image)
            .subresource_range(range());
        unsafe {
            d.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(std::slice::from_ref(&b)),
            );
        }
    }

    pub fn destroy(&self, dev: &VulkanDevice) {
        let d = &dev.device;
        self.pass.destroy(dev);
        unsafe {
            d.destroy_image_view(self.view, None);
            d.destroy_image(self.image, None);
            d.free_memory(self.memory, None);
            d.destroy_buffer(self.buf, None);
            d.free_memory(self.buf_mem, None);
        }
    }
}
