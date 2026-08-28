//! HDR composite pipeline (M5 stage 1a). A parallel composite pipeline used
//! ONLY when the HDR output path is active; the SDR GLSL composite
//! (`pipeline.composite`) is untouched. Shaders are `composite_hdr.wgsl`
//! compiled to SPIR-V by naga at build time (`OUT_DIR/composite_hdr.spv`).
//!
//! Bindings (WGSL → SPIR-V): set 0 = {sampled image (0), sampler (1)} per
//! textured draw; set 1 = {uniform (0)} the `Tuning` buffer, bound once per
//! frame. Push constants (64 B) carry per-draw geometry + the per-surface flag.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;

const SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/composite_hdr.spv"));

/// Per-draw push constants — matches the WGSL `Push` (64 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HdrPush {
    pub dst: [f32; 4],
    pub src: [f32; 4],
    pub color: [f32; 4],
    /// x = source transfer (0 sRGB, 1 PQ, 2 HLG, 3 linear), y = is_hdr (0/1).
    pub surf: [f32; 4],
}

/// Tuning uniform — matches the WGSL `Tuning` (12 f32, 48 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HdrTuningUbo {
    pub enabled: f32,
    pub sdr_white_nits: f32,
    pub max_nits: f32,
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub gamut: f32,
    pub tone_map: f32,
    pub transfer: f32,
    pub gamma: f32,
    pub exposure: f32,
    pub _pad: f32,
}

#[derive(Debug, thiserror::Error)]
pub enum HdrError {
    #[error("vulkan call failed: {0}")]
    Vk(String),
    #[error("no host-visible memory type for the tuning UBO")]
    NoMemoryType,
}

pub struct HdrComposite {
    set0_layout: vk::DescriptorSetLayout,
    set1_layout: vk::DescriptorSetLayout,
    layout: vk::PipelineLayout,
    textured: vk::Pipeline,
    /// Blending OFF, RGB-only write — the opaque-region variant. See the SDR
    /// composite's `textured_opaque`; the reasoning is identical and applies here
    /// whenever the HDR composite is the one drawing client surfaces.
    textured_opaque: vk::Pipeline,
    solid: vk::Pipeline,
    sampler: vk::Sampler,
    ubo: vk::Buffer,
    ubo_mem: vk::DeviceMemory,
    ubo_mapped: *mut u8,
    ubo_pool: vk::DescriptorPool,
    ubo_set: vk::DescriptorSet,
    tex_pool: vk::DescriptorPool,
    pub color_format: vk::Format,
}

fn shader_module(dev: &ash::Device, spv: &[u8]) -> Result<vk::ShaderModule, HdrError> {
    let words: Vec<u32> = spv
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let info = vk::ShaderModuleCreateInfo::default().code(&words);
    unsafe {
        dev.create_shader_module(&info, None)
            .map_err(|e| HdrError::Vk(format!("shader module: {e}")))
    }
}

impl HdrComposite {
    pub fn create(
        device: &VulkanDevice,
        phd: vk::PhysicalDevice,
        cache: vk::PipelineCache,
        color_format: vk::Format,
    ) -> Result<Self, HdrError> {
        let dev = &device.device;

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
            .map_err(|e| HdrError::Vk(format!("sampler: {e}")))?
        };

        // set 0: sampled image + sampler (per textured draw).
        let set0_bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];
        let set0_layout = unsafe {
            dev.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&set0_bindings),
                None,
            )
            .map_err(|e| HdrError::Vk(format!("set0 layout: {e}")))?
        };

        // set 1: tuning uniform (bound once per frame).
        let set1_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        let set1_layout = unsafe {
            dev.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default()
                    .bindings(std::slice::from_ref(&set1_binding)),
                None,
            )
            .map_err(|e| HdrError::Vk(format!("set1 layout: {e}")))?
        };

        let push_range = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
            .offset(0)
            .size(std::mem::size_of::<HdrPush>() as u32);
        let set_layouts = [set0_layout, set1_layout];
        let layout = unsafe {
            dev.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&set_layouts)
                    .push_constant_ranges(std::slice::from_ref(&push_range)),
                None,
            )
            .map_err(|e| HdrError::Vk(format!("pipeline layout: {e}")))?
        };

        let module = shader_module(dev, SPV)?;
        let vs = std::ffi::CStr::from_bytes_with_nul(b"vs_main\0").unwrap();
        let fs_tex = std::ffi::CStr::from_bytes_with_nul(b"fs_tex\0").unwrap();
        let fs_solid = std::ffi::CStr::from_bytes_with_nul(b"fs_solid\0").unwrap();

        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_STRIP);
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
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
        // Opaque variant, same reasoning as the SDR composite: a surface inside its
        // declared opaque region must REPLACE the destination, because the damage
        // tracker has already excluded that region from the clear. RGB only, so the
        // target's alpha survives for anything drawn over it later this frame.
        let opaque_attachment = vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(false)
            .color_write_mask(
                vk::ColorComponentFlags::R | vk::ColorComponentFlags::G | vk::ColorComponentFlags::B,
            );
        let opaque_blend = vk::PipelineColorBlendStateCreateInfo::default()
            .attachments(std::slice::from_ref(&opaque_attachment));
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let color_formats = [color_format];

        let build = |frag: &std::ffi::CStr,
                     blend: &vk::PipelineColorBlendStateCreateInfo|
         -> Result<vk::Pipeline, HdrError> {
            let stages = [
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::VERTEX)
                    .module(module)
                    .name(vs),
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::FRAGMENT)
                    .module(module)
                    .name(frag),
            ];
            let mut rendering_info = vk::PipelineRenderingCreateInfo::default()
                .color_attachment_formats(&color_formats);
            let info = vk::GraphicsPipelineCreateInfo::default()
                .stages(&stages)
                .vertex_input_state(&vertex_input)
                .input_assembly_state(&input_assembly)
                .viewport_state(&viewport_state)
                .rasterization_state(&raster)
                .multisample_state(&multisample)
                .color_blend_state(blend)
                .dynamic_state(&dynamic)
                .layout(layout)
                .push_next(&mut rendering_info);
            unsafe {
                dev.create_graphics_pipelines(cache, std::slice::from_ref(&info), None)
                    .map_err(|(_, e)| HdrError::Vk(format!("graphics pipeline: {e}")))
                    .map(|p| p[0])
            }
        };
        let textured = build(fs_tex, &blend)?;
        let textured_opaque = build(fs_tex, &opaque_blend)?;
        let solid = build(fs_solid, &blend)?;
        unsafe { dev.destroy_shader_module(module, None) };

        // Tuning UBO: host-visible, persistently mapped, coherent.
        let ubo_size = std::mem::size_of::<HdrTuningUbo>() as vk::DeviceSize;
        let ubo = unsafe {
            dev.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(ubo_size)
                    .usage(vk::BufferUsageFlags::UNIFORM_BUFFER)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
            .map_err(|e| HdrError::Vk(format!("ubo buffer: {e}")))?
        };
        let req = unsafe { dev.get_buffer_memory_requirements(ubo) };
        let mem_props = unsafe { device.instance.get_physical_device_memory_properties(phd) };
        let mem_type = (0..mem_props.memory_type_count)
            .find(|&i| {
                req.memory_type_bits & (1 << i) != 0
                    && mem_props.memory_types[i as usize].property_flags.contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
            })
            .ok_or(HdrError::NoMemoryType)?;
        let ubo_mem = unsafe {
            dev.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(mem_type),
                None,
            )
            .map_err(|e| HdrError::Vk(format!("ubo memory: {e}")))?
        };
        unsafe {
            dev.bind_buffer_memory(ubo, ubo_mem, 0)
                .map_err(|e| HdrError::Vk(format!("ubo bind: {e}")))?;
        }
        let ubo_mapped = unsafe {
            dev.map_memory(ubo_mem, 0, ubo_size, vk::MemoryMapFlags::empty())
                .map_err(|e| HdrError::Vk(format!("ubo map: {e}")))? as *mut u8
        };

        // set 1 pool + set (persistent — the UBO never moves).
        let ubo_pool = unsafe {
            dev.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&[
                    vk::DescriptorPoolSize::default()
                        .ty(vk::DescriptorType::UNIFORM_BUFFER)
                        .descriptor_count(1),
                ]),
                None,
            )
            .map_err(|e| HdrError::Vk(format!("ubo pool: {e}")))?
        };
        let ubo_set = unsafe {
            dev.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(ubo_pool)
                    .set_layouts(std::slice::from_ref(&set1_layout)),
            )
            .map_err(|e| HdrError::Vk(format!("ubo set: {e}")))?[0]
        };
        let buf_info = vk::DescriptorBufferInfo::default()
            .buffer(ubo)
            .offset(0)
            .range(ubo_size);
        unsafe {
            dev.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(ubo_set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .buffer_info(std::slice::from_ref(&buf_info))],
                &[],
            );
        }

        // set 0 pool: reset + re-allocated each frame (one set per textured op).
        const MAX_TEX: u32 = 1024;
        let tex_pool = unsafe {
            dev.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default().max_sets(MAX_TEX).pool_sizes(&[
                    vk::DescriptorPoolSize::default()
                        .ty(vk::DescriptorType::SAMPLED_IMAGE)
                        .descriptor_count(MAX_TEX),
                    vk::DescriptorPoolSize::default()
                        .ty(vk::DescriptorType::SAMPLER)
                        .descriptor_count(MAX_TEX),
                ]),
                None,
            )
            .map_err(|e| HdrError::Vk(format!("tex pool: {e}")))?
        };

        Ok(Self {
            set0_layout,
            set1_layout,
            layout,
            textured,
            textured_opaque,
            solid,
            sampler,
            ubo,
            ubo_mem,
            ubo_mapped,
            ubo_pool,
            ubo_set,
            tex_pool,
            color_format,
        })
    }

    /// Upload the latest tuning into the mapped UBO (coherent — no flush).
    pub fn update_tuning(&self, t: &HdrTuningUbo) {
        unsafe {
            std::ptr::copy_nonoverlapping(
                t as *const HdrTuningUbo as *const u8,
                self.ubo_mapped,
                std::mem::size_of::<HdrTuningUbo>(),
            );
        }
    }

    /// Start a frame's HDR draws: reset the per-frame texture-set pool and bind
    /// the tuning set (set 1) once. Call after `composite::begin`.
    pub fn begin_frame(&self, device: &VulkanDevice, cmd: vk::CommandBuffer) {
        unsafe {
            let _ = device
                .device
                .reset_descriptor_pool(self.tex_pool, vk::DescriptorPoolResetFlags::empty());
            device.device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.layout,
                1,
                &[self.ubo_set],
                &[],
            );
        }
    }

    /// Allocate + write a set-0 descriptor (sampled image + sampler) for a draw.
    pub fn texture_set(
        &self,
        device: &VulkanDevice,
        view: vk::ImageView,
    ) -> Result<vk::DescriptorSet, HdrError> {
        let dev = &device.device;
        let set = unsafe {
            dev.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.tex_pool)
                    .set_layouts(std::slice::from_ref(&self.set0_layout)),
            )
            .map_err(|e| HdrError::Vk(format!("texture set: {e}")))?[0]
        };
        let img = vk::DescriptorImageInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        let smp = vk::DescriptorImageInfo::default().sampler(self.sampler);
        unsafe {
            dev.update_descriptor_sets(
                &[
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(0)
                        .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                        .image_info(std::slice::from_ref(&img)),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(1)
                        .descriptor_type(vk::DescriptorType::SAMPLER)
                        .image_info(std::slice::from_ref(&smp)),
                ],
                &[],
            );
        }
        Ok(set)
    }

    /// `opaque`: inside the surface's declared opaque region — replace, don't blend.
    pub fn draw_textured(
        &self,
        device: &VulkanDevice,
        cmd: vk::CommandBuffer,
        set0: vk::DescriptorSet,
        push: HdrPush,
        opaque: bool,
    ) {
        let dev = &device.device;
        unsafe {
            let pipeline = if opaque { self.textured_opaque } else { self.textured };
            dev.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipeline);
            dev.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.layout,
                0,
                &[set0],
                &[],
            );
            dev.cmd_push_constants(
                cmd,
                self.layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                bytes_of(&push),
            );
            dev.cmd_draw(cmd, 4, 1, 0, 0);
        }
    }

    pub fn draw_solid(&self, device: &VulkanDevice, cmd: vk::CommandBuffer, push: HdrPush) {
        let dev = &device.device;
        unsafe {
            dev.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.solid);
            dev.cmd_push_constants(
                cmd,
                self.layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                bytes_of(&push),
            );
            dev.cmd_draw(cmd, 4, 1, 0, 0);
        }
    }

    pub fn destroy(&self, device: &VulkanDevice) {
        let dev = &device.device;
        unsafe {
            dev.destroy_descriptor_pool(self.tex_pool, None);
            dev.destroy_descriptor_pool(self.ubo_pool, None);
            dev.unmap_memory(self.ubo_mem);
            dev.destroy_buffer(self.ubo, None);
            dev.free_memory(self.ubo_mem, None);
            dev.destroy_sampler(self.sampler, None);
            dev.destroy_pipeline(self.textured, None);
            dev.destroy_pipeline(self.textured_opaque, None);
            dev.destroy_pipeline(self.solid, None);
            dev.destroy_pipeline_layout(self.layout, None);
            dev.destroy_descriptor_set_layout(self.set0_layout, None);
            dev.destroy_descriptor_set_layout(self.set1_layout, None);
        }
    }
}

fn bytes_of(push: &HdrPush) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            (push as *const HdrPush) as *const u8,
            std::mem::size_of::<HdrPush>(),
        )
    }
}
