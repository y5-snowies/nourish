use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use compositor_kernel_vulkan_renderer_error_base::VulkanError;

/// A built fullscreen-shader pipeline for one color format + shader module.
pub struct FullscreenPass {
    pub pipeline: vk::Pipeline,
    pub layout: vk::PipelineLayout,
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
            .map_err(|e| VulkanError::Vk(format!("fullscreen shader module: {e}")))
    }
}

impl FullscreenPass {
    /// Build the pipeline. `spv` holds the fragment stage (and the vertex stage
    /// too, unless `vert_spv` is `Some`, in which case the vertex stage comes
    /// from that separate module — used for `glsl/` fragment-only bundles).
    /// `push_size` is the push-constant range in bytes (visible to vert+frag).
    pub fn create(
        device: &VulkanDevice,
        color_format: vk::Format,
        spv: &[u8],
        vert_spv: Option<&[u8]>,
        vert_entry: &str,
        frag_entry: &str,
        push_size: u32,
    ) -> Result<Self, VulkanError> {
        let dev = &device.device;
        let push_stages = vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT;

        let push_range = vk::PushConstantRange::default()
            .stage_flags(push_stages)
            .offset(0)
            .size(push_size);
        let layout = unsafe {
            dev.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .push_constant_ranges(std::slice::from_ref(&push_range)),
                None,
            )
            .map_err(|e| VulkanError::Vk(format!("fullscreen pipeline layout: {e}")))?
        };

        // One module for both stages, or a separate vertex module when given.
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
        // Premultiplied-alpha over (the shader emits color*alpha); the same blend
        // the composite pipeline uses, so the pass sits correctly under the scene.
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
            dev.create_graphics_pipelines(vk::PipelineCache::null(), std::slice::from_ref(&info), None)
                .map_err(|(_, e)| VulkanError::Vk(format!("fullscreen graphics pipeline: {e}")))?[0]
        };

        unsafe {
            dev.destroy_shader_module(frag_module, None);
            if vert_module != frag_module {
                dev.destroy_shader_module(vert_module, None);
            }
        };

        Ok(Self {
            pipeline,
            layout,
            color_format,
        })
    }

    /// Bind the pipeline and draw the fullscreen triangle with `push` bytes
    /// (visible to both shader stages).
    pub fn draw(&self, device: &VulkanDevice, cmd: vk::CommandBuffer, push: &[u8]) {
        let dev = &device.device;
        let push_stages = vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT;
        unsafe {
            dev.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
            dev.cmd_push_constants(cmd, self.layout, push_stages, 0, push);
            dev.cmd_draw(cmd, 3, 1, 0, 0);
        }
    }

    pub fn destroy(&self, device: &VulkanDevice) {
        unsafe {
            device.device.destroy_pipeline(self.pipeline, None);
            device.device.destroy_pipeline_layout(self.layout, None);
        }
    }
}
