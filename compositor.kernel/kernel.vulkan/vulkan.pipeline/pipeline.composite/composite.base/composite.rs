//! The composition pipeline: textured-quad + solid-color draws into a
//! dynamic-rendering color attachment — the vulkan counterpart of
//! GlesRenderer's element drawing. Phase 4 Step 3 — real.
//!
//! Shaders are compiled SPIR-V embedded at build time (sources `quad.vert`,
//! `tex.frag`, `solid.frag` sit beside this file; rebuilt with
//! `glslangValidator -V --target-env vulkan1.3`). The quad is generated from
//! gl_VertexIndex; geometry travels in push constants — no vertex buffers.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;

pub const VERT_SPV: &[u8] = include_bytes!("quad.vert.spv");
pub const TEX_FRAG_SPV: &[u8] = include_bytes!("tex.frag.spv");
pub const SOLID_FRAG_SPV: &[u8] = include_bytes!("solid.frag.spv");

/// Push constants shared by both pipelines (48 bytes, VS+FS).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PushQuad {
    /// x, y, w, h in NDC.
    pub dst: [f32; 4],
    /// u, v, w, h in UV space.
    pub src: [f32; 4],
    /// rgba for solid; (1,1,1,alpha) for textured.
    pub color: [f32; 4],
}

pub struct CompositePipelines {
    pub descriptor_layout: vk::DescriptorSetLayout,
    pub layout: vk::PipelineLayout,
    pub textured: vk::Pipeline,
    pub solid: vk::Pipeline,
    pub sampler: vk::Sampler,
    pub color_format: vk::Format,
}

#[derive(Debug, thiserror::Error)]
pub enum CompositeError {
    #[error("vulkan call failed: {0}")]
    Vk(String),
}

fn shader_module(dev: &ash::Device, spv: &[u8]) -> Result<vk::ShaderModule, CompositeError> {
    // SPIR-V is u32-aligned; the embedded bytes come from the compiler so the
    // length is a multiple of 4.
    let words: Vec<u32> = spv
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let info = vk::ShaderModuleCreateInfo::default().code(&words);
    unsafe {
        dev.create_shader_module(&info, None)
            .map_err(|e| CompositeError::Vk(format!("shader module: {e}")))
    }
}

pub fn create(
    device: &VulkanDevice,
    cache: vk::PipelineCache,
    color_format: vk::Format,
) -> Result<CompositePipelines, CompositeError> {
    let dev = &device.device;

    // Sampler + single combined-image-sampler binding for the textured arm.
    // Plain bilinear — all anti-aliasing lives in the separate `AaComposite`.
    let sampler_info = vk::SamplerCreateInfo::default()
        .mag_filter(vk::Filter::LINEAR)
        .min_filter(vk::Filter::LINEAR)
        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE);
    let sampler = unsafe {
        dev.create_sampler(&sampler_info, None)
            .map_err(|e| CompositeError::Vk(format!("sampler: {e}")))?
    };

    let binding = vk::DescriptorSetLayoutBinding::default()
        .binding(0)
        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::FRAGMENT);
    let dsl_info = vk::DescriptorSetLayoutCreateInfo::default()
        .bindings(std::slice::from_ref(&binding));
    let descriptor_layout = unsafe {
        dev.create_descriptor_set_layout(&dsl_info, None)
            .map_err(|e| CompositeError::Vk(format!("descriptor layout: {e}")))?
    };

    let push_range = vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
        .offset(0)
        .size(std::mem::size_of::<PushQuad>() as u32);
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(std::slice::from_ref(&descriptor_layout))
        .push_constant_ranges(std::slice::from_ref(&push_range));
    let layout = unsafe {
        dev.create_pipeline_layout(&layout_info, None)
            .map_err(|e| CompositeError::Vk(format!("pipeline layout: {e}")))?
    };

    let vert = shader_module(dev, VERT_SPV)?;
    let tex_frag = shader_module(dev, TEX_FRAG_SPV)?;
    let solid_frag = shader_module(dev, SOLID_FRAG_SPV)?;

    let entry = std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap();
    let make_stages = |frag: vk::ShaderModule| {
        [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vert)
                .name(entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(frag)
                .name(entry),
        ]
    };

    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_STRIP);
    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);
    let raster = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .line_width(1.0);
    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);
    // Premultiplied-alpha over (matching the gles path's blending).
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
    let build = |stages: &[vk::PipelineShaderStageCreateInfo]| {
        let mut rendering_info = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&color_formats);
        let info = vk::GraphicsPipelineCreateInfo::default()
            .stages(stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .color_blend_state(&blend)
            .dynamic_state(&dynamic)
            .layout(layout)
            .push_next(&mut rendering_info);
        unsafe {
            dev.create_graphics_pipelines(cache, std::slice::from_ref(&info), None)
                .map_err(|(_, e)| CompositeError::Vk(format!("graphics pipeline: {e}")))
                .map(|p| p[0])
        }
    };

    let textured = build(&make_stages(tex_frag))?;
    let solid = build(&make_stages(solid_frag))?;

    unsafe {
        dev.destroy_shader_module(vert, None);
        dev.destroy_shader_module(tex_frag, None);
        dev.destroy_shader_module(solid_frag, None);
    }

    Ok(CompositePipelines {
        descriptor_layout,
        layout,
        textured,
        solid,
        sampler,
        color_format,
    })
}

/// Begin a dynamic-rendering composition pass into `target`.
pub fn begin(
    device: &VulkanDevice,
    cmd: vk::CommandBuffer,
    target: vk::ImageView,
    extent: (u32, u32),
    clear: [f32; 4],
) {
    let attachment = vk::RenderingAttachmentInfo::default()
        .image_view(target)
        .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .clear_value(vk::ClearValue {
            color: vk::ClearColorValue { float32: clear },
        });
    let area = vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent: vk::Extent2D {
            width: extent.0,
            height: extent.1,
        },
    };
    let rendering = vk::RenderingInfo::default()
        .render_area(area)
        .layer_count(1)
        .color_attachments(std::slice::from_ref(&attachment));
    unsafe {
        device.device.cmd_begin_rendering(cmd, &rendering);
        device.device.cmd_set_viewport(
            cmd,
            0,
            &[vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: extent.0 as f32,
                height: extent.1 as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            }],
        );
        device.device.cmd_set_scissor(cmd, 0, &[area]);
    }
}

pub fn draw_textured(
    device: &VulkanDevice,
    pipelines: &CompositePipelines,
    cmd: vk::CommandBuffer,
    descriptor_set: vk::DescriptorSet,
    quad: PushQuad,
) {
    unsafe {
        let dev = &device.device;
        dev.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipelines.textured);
        dev.cmd_bind_descriptor_sets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            pipelines.layout,
            0,
            &[descriptor_set],
            &[],
        );
        dev.cmd_push_constants(
            cmd,
            pipelines.layout,
            vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
            0,
            bytes_of(&quad),
        );
        dev.cmd_draw(cmd, 4, 1, 0, 0);
    }
}

pub fn draw_solid(
    device: &VulkanDevice,
    pipelines: &CompositePipelines,
    cmd: vk::CommandBuffer,
    quad: PushQuad,
) {
    unsafe {
        let dev = &device.device;
        dev.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipelines.solid);
        dev.cmd_push_constants(
            cmd,
            pipelines.layout,
            vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
            0,
            bytes_of(&quad),
        );
        dev.cmd_draw(cmd, 4, 1, 0, 0);
    }
}

pub fn end(device: &VulkanDevice, cmd: vk::CommandBuffer) {
    unsafe { device.device.cmd_end_rendering(cmd) };
}

fn bytes_of(quad: &PushQuad) -> &[u8] {
    // PushQuad is repr(C), plain f32s — safe to view as bytes.
    unsafe {
        std::slice::from_raw_parts(
            (quad as *const PushQuad) as *const u8,
            std::mem::size_of::<PushQuad>(),
        )
    }
}

// ===========================================================================
// world anti-aliasing pipeline (see shaders/aa.wgsl).
// ===========================================================================

/// The `aa.wgsl` module, naga-compiled to SPIR-V by `build.rs`.
pub const AA_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/aa.spv"));

/// Which pre-built sampler a textured draw binds. The renderer picks this from
/// the graphics config's method; the per-draw numeric knobs (taps/spread/
/// sharpen/lod_bias) travel in [`AaPush`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SamplerSel {
    /// Bilinear, no mips — SSAA (samples the source directly) and mip-0 fills.
    Bilinear,
    /// LINEAR-mipmap (trilinear).
    Trilinear,
    /// Anisotropic (with `level` max anisotropy), nearest available sampler.
    Aniso,
}

/// Per-draw push constants — matches `aa.wgsl`'s `Push` (80 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AaPush {
    pub dst: [f32; 4],
    pub src: [f32; 4],
    pub color: [f32; 4],
    /// x = taps per axis (>= 1); y = spread; z = sharpen amount; w = mip LOD bias.
    pub params: [f32; 4],
    /// FSR toggles + inputs (independent of the classic AA method in `params`).
    /// x = EASU on (>0.5); y = RCAS strength (0 = RCAS off, else 0..1); z,w =
    /// source texture width,height in texels (EASU/RCAS fetch by integer texel).
    pub params2: [f32; 4],
}

/// The world anti-aliasing composite pipeline: one WGSL pipeline with three pre-built
/// sampler variants, selected per draw by [`SamplerSel`]. Separate
/// image+sampler bindings (naga can't emit combined samplers), so it carries
/// its own descriptor layout + per-frame pool, independent of the plain GLSL
/// composite above.
pub struct AaComposite {
    set_layout: vk::DescriptorSetLayout,
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    sampler_bilinear: vk::Sampler,
    /// Anisotropic samplers at a few max-anisotropy levels (each also LINEAR-mip
    /// so it samples the generated mip chain). Empty when the device lacks
    /// anisotropy — `sampler_for` then falls back to trilinear.
    aniso: Vec<(f32, vk::Sampler)>,
    sampler_trilinear: vk::Sampler,
    tex_pool: vk::DescriptorPool,
    pub color_format: vk::Format,
}

impl AaComposite {
    pub fn create(
        device: &VulkanDevice,
        cache: vk::PipelineCache,
        color_format: vk::Format,
        max_anisotropy: f32,
    ) -> Result<Self, CompositeError> {
        let dev = &device.device;

        let base = || {
            vk::SamplerCreateInfo::default()
                .mag_filter(vk::Filter::LINEAR)
                .min_filter(vk::Filter::LINEAR)
                .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        };
        let mk = |info: &vk::SamplerCreateInfo| -> Result<vk::Sampler, CompositeError> {
            unsafe { dev.create_sampler(info, None) }
                .map_err(|e| CompositeError::Vk(format!("aa sampler: {e}")))
        };
        let sampler_bilinear = mk(&base())?;
        // Trilinear: LINEAR mipmap sampling of the generated mip chain.
        let trilinear_info = || {
            base()
                .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
                .min_lod(0.0)
                .max_lod(vk::LOD_CLAMP_NONE)
        };
        let sampler_trilinear = mk(&trilinear_info())?;
        // Anisotropic (mip-sampling) samplers at a few levels, each clamped to
        // the device max; `sampler_for` picks the nearest to the config value.
        let mut aniso: Vec<(f32, vk::Sampler)> = Vec::new();
        if max_anisotropy > 1.0 {
            for lvl in [2.0f32, 4.0, 8.0, 16.0] {
                if lvl <= max_anisotropy + 0.5 {
                    let s = mk(&trilinear_info().anisotropy_enable(true).max_anisotropy(lvl))?;
                    aniso.push((lvl, s));
                }
            }
            if aniso.is_empty() {
                let s = mk(&trilinear_info()
                    .anisotropy_enable(true)
                    .max_anisotropy(max_anisotropy))?;
                aniso.push((max_anisotropy, s));
            }
        }

        let set_bindings = [
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
        let set_layout = unsafe {
            dev.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&set_bindings),
                None,
            )
            .map_err(|e| CompositeError::Vk(format!("aa set layout: {e}")))?
        };

        let push_range = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
            .offset(0)
            .size(std::mem::size_of::<AaPush>() as u32);
        let layout = unsafe {
            dev.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(std::slice::from_ref(&set_layout))
                    .push_constant_ranges(std::slice::from_ref(&push_range)),
                None,
            )
            .map_err(|e| CompositeError::Vk(format!("aa pipeline layout: {e}")))?
        };

        let module = shader_module(dev, AA_SPV)?;
        let vs = std::ffi::CStr::from_bytes_with_nul(b"vs_main\0").unwrap();
        let fs = std::ffi::CStr::from_bytes_with_nul(b"fs_main\0").unwrap();
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(module)
                .name(vs),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(module)
                .name(fs),
        ];

        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_STRIP);
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
                .map_err(|(_, e)| CompositeError::Vk(format!("aa pipeline: {e}")))?[0]
        };
        unsafe { dev.destroy_shader_module(module, None) };

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
            .map_err(|e| CompositeError::Vk(format!("aa tex pool: {e}")))?
        };

        Ok(Self {
            set_layout,
            layout,
            pipeline,
            sampler_bilinear,
            aniso,
            sampler_trilinear,
            tex_pool,
            color_format,
        })
    }

    fn sampler_for(&self, sel: SamplerSel, aniso_level: f32) -> vk::Sampler {
        match sel {
            SamplerSel::Trilinear => self.sampler_trilinear,
            SamplerSel::Bilinear => self.sampler_bilinear,
            // Highest built level not exceeding the requested one (fall back to
            // the smallest, or to trilinear if the device has no anisotropy).
            SamplerSel::Aniso => {
                let mut chosen = match self.aniso.first() {
                    Some(&(_, s)) => s,
                    None => return self.sampler_trilinear,
                };
                for &(lvl, s) in &self.aniso {
                    if lvl <= aniso_level + 0.5 {
                        chosen = s;
                    }
                }
                chosen
            }
        }
    }

    /// Reset the per-frame descriptor pool. Call once before recording draws.
    pub fn begin_frame(&self, device: &VulkanDevice) {
        unsafe {
            let _ = device
                .device
                .reset_descriptor_pool(self.tex_pool, vk::DescriptorPoolResetFlags::empty());
        }
    }

    /// Allocate + write a set (image + selected sampler) for one draw.
    /// `aniso_level` selects the anisotropy sampler for `SamplerSel::Aniso`
    /// (ignored otherwise).
    pub fn texture_set(
        &self,
        device: &VulkanDevice,
        view: vk::ImageView,
        sel: SamplerSel,
        aniso_level: f32,
    ) -> Result<vk::DescriptorSet, CompositeError> {
        let dev = &device.device;
        let set = unsafe {
            dev.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.tex_pool)
                    .set_layouts(std::slice::from_ref(&self.set_layout)),
            )
            .map_err(|e| CompositeError::Vk(format!("aa texture set: {e}")))?[0]
        };
        let img = vk::DescriptorImageInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        let smp = vk::DescriptorImageInfo::default().sampler(self.sampler_for(sel, aniso_level));
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

    pub fn draw(
        &self,
        device: &VulkanDevice,
        cmd: vk::CommandBuffer,
        set: vk::DescriptorSet,
        push: AaPush,
    ) {
        let dev = &device.device;
        unsafe {
            dev.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
            dev.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.layout,
                0,
                &[set],
                &[],
            );
            dev.cmd_push_constants(
                cmd,
                self.layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                aa_bytes_of(&push),
            );
            dev.cmd_draw(cmd, 4, 1, 0, 0);
        }
    }

    pub fn destroy(&self, device: &VulkanDevice) {
        let dev = &device.device;
        unsafe {
            dev.destroy_descriptor_pool(self.tex_pool, None);
            dev.destroy_sampler(self.sampler_bilinear, None);
            for &(_, s) in &self.aniso {
                dev.destroy_sampler(s, None);
            }
            dev.destroy_sampler(self.sampler_trilinear, None);
            dev.destroy_pipeline(self.pipeline, None);
            dev.destroy_pipeline_layout(self.layout, None);
            dev.destroy_descriptor_set_layout(self.set_layout, None);
        }
    }
}

fn aa_bytes_of(push: &AaPush) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            (push as *const AaPush) as *const u8,
            std::mem::size_of::<AaPush>(),
        )
    }
}
