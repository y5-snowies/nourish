//! Construction (`new`/`new_default`/`validate`) and the one-time device-object
//! creators (command buffer, descriptor pool, timeline/render semaphores).

use ash::vk;
use compositor_kernel_vulkan_capture_blit_base::blit::CaptureCache;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use compositor_kernel_vulkan_memory_upload_base::upload::StagingBuffer;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::{Bind, Color32F, ContextId, DebugFlags, Frame, Renderer, Texture, TextureFilter};
use smithay::backend::vulkan::PhysicalDevice;
use smithay::utils::{Rectangle, Size, Transform};
use std::collections::HashMap;
use compositor_developer_stats_registry_base::base as stats;

use crate::error::VulkanError;
use super::VulkanRenderer;

impl VulkanRenderer {
    /// Build a renderer on the physical device matched to the scanout node
    /// (`instance.physical::for_node`).
    pub fn new(phd: PhysicalDevice) -> Result<Self, VulkanError> {
        let dev = compositor_kernel_vulkan_device_factory_base::factory::create(&phd)
            .map_err(|e| VulkanError::Vk(format!("device create: {e}")))?;
        let queue = compositor_kernel_vulkan_device_queue_base::queue::graphics_queue(&dev);
        let command_pool = compositor_kernel_vulkan_command_pool_base::pool::create(&dev)
            .map_err(|e| VulkanError::Vk(format!("command pool: {e}")))?;
        let cmd = Self::alloc_command_buffer(&dev, command_pool)?;
        let pipeline_cache = compositor_kernel_vulkan_pipeline_cache_base::cache::create(&dev)
            .map_err(|e| VulkanError::Vk(format!("pipeline cache: {e}")))?;
        let descriptor_pool = Self::create_descriptor_pool(&dev)?;
        let timeline = Self::create_timeline(&dev)?;
        let render_semaphore = Self::create_render_semaphore(&dev)?;
        let frame_fence = unsafe {
            dev.device.create_fence(
                &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                None,
            )?
        };

        // `renderer_sync == "infence"` opts in to the native KMS IN_FENCE path.
        // DEFAULT is synchronous `device_wait_idle`.
        let native_fence_optin = compositor_developer_environment_config_base::base::get()
            .renderer_sync
            .eq_ignore_ascii_case("infence");
        info!(
            "VulkanRenderer initialized (queue family {}, native_fence_optin={})",
            queue.family_index, native_fence_optin
        );
        stats::set_renderer("vulkan", true);
        stats::set_sync_mode("synchronous (device_wait_idle)");

        Ok(Self {
            dev,
            phd,
            queue,
            command_pool,
            cmd,
            pipeline_cache,
            pipelines: HashMap::new(),
            aa_pipelines: HashMap::new(),
            mipgen: std::cell::RefCell::new(crate::renderer::mipgen::MipGen::default()),
            aa_was_active: false,
            shader_passes: HashMap::new(),
            hdr_pipelines: HashMap::new(),
            hdr_enabled: false,
            descriptor_pool,
            timeline,
            render_semaphore,
            frame_fence,
            drm_fd: None,
            native_fence_optin,
            last_fence_warn: None,
            capture_targets: Vec::new(),
            capture_cache: CaptureCache::new(),
            shm_staging: StagingBuffer::new(),
            frame_counter: 0,
            debug_flags: DebugFlags::empty(),
            downscale: TextureFilter::Linear,
            upscale: TextureFilter::Linear,
            context_id: ContextId::new(),
        })
    }

    /// Build a renderer on the first available vulkan physical device. Used by
    /// the env-gated winit Vulkan present path and the `validate()` self-test.
    pub fn new_default() -> Result<Self, VulkanError> {
        let instance = compositor_kernel_vulkan_instance_factory_base::factory::create()
            .map_err(|e| VulkanError::Vk(format!("instance: {e:?}")))?;
        let phd = compositor_kernel_vulkan_instance_physical_base::physical::enumerate(&instance)
            .map_err(VulkanError::Vk)?
            .into_iter()
            .next()
            .ok_or(VulkanError::Unimplemented("no vulkan physical device found"))?;
        info!("vulkan renderer: using physical device '{}'", phd.name());
        Self::new(phd)
    }

    /// Hardware self-test: build a renderer on the first vulkan device, export a
    /// 256×256 dmabuf, bind it as a target, and render one frame (clear + a
    /// solid quad), then round-trip the dmabuf back through `import_dmabuf`.
    pub fn validate() -> Result<String, VulkanError> {
        let instance = compositor_kernel_vulkan_instance_factory_base::factory::create()
            .map_err(|e| VulkanError::Vk(format!("instance: {e:?}")))?;
        let phd = compositor_kernel_vulkan_instance_physical_base::physical::enumerate(&instance)
            .map_err(VulkanError::Vk)?
            .into_iter()
            .next()
            .ok_or(VulkanError::Unimplemented("no vulkan physical device found"))?;
        let phd_name = phd.name().to_string();
        info!("vulkan validate: using physical device '{phd_name}'");

        let mut renderer = Self::new(phd)?;

        // A 256×256 exportable render target, exported to a dmabuf we then bind.
        let fourcc = Fourcc::Argb8888;
        let vk_fmt = compositor_kernel_vulkan_format_query_base::query::vk_format(fourcc)
            .ok_or(VulkanError::UnsupportedFormat(fourcc))?;
        let mods: Vec<_> =
            compositor_kernel_vulkan_format_modifier_base::modifier::modifiers(&renderer.phd, vk_fmt)
                .into_iter()
                .map(|(m, _)| m)
                .collect();
        let target = compositor_kernel_vulkan_memory_export_base::export::create_exportable(
            &renderer.dev,
            &renderer.phd,
            fourcc,
            (256, 256),
            &mods,
        )
        .map_err(|e| VulkanError::Vk(format!("create_exportable: {e:?}")))?;
        let mut dmabuf = compositor_kernel_vulkan_memory_export_base::export::export(&renderer.dev, &target)
            .map_err(|e| VulkanError::Vk(format!("export: {e:?}")))?;
        // The self-test's exportable image is only ever sampled back via the
        // dmabuf below; the source Vk image/memory are not needed afterwards, so
        // free them (the dmabuf fd keeps the underlying allocation alive).
        target.destroy(&renderer.dev);

        {
            let mut fb = <Self as Bind<Dmabuf>>::bind(&mut renderer, &mut dmabuf)?;
            let size = Size::from((256, 256));
            let mut frame = renderer.render(&mut fb, size, Transform::Normal)?;
            frame.clear(
                Color32F::new(0.1, 0.2, 0.3, 1.0),
                &[Rectangle::from_size(size)],
            )?;
            frame.draw_solid(
                Rectangle::from_loc_and_size((32, 32), (64, 64)),
                &[Rectangle::from_size(size)],
                Color32F::new(1.0, 0.0, 0.0, 1.0),
            )?;
            frame.finish()?;
        }

        {
            let tex = <Self as smithay::backend::renderer::ImportDma>::import_dmabuf(
                &mut renderer,
                &dmabuf,
                None,
            )?;
            info!(
                "vulkan validate: import_dmabuf round-trip OK ({}x{})",
                tex.width(),
                tex.height()
            );
        }

        Ok(format!(
            "VulkanRenderer validated on '{phd_name}': exported 256x256 dmabuf, \
             bound it as a target, cleared + drew a solid quad, submitted"
        ))
    }

    pub(super) fn alloc_command_buffer(
        dev: &VulkanDevice,
        pool: vk::CommandPool,
    ) -> Result<vk::CommandBuffer, VulkanError> {
        let info = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let bufs = unsafe { dev.device.allocate_command_buffers(&info)? };
        Ok(bufs[0])
    }

    fn create_descriptor_pool(dev: &VulkanDevice) -> Result<vk::DescriptorPool, VulkanError> {
        const MAX_SETS: u32 = 1024;
        let sizes = [vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(MAX_SETS)];
        let info = vk::DescriptorPoolCreateInfo::default()
            .flags(vk::DescriptorPoolCreateFlags::empty())
            .max_sets(MAX_SETS)
            .pool_sizes(&sizes);
        Ok(unsafe { dev.device.create_descriptor_pool(&info, None)? })
    }

    fn create_timeline(dev: &VulkanDevice) -> Result<vk::Semaphore, VulkanError> {
        let mut type_info = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(0);
        let info = vk::SemaphoreCreateInfo::default().push_next(&mut type_info);
        Ok(unsafe { dev.device.create_semaphore(&info, None)? })
    }

    /// A binary semaphore whose signal can be exported as a `sync_file`
    /// (SYNC_FD). Re-signaled each frame; SYNC_FD export consumes the pending
    /// signal, leaving it ready to be signaled again next frame.
    fn create_render_semaphore(dev: &VulkanDevice) -> Result<vk::Semaphore, VulkanError> {
        let mut export = vk::ExportSemaphoreCreateInfo::default()
            .handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
        let info = vk::SemaphoreCreateInfo::default().push_next(&mut export);
        Ok(unsafe { dev.device.create_semaphore(&info, None)? })
    }
}
