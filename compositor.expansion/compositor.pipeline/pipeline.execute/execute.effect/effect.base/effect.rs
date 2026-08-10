//! A fullscreen-triangle shader pass that samples N input textures — the
//! descriptor-carrying sibling of `pipeline.fullscreen::FullscreenPass` (which
//! has no descriptors and cannot sample). One shared bilinear sampler at
//! `@group(0) @binding(0)`, then `input_count` sampled images at bindings `1..`,
//! matching the multipass pass ABI (see `document/SHADER_PIPELINE.md`). Modeled
//! on `pipeline.composite::AaComposite`'s separate image+sampler descriptors.
//!
//! `input_count == 0` degenerates to a push-only pass (no descriptor set),
//! identical in effect to `FullscreenPass`, so the graph executor can drive
//! every pass through this one type.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use compositor_kernel_vulkan_renderer_error_base::VulkanError;

/// A built effect pipeline for one color format + shader module + input arity.
pub struct EffectPass {
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    /// `null` when `input_count == 0` (push-only pass).
    set_layout: vk::DescriptorSetLayout,
    /// `null` when `input_count == 0`.
    sampler: vk::Sampler,
    /// `null` when `input_count == 0`; per-frame reset otherwise.
    pool: vk::DescriptorPool,
    input_count: u32,
    push_stages: vk::ShaderStageFlags,
    /// True when the pipeline layout has a `@group(1)` window-rects UBO set.
    has_window_ubo: bool,
    /// True when the layout has a `@group(2)` storage set.
    has_storage: bool,
    /// A zero-binding layout standing in for `@group(1)` when a pass has storage
    /// but no window blocks.
    ///
    /// Set indices are POSITIONS in the layout array, so a gap is not expressible:
    /// omitting set 1 would silently move storage to `@group(1)` and the shader
    /// would read its buffers where the window UBO is meant to be. The same bug
    /// this file already records having hit once with set 0. `null` when unused.
    empty_layout: vk::DescriptorSetLayout,
    pub color_format: vk::Format,
}

fn shader_module(dev: &ash::Device, spv: &[u8]) -> Result<vk::ShaderModule, VulkanError> {
    let words: Vec<u32> = spv
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let info = vk::ShaderModuleCreateInfo::default().code(&words);
    unsafe {
        dev.create_shader_module(&info, None)
            .map_err(|e| VulkanError::Vk(format!("effect shader module: {e}")))
    }
}

impl EffectPass {
    /// Build the pipeline. `spv` holds the fragment stage (and vertex, unless
    /// `vert_spv` is set — a fragment-only composed pass paired with the
    /// fullscreen vertex). `input_count` sampled images bind at `1..=input_count`;
    /// `push_size` is the push-constant range (visible to both stages).
    /// `window_ubo` is the caller-owned `@group(1)` UBO set layout to append (the
    /// window-rects buffer); requires `input_count > 0` so `@group(0)` occupies
    /// set 0.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        device: &VulkanDevice,
        cache: vk::PipelineCache,
        color_format: vk::Format,
        input_count: u32,
        spv: &[u8],
        vert_spv: Option<&[u8]>,
        vert_entry: &str,
        frag_entry: &str,
        push_size: u32,
        window_ubo: Option<vk::DescriptorSetLayout>,
        storage: Option<vk::DescriptorSetLayout>,
    ) -> Result<Self, VulkanError> {
        let dev = &device.device;
        let push_stages = vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT;

        // Descriptor set layout: sampler at binding 0, one sampled image per
        // input at bindings 1..=input_count. Skipped entirely for push-only.
        //
        // Set 0 must also exist for a pass with NO inputs that binds the window
        // set: `draw` binds the window set at firstSet=1, and the shader reads its
        // sampler from `@group(0)`. Building the layout without set 0 put the
        // window UBO at index 0 and made that bind invalid — a GPU fault with no
        // diagnostic in release, where the old `debug_assert` was compiled out.
        let need_set0 = input_count > 0 || window_ubo.is_some() || storage.is_some();
        let (set_layout, sampler, pool) = if need_set0 {
            let mut bindings = vec![vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)];
            for i in 0..input_count {
                bindings.push(
                    vk::DescriptorSetLayoutBinding::default()
                        .binding(1 + i)
                        .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                        .descriptor_count(1)
                        .stage_flags(vk::ShaderStageFlags::FRAGMENT),
                );
            }
            let set_layout = unsafe {
                dev.create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )
                .map_err(|e| VulkanError::Vk(format!("effect set layout: {e}")))?
            };
            let sampler = unsafe {
                dev.create_sampler(
                    &vk::SamplerCreateInfo::default()
                        .mag_filter(vk::Filter::LINEAR)
                        .min_filter(vk::Filter::LINEAR)
                        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                    None,
                )
                .map_err(|e| VulkanError::Vk(format!("effect sampler: {e}")))?
            };
            // Enough sets for a deep graph re-recorded each frame; reset per frame.
            const MAX_SETS: u32 = 64;
            // `descriptorCount` must be > 0, so the image size is omitted when the
            // pass has no inputs (sampler-only set 0 for a window-texture pass).
            let mut pool_sizes = vec![vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLER)
                .descriptor_count(MAX_SETS)];
            if input_count > 0 {
                pool_sizes.push(
                    vk::DescriptorPoolSize::default()
                        .ty(vk::DescriptorType::SAMPLED_IMAGE)
                        .descriptor_count(MAX_SETS * input_count),
                );
            }
            let pool = unsafe {
                dev.create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .max_sets(MAX_SETS)
                        .pool_sizes(&pool_sizes),
                    None,
                )
                .map_err(|e| VulkanError::Vk(format!("effect pool: {e}")))?
            };
            (set_layout, sampler, pool)
        } else {
            (vk::DescriptorSetLayout::null(), vk::Sampler::null(), vk::DescriptorPool::null())
        };

        let push_range = vk::PushConstantRange::default()
            .stage_flags(push_stages)
            .offset(0)
            .size(push_size);
        // set 0 = inputs (sampler + images), set 1 = window blocks, set 2 = storage.
        // Indices are positions in this array, so every set below the highest one
        // in use must be present even when the pass does not read it.
        let mut sets: Vec<vk::DescriptorSetLayout> = Vec::new();
        if need_set0 {
            sets.push(set_layout);
        }
        let mut empty_layout = vk::DescriptorSetLayout::null();
        match window_ubo {
            Some(ubo) => sets.push(ubo),
            None if storage.is_some() => {
                empty_layout = unsafe {
                    dev.create_descriptor_set_layout(
                        &vk::DescriptorSetLayoutCreateInfo::default(),
                        None,
                    )
                    .map_err(|e| VulkanError::Vk(format!("effect empty set layout: {e}")))?
                };
                sets.push(empty_layout);
            }
            None => {}
        }
        if let Some(st) = storage {
            sets.push(st);
        }
        let mut layout_info = vk::PipelineLayoutCreateInfo::default()
            .push_constant_ranges(std::slice::from_ref(&push_range));
        if !sets.is_empty() {
            layout_info = layout_info.set_layouts(&sets);
        }
        let layout = unsafe {
            dev.create_pipeline_layout(&layout_info, None)
                .map_err(|e| VulkanError::Vk(format!("effect pipeline layout: {e}")))?
        };
        let has_window_ubo = window_ubo.is_some();
        let has_storage = storage.is_some();

        let frag_module = shader_module(dev, spv)?;
        let vert_module = match vert_spv {
            Some(vs) => shader_module(dev, vs)?,
            None => frag_module,
        };
        let vs = std::ffi::CString::new(vert_entry).unwrap();
        let fs = std::ffi::CString::new(frag_entry).unwrap();
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vert_module)
                .name(&vs),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(frag_module)
                .name(&fs),
        ];

        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        // Premultiplied-alpha over — same blend as FullscreenPass, so a pass sits
        // correctly over a cleared target (alpha 1 → replace).
        let blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA);
        let blend = vk::PipelineColorBlendStateCreateInfo::default()
            .attachments(std::slice::from_ref(&blend_attachment));
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let color_formats = [color_format];
        let mut rendering_info =
            vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&color_formats);
        let info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .color_blend_state(&blend)
            .dynamic_state(&dynamic)
            .layout(layout)
            .push_next(&mut rendering_info);
        let pipeline = unsafe {
            dev.create_graphics_pipelines(cache, std::slice::from_ref(&info), None)
                .map_err(|(_, e)| VulkanError::Vk(format!("effect graphics pipeline: {e}")))?[0]
        };
        unsafe {
            dev.destroy_shader_module(frag_module, None);
            if vert_module != frag_module {
                dev.destroy_shader_module(vert_module, None);
            }
        }

        Ok(Self {
            pipeline,
            layout,
            set_layout,
            sampler,
            pool,
            input_count,
            push_stages,
            has_window_ubo,
            has_storage,
            empty_layout,
            color_format,
        })
    }

    /// Reset the per-frame descriptor pool. Call once before recording draws
    /// (no-op for a push-only pass).
    pub fn begin_frame(&self, device: &VulkanDevice) {
        if self.pool != vk::DescriptorPool::null() {
            unsafe {
                let _ = device
                    .device
                    .reset_descriptor_pool(self.pool, vk::DescriptorPoolResetFlags::empty());
            }
        }
    }

    /// Allocate + write a descriptor set binding the shared sampler (binding 0)
    /// and one image view per input (bindings `1..`), each expected in
    /// `SHADER_READ_ONLY_OPTIMAL`. `views.len()` must equal `input_count`.
    pub fn input_set(
        &self,
        device: &VulkanDevice,
        views: &[vk::ImageView],
    ) -> Result<vk::DescriptorSet, VulkanError> {
        assert_eq!(views.len() as u32, self.input_count, "effect input arity mismatch");
        let dev = &device.device;
        let set = unsafe {
            dev.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.pool)
                    .set_layouts(std::slice::from_ref(&self.set_layout)),
            )
            .map_err(|e| VulkanError::Vk(format!("effect set: {e}")))?[0]
        };
        let smp = vk::DescriptorImageInfo::default().sampler(self.sampler);
        // The per-view image infos must outlive the update call.
        let imgs: Vec<vk::DescriptorImageInfo> = views
            .iter()
            .map(|v| {
                vk::DescriptorImageInfo::default()
                    .image_view(*v)
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            })
            .collect();
        let mut writes = vec![vk::WriteDescriptorSet::default()
            .dst_set(set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .image_info(std::slice::from_ref(&smp))];
        for (i, img) in imgs.iter().enumerate() {
            writes.push(
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(1 + i as u32)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(std::slice::from_ref(img)),
            );
        }
        unsafe { dev.update_descriptor_sets(&writes, &[]) };
        Ok(set)
    }

    /// Bind and draw the fullscreen triangle. `set` is the input set from
    /// [`Self::input_set`] (`None` for a push-only pass); `window_set` is the
    /// `@group(1)` window-rects UBO set (required iff the pass declared it).
    /// Whether this pass has a set-0 to bind. True for any pass with inputs, and
    /// for an input-less pass that still needs the set-0 sampler (window textures).
    pub fn has_input_set(&self) -> bool {
        self.set_layout != vk::DescriptorSetLayout::null()
    }

    pub fn draw(
        &self,
        device: &VulkanDevice,
        cmd: vk::CommandBuffer,
        set: Option<vk::DescriptorSet>,
        window_set: Option<vk::DescriptorSet>,
        storage_set: Option<vk::DescriptorSet>,
        push: &[u8],
    ) {
        let dev = &device.device;
        unsafe {
            dev.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
            if let Some(set) = set {
                dev.cmd_bind_descriptor_sets(
                    cmd, vk::PipelineBindPoint::GRAPHICS, self.layout, 0, &[set], &[],
                );
            }
            if self.has_window_ubo {
                if let Some(ws) = window_set {
                    dev.cmd_bind_descriptor_sets(
                        cmd, vk::PipelineBindPoint::GRAPHICS, self.layout, 1, &[ws], &[],
                    );
                }
            }
            if self.has_storage {
                if let Some(ss) = storage_set {
                    dev.cmd_bind_descriptor_sets(
                        cmd, vk::PipelineBindPoint::GRAPHICS, self.layout, 2, &[ss], &[],
                    );
                }
            }
            dev.cmd_push_constants(cmd, self.layout, self.push_stages, 0, push);
            dev.cmd_draw(cmd, 3, 1, 0, 0);
        }
    }

    pub fn destroy(&self, device: &VulkanDevice) {
        let dev = &device.device;
        unsafe {
            dev.destroy_pipeline(self.pipeline, None);
            dev.destroy_pipeline_layout(self.layout, None);
            if self.pool != vk::DescriptorPool::null() {
                dev.destroy_descriptor_pool(self.pool, None);
            }
            if self.sampler != vk::Sampler::null() {
                dev.destroy_sampler(self.sampler, None);
            }
            if self.empty_layout != vk::DescriptorSetLayout::null() {
                dev.destroy_descriptor_set_layout(self.empty_layout, None);
            }
            if self.set_layout != vk::DescriptorSetLayout::null() {
                dev.destroy_descriptor_set_layout(self.set_layout, None);
            }
        }
    }
}
