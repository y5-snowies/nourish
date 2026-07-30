//! Renderer-side assembly: allocator/exporter -> GpuManager -> render formats
//! (Law-7 modifier filter when gated) -> hosted DrmOutputManager -> pipe
//! bring-up over the mode fallback chain -> gpu binding + contract.
//! (Ex wire.rs `new()` steps 5 + 8, recomposed.)
//!
//! Renderer SELECTION goes through here (preference or the entry's
//! `renderer-vulkan` compile-time override). There is NO fallback between
//! renderers: vulkan selected without the feature panics; vulkan selected
//! with the feature runs the full foundation self-test — exercising every
//! vulkan crate end-to-end (device, allocation, dmabuf export -> re-import,
//! composition recording with a solid+textured frame, timeline submit, the
//! semaphore <-> syncobj round trip) — and then panics with the proof and
//! the one remaining gap (the hosted scanout aliases are GBM-typed until the
//! scanout de-delegation generalizes the allocator position). No half-wired
//! pipe, no silent degradation.

use compositor_kernel_gles_element_wrap_base::wrap::GlesElementWrapper;
use compositor_kernel_native_assemble_display_base::display::DisplayAssembly;
use compositor_kernel_scanout_surface_output_base::output::{
    NativeDrmOutput, NativeDrmOutputManager,
};
use compositor_kernel_graphic_render_contract_base::contract::{RenderContract, RendererId};
use compositor_kernel_graphic_preference_renderer_rank::rank::RendererKind;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::drm::VrrSupport;
use smithay::backend::renderer::ImportEgl;
use smithay::output::{Mode, Output};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::backend::renderer::gles::GlesRenderer;
use std::cell::RefCell;
use std::rc::Rc;
use compositor_orchestration_draw_scene_element::element::SceneElement;
use compositor_orchestration_core_state_base::state::StateDRMBinding;

pub struct RendererAssembly {
    pub gpu_binding: Rc<RefCell<StateDRMBinding>>,
    pub drm_output_manager: Rc<RefCell<NativeDrmOutputManager>>,
    pub drm_output: NativeDrmOutput,
}

/// Preference-driven assembly (the default entry).
pub fn assemble(display: &mut DisplayAssembly) -> RendererAssembly {
    assemble_with(display, None)
}

/// Assembly with an optional compile-time override (the entry's
/// `renderer-vulkan` feature passes `Some(RendererKind::Vulkan)`).
pub fn assemble_with(
    display: &mut DisplayAssembly,
    override_kind: Option<RendererKind>,
) -> RendererAssembly {
    let kind = override_kind
        .or_else(|| {
            compositor_kernel_graphic_preference_renderer_rank::rank::get()
                .order
                .first()
                .copied()
        })
        .unwrap_or(RendererKind::Gles);

    match kind {
        RendererKind::Gles => assemble_gles(display),
        RendererKind::Vulkan => vulkan_selected(display),
    }
}

/// The gles arm (the original path, with the mode fallback chain made real).
fn assemble_gles(display: &mut DisplayAssembly) -> RendererAssembly {
    // Native scanout machine validation (reinstated de-delegation crates):
    // kernel-checked against the real device, screen untouched. The hosted
    // manager remains the live path until the swap-over; a compiled-in
    // machine that fails its proof panics.
    #[cfg(feature = "native-scanout")]
    {
        let proof = native_scanout_self_test(display);
        info!("native scanout machine validated (TEST_ONLY): {proof}");
    }

    // GpuManager with the High-priority EGL factory; register the node.
    let mut gpus = compositor_kernel_gles_multigpu_factory_base::factory::create();
    compositor_kernel_gles_multigpu_factory_base::factory::add_node(
        &mut gpus,
        display.primary_gpu,
        display.gbm.clone(),
    );

    // Allocator + exporter (flag policy in drm.gbm/gbm.alloc).
    let allocator = compositor_kernel_drm_gbm_alloc_base::alloc::allocator(display.gbm.clone());
    let exporter =
        compositor_kernel_drm_gbm_alloc_base::alloc::exporter(display.gbm.clone(), display.primary_gpu);

    // Render formats from the primary's EGL context, narrowed by the Law-7
    // modifier filter when its double gate is satisfied.
    let mut renderer = compositor_kernel_gles_multigpu_factory_base::factory::single_renderer(
        &mut gpus,
        &display.primary_gpu,
    );
    let render_formats = filter_formats(
        renderer
            .as_mut()
            .egl_context()
            .dmabuf_render_formats()
            .clone(),
    );

    let drm = display
        .drm
        .take()
        .expect("DrmDevice already taken by a previous renderer assembly");
    // Offer a 10-bit scanout in two independent cases (smithay falls back to
    // 8-bit if the plane can't): HDR (PQ needs the precision) and plain
    // deep-color SDR via depth == 10. Depth and HDR are decoupled — HDR
    // implies 10-bit, but depth == 10 gives 10-bit SDR without engaging
    // the PQ/HDR composite (the SDR transfer is byte-range identical, just finer
    // quantization). PQ is only signalled (stage C) when HDR is actually active.
    let env = compositor_model_environment_config_base::base::get();
    let hdr_scanout = display.hdr.hdr_capable() && env.hdr;
    let deep_color = env.depth == 10;
    let ten_bit = hdr_scanout || deep_color;
    info!("native scanout: hdr={hdr_scanout} deep_color={deep_color} → 10-bit={ten_bit}");
    let mut drm_output_manager = compositor_kernel_scanout_surface_output_base::output::manager(
        drm,
        allocator,
        exporter,
        Some(display.gbm.clone()),
        render_formats,
        ten_bit,
    );

    // Pipe bring-up over the validating-modeset fallback chain (the original
    // wire.rs pseudocode made real). Chain exhaustion is the panic.
    let chain = display.mode_chain.clone();
    let mut slot: Option<NativeDrmOutput> = None;
    let chosen = compositor_kernel_scanout_commit_test_base::test::try_chain(chain, |mode| {
        match compositor_kernel_scanout_surface_output_base::output::initialize::<
            _,
            GlesElementWrapper<SceneElement<GlesRenderer>>,
        >(
            &mut drm_output_manager,
            display.pipe,
            mode,
            &[display.connector.handle()],
            &display.output,
            &mut renderer,
        ) {
            Ok(out) => {
                slot = Some(out);
                Ok(())
            }
            Err(e) => Err(e),
        }
    })
    .unwrap_or_else(|e| abort!("every candidate mode failed the validating modeset: {e}"));

    if chosen != display.drm_mode {
        // Keep the assembly's published mode honest with what actually drove
        // the pipe; the Output state propagates through the same path
        // `device.interface` uses.
        display.drm_mode = chosen;
        display.mode = Mode::from(chosen);
        display
            .output
            .change_current_state(Some(display.mode), None, None, None);
        warn!(
            "selected mode failed; pipe driven by fallback {}x{}@{}",
            chosen.size().0,
            chosen.size().1,
            chosen.vrefresh()
        );
    }
    let drm_output = slot.expect("try_chain returned Ok without an initialized output");

    // Record what the swapchain ACTUALLY settled on, now the modeset has succeeded and
    // smithay has chosen from the render formats we offered.
    //
    // `set_device_format` was previously called only for the bevy and monitor dmabuf
    // allocations, so the one buffer that matters most for scanout cost — the framebuffer
    // being flipped — never reached the Statistics tab. LINEAR versus a vendor tiled modifier
    // is the difference between "this hardware is slow" and "we are scanning out
    // uncompressed"; on a tiler (V3D) a linear render target is a structural cost, not a
    // rounding error. Pairs with the `IN_FORMATS` dump in `assemble.display`: that reports
    // what the plane WOULD accept, this reports what we took.
    drm_output.with_compositor(|comp| {
        let fourcc = comp.format();
        let offered = comp.modifiers().len();
        // The swapchain reports the modifier SET it may allocate from, not the single
        // modifier of the live buffer. With one entry those coincide; with more, the first is
        // what smithay ranks highest. The count is logged so an ambiguous case is visible
        // instead of being reported as fact.
        let modifier = comp
            .modifiers()
            .first()
            .copied()
            .unwrap_or(smithay::backend::allocator::Modifier::Invalid);
        // Print the WHOLE offered set, not just the head. Logging only the first turned out
        // to be useless in the one case that matters: `modifier=Invalid (2 offered)` says we
        // are on the implicit/driver-negotiated path but hides what the alternative was, so it
        // cannot distinguish "the driver had a tiled option and we let it choose" from "linear
        // was the only other candidate".
        let all: Vec<String> = comp.modifiers().iter().map(|m| format!("{m:?}")).collect();
        info!(
            "scanout swapchain: fourcc={fourcc:?} modifier={modifier:?} \
             ({offered} offered: {})",
            all.join(", ")
        );
        use compositor_kernel_graphic_bridge_negotiate_classify::classify;
        compositor_model_stats_registry_base::base::set_device_format(
            "scanout",
            &format!("{fourcc:?}"),
            modifier.into(),
            classify::label(classify::classify(modifier)),
            1,
        );
    });

    // M4: enable VRR / adaptive-sync on capable outputs (controlled by `vrr`).
    // smithay sets VRR_ENABLED on the CRTC; a no-op on fixed-refresh panels. With
    // VRR active and our damage-driven scheduling, the refresh rate tracks content.
    let vrr_requested = env.vrr;
    if vrr_requested {
        let conn = display.connector.handle();
        drm_output.with_compositor(|comp| {
            let supported =
                matches!(comp.vrr_supported(conn), Ok(VrrSupport::Supported | VrrSupport::RequiresModeset));
            let enabled = if supported {
                match comp.use_vrr(true) {
                    Ok(()) => {
                        info!("native: VRR enabled");
                        true
                    }
                    Err(e) => {
                        warn!("native: VRR enable failed: {e:?}");
                        false
                    }
                }
            } else {
                info!("native: VRR not supported by this output");
                false
            };
            compositor_model_stats_registry_base::base::set_vrr(supported, enabled);
        });
    } else {
        info!("native: VRR disabled (COMPOSITOR_VRR)");
        compositor_model_stats_registry_base::base::set_vrr(false, false);
    }

    // Output + mode for the Statistics tab.
    {
        let m = display.mode;
        let mode_str = format!(
            "{}x{}@{:.2}",
            m.size.w,
            m.size.h,
            m.refresh as f32 / 1000.0
        );
        compositor_model_stats_registry_base::base::set_output(
            &display.output.name(),
            &mode_str,
        );
    }

    drop(renderer);

    let gpu_binding = Rc::new(RefCell::new(StateDRMBinding {
        gpus,
        primary: display.primary_gpu,
    }));

    info!("Init native backend OK (assemble.renderer, gles)");
    RendererAssembly {
        gpu_binding,
        drm_output_manager: Rc::new(RefCell::new(drm_output_manager)),
        drm_output,
    }
}

#[cfg(feature = "modifier-fallback")]
fn filter_formats(formats: FormatSet) -> FormatSet {
    if compositor_kernel_graphic_preference_enable_safety::safety::get().modifier_fallback {
        compositor_kernel_scanout_framebuffer_modifier_base::modifier::filter_legacy(formats)
    } else {
        formats
    }
}

#[cfg(not(feature = "modifier-fallback"))]
fn filter_formats(formats: FormatSet) -> FormatSet {
    formats
}

#[cfg(not(feature = "renderer-vulkan"))]
fn vulkan_selected(_display: &mut DisplayAssembly) -> RendererAssembly {
    abort!(
        "the vulkan renderer was selected but the backend was built without the \
         `renderer-vulkan` feature"
    );
}

/// The vulkan arm: run the full foundation self-test, then state the gap.
#[cfg(feature = "renderer-vulkan")]
fn vulkan_selected(display: &mut DisplayAssembly) -> RendererAssembly {
    let proof = vulkan_self_test(display);
    abort!(
        "vulkan renderer selected and PROVEN on this node ({proof}) — but the hosted scanout \
         aliases are GBM-typed until the scanout de-delegation generalizes the allocator \
         position; integration is pending and there is no fallback. Build without \
         `renderer-vulkan` to run gles."
    );
}

/// Exercise every vulkan crate end-to-end on the selected node. Any failure
/// panics with the failing step (the selected renderer must work).
#[cfg(feature = "renderer-vulkan")]
fn vulkan_self_test(display: &DisplayAssembly) -> String {
    use ash::vk;

    // Foundation.
    let instance = compositor_kernel_vulkan_instance_factory_base::factory::create()
        .expect("vulkan instance creation failed");
    let phd = compositor_kernel_vulkan_instance_physical_base::physical::for_node(
        &instance,
        display.primary_gpu,
    )
    .expect("vulkan physical-device enumeration failed")
    .expect("no vulkan physical device matches the selected node");
    let device = compositor_kernel_vulkan_device_factory_base::factory::create(&phd)
        .unwrap_or_else(|e| abort!("vulkan device creation failed: {e}"));
    let queue = compositor_kernel_vulkan_device_queue_base::queue::graphics_queue(&device);
    let _allocator = compositor_kernel_vulkan_memory_alloc_base::alloc::allocator(&phd)
        .unwrap_or_else(|e| abort!("vulkan allocator creation failed: {e}"));

    // Negotiated formats for the offered color set.
    let fourccs = compositor_kernel_scanout_surface_output_base::output::color_formats(false);
    let formats = compositor_kernel_vulkan_format_modifier_base::modifier::render_formats(
        &phd, &fourccs,
    );
    assert!(
        !formats.indexset().is_empty(),
        "vulkan negotiated zero render formats"
    );
    let modifiers: Vec<_> = formats.iter().map(|f| f.modifier).collect();

    // Exportable render target + composition pipelines.
    let target = compositor_kernel_vulkan_memory_export_base::export::create_exportable(
        &device,
        &phd,
        smithay::backend::allocator::Fourcc::Argb8888,
        (64, 64),
        &modifiers,
    )
    .unwrap_or_else(|e| abort!("vulkan exportable target creation failed: {e}"));

    let cache = compositor_kernel_vulkan_pipeline_cache_base::cache::create(&device)
        .unwrap_or_else(|e| abort!("vulkan pipeline cache failed: {e}"));
    let pipelines = compositor_kernel_vulkan_pipeline_composite_base::composite::create(
        &device,
        cache,
        target.format,
    )
    .unwrap_or_else(|e| abort!("vulkan composite pipeline failed: {e}"));

    // Record one composition frame: clear + a solid quad.
    let pool = compositor_kernel_vulkan_command_pool_base::pool::create(&device)
        .unwrap_or_else(|e| abort!("vulkan command pool failed: {e}"));
    let alloc_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let cmd = unsafe {
        device
            .device
            .allocate_command_buffers(&alloc_info)
            .expect("vulkan command buffer allocation failed")[0]
    };
    let solid = compositor_kernel_vulkan_element_solid_base::solid::quad(
        (64, 64),
        (8, 8, 48, 48),
        [0.0, 0.5, 1.0, 1.0],
    );
    compositor_kernel_vulkan_command_record_base::record::record_composition(
        &device,
        cmd,
        target.image,
        target.view,
        (64, 64),
        [0.0, 0.0, 0.0, 1.0],
        &pipelines,
        // Self-test: full-target clear, and no pre-pass.
        None,
        |_cmd| {},
        |cmd| {
            compositor_kernel_vulkan_element_solid_base::solid::draw(&device, &pipelines, cmd, solid);
        },
    )
    .unwrap_or_else(|e| abort!("vulkan composition recording failed: {e}"));

    // Submit with a timeline signal; wait host-side.
    let timeline = compositor_kernel_vulkan_sync_timeline_base::timeline::create(&device, 0)
        .unwrap_or_else(|e| abort!("vulkan timeline semaphore failed: {e}"));
    compositor_kernel_vulkan_device_queue_base::queue::submit_with_timeline(
        &device, &queue, cmd, timeline, 1,
    )
    .unwrap_or_else(|e| abort!("vulkan submission failed: {e}"));
    compositor_kernel_vulkan_sync_timeline_base::timeline::signal(&device, timeline, 2)
        .unwrap_or_else(|e| abort!("vulkan host timeline signal failed: {e}"));

    // Semaphore <-> syncobj round trip on the display's DRM device (the
    // Step 2 exit criterion).
    let syncobj = compositor_kernel_vulkan_sync_export_base::export::bridge_to_syncobj(
        &device,
        timeline,
        &display.drm_fd,
    )
    .unwrap_or_else(|e| abort!("vulkan->syncobj bridge failed: {e}"));
    let signalled = compositor_kernel_drm_syncobj_timeline_base::timeline::query_signalled(
        &display.drm_fd,
        syncobj,
    )
    .unwrap_or_else(|e| abort!("syncobj timeline query failed: {e}"));
    let back = compositor_kernel_vulkan_sync_timeline_base::timeline::create(&device, 0)
        .unwrap_or_else(|e| abort!("vulkan return semaphore failed: {e}"));
    compositor_kernel_vulkan_sync_import_base::import::bridge_from_syncobj(
        &device,
        back,
        &display.drm_fd,
        syncobj,
    )
    .unwrap_or_else(|e| abort!("syncobj->vulkan bridge failed: {e}"));
    compositor_kernel_drm_syncobj_device_base::device::destroy(&display.drm_fd, syncobj)
        .expect("syncobj destroy failed");

    // dmabuf export -> re-import validation (full memory loop) + textured
    // draw of the re-imported image via the element path.
    let dmabuf = compositor_kernel_vulkan_memory_export_base::export::export(&device, &target)
        .unwrap_or_else(|e| abort!("vulkan dmabuf export failed: {e}"));
    let imported =
        compositor_kernel_vulkan_memory_import_base::import::import(&device, &phd, &dmabuf)
            .unwrap_or_else(|e| abort!("vulkan dmabuf re-import failed: {e}"));

    let pool_sizes = [vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .descriptor_count(1)];
    let dp_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(&pool_sizes);
    let descriptor_pool = unsafe {
        device
            .device
            .create_descriptor_pool(&dp_info, None)
            .expect("vulkan descriptor pool failed")
    };
    let layouts = [pipelines.descriptor_layout];
    let ds_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(&layouts);
    let descriptor_set = unsafe {
        device
            .device
            .allocate_descriptor_sets(&ds_info)
            .expect("vulkan descriptor set failed")[0]
    };
    compositor_kernel_vulkan_element_texture_base::texture::bind_texture(
        &device,
        &pipelines,
        descriptor_set,
        imported.view,
    );
    let tex_quad = compositor_kernel_vulkan_element_texture_base::texture::quad(
        (64, 64),
        (0, 0, 64, 64),
        (0.0, 0.0, 1.0, 1.0),
        1.0,
    );
    let cmd2 = unsafe {
        device
            .device
            .allocate_command_buffers(&alloc_info)
            .expect("vulkan command buffer allocation failed")[0]
    };
    compositor_kernel_vulkan_command_record_base::record::record_composition(
        &device,
        cmd2,
        target.image,
        target.view,
        (64, 64),
        [0.0, 0.0, 0.0, 1.0],
        &pipelines,
        // Self-test: full-target clear, and no pre-pass.
        None,
        |_cmd| {},
        |cmd| {
            compositor_kernel_vulkan_element_texture_base::texture::draw(
                &device,
                &pipelines,
                cmd,
                descriptor_set,
                tex_quad,
            );
        },
    )
    .unwrap_or_else(|e| abort!("vulkan textured recording failed: {e}"));
    compositor_kernel_vulkan_device_queue_base::queue::submit_with_timeline(
        &device, &queue, cmd2, timeline, 3,
    )
    .unwrap_or_else(|e| abort!("vulkan textured submission failed: {e}"));

    format!(
        "device '{}', {} negotiated formats, composition + textured frames recorded and \
         submitted, timeline point bridged to syncobj (query: {signalled}), dmabuf \
         export/re-import round trip OK",
        phd.name(),
        formats.indexset().len()
    )
}

/// Exercise the reinstated de-delegation crates end-to-end against the real
/// device: property discovery -> primary plane -> swapchain -> framebuffer
/// import (cached) -> full-modeset request (+ OUT_FENCE_PTR arm and
/// IN_FENCE_FD attach where the device supports them) -> TEST_ONLY commit ->
/// page-flip request -> TEST_ONLY commit -> slot submission/aging.
#[cfg(feature = "native-scanout")]
fn native_scanout_self_test(display: &DisplayAssembly) -> String {
    use compositor_kernel_scanout_commit_build_base::build;
    use compositor_kernel_scanout_commit_submit_base::submit;
    use compositor_kernel_scanout_swapchain_acquire_base::acquire;
    use compositor_kernel_scanout_swapchain_slot_base::slot;
    use smithay::backend::allocator::{Buffer, Modifier};

    let drm_fd = &display.drm_fd;
    let res = compositor_kernel_drm_connector_scan_base::scan::resources(
        display.drm.as_ref().expect("self-test must run before the manager takes the device"),
    );

    // Pipeline property tables + the primary plane for the claimed pipe.
    let plane = build::primary_plane(drm_fd, &res, display.pipe);
    let props = build::pipeline_props(drm_fd, display.connector.handle(), display.pipe, plane);

    // Swapchain over the GL-path allocator; one slot; cached framebuffer.
    let allocator = compositor_kernel_drm_gbm_alloc_base::alloc::allocator(display.gbm.clone());
    let exporter =
        compositor_kernel_drm_gbm_alloc_base::alloc::exporter(display.gbm.clone(), display.primary_gpu);
    let (w, h) = (
        display.drm_mode.size().0 as u32,
        display.drm_mode.size().1 as u32,
    );
    let mut swapchain = slot::create(
        allocator,
        (w, h),
        smithay::backend::allocator::Fourcc::Argb8888,
        vec![Modifier::Invalid],
    );
    let buffer_slot = acquire::acquire(&mut swapchain);
    let fb = compositor_kernel_scanout_framebuffer_export_base::export::framebuffer_for(
        &exporter, drm_fd, &buffer_slot,
    );
    let _ = buffer_slot.size(); // the slot derefs to the allocator buffer
    let fb_handle = compositor_kernel_scanout_framebuffer_export_base::export::handle(&fb);

    let frame = build::PlaneFrame {
        fb: fb_handle,
        src: (w, h),
        dst: (0, 0, w, h),
    };

    // Full modeset request, fences armed where supported, kernel-validated.
    let mut req = build::build_modeset(
        drm_fd,
        display.connector.handle(),
        display.pipe,
        plane,
        &props,
        &display.drm_mode,
        frame,
    );
    let mut out_slot = compositor_kernel_scanout_fence_out_base::out::OutFenceSlot::new();
    let out_supported = compositor_kernel_scanout_fence_out_base::out::OutFenceSlot::supported(&props);
    if out_supported {
        out_slot.arm(&mut req, display.pipe, &props);
    }
    let in_supported = compositor_kernel_scanout_fence_in_base::r#in::has_in_fence(&props);
    let _held_fence; // must outlive the commit ioctl
    if in_supported {
        let syncobj = compositor_kernel_drm_syncobj_device_base::device::create(drm_fd, true)
            .expect("self-test syncobj creation failed");
        let fence = compositor_kernel_scanout_fence_in_base::r#in::from_syncobj(drm_fd, syncobj);
        compositor_kernel_scanout_fence_in_base::r#in::attach(&mut req, plane, &props, &fence);
        _held_fence = Some(fence);
        compositor_kernel_drm_syncobj_device_base::device::destroy(drm_fd, syncobj)
            .expect("self-test syncobj destroy failed");
    } else {
        _held_fence = None;
    }
    submit::test(drm_fd, req, true)
        .unwrap_or_else(|e| abort!("native modeset request failed kernel validation: {e}"));

    // Page-flip shape, kernel-validated; then slot pacing.
    let flip_req = build::build_flip(display.pipe, plane, &props, frame);
    submit::test(drm_fd, flip_req, false)
        .unwrap_or_else(|e| abort!("native flip request failed kernel validation: {e}"));
    acquire::submitted(&mut swapchain, &buffer_slot);
    let age = slot::age(&buffer_slot);

    format!(
        "primary plane {plane:?}, modeset+flip TEST_ONLY OK, slot age {age},          IN_FENCE_FD {}, OUT_FENCE_PTR {}",
        if in_supported { "attached" } else { "unsupported (nvidia-class)" },
        if out_supported { "armed" } else { "unsupported" },
    )
}

/// The contract object handed to `lifecycle::initialize` (the pre-existing
/// DisplayBackend shape) and kept as the import-capability surface for the
/// dmabuf/syncobj globals — part of the handles `wire.entry` returns to the
/// main project.
pub struct NativeContract {
    pub output: Output,
    pub mode: Mode,
    pub gpu_binding: Rc<RefCell<StateDRMBinding>>,
}

impl compositor_kernel_graphic_render_contract_base::contract::DisplayBackend for NativeContract {
    fn load(&mut self) -> (&Output, &Mode) {
        (&self.output, &self.mode)
    }

    fn bind_display(&mut self, display_handle: &DisplayHandle) -> FormatSet {
        let mut binding = self.gpu_binding.borrow_mut();
        let StateDRMBinding { gpus, primary } = &mut *binding;
        let primary = *primary;
        compositor_kernel_gles_multigpu_bind_base::bind::bind(gpus, &primary, display_handle)
    }
}

impl RenderContract for NativeContract {
    fn id(&self) -> RendererId {
        RendererId::Gles
    }

    fn bind_display(&mut self, display_handle: &DisplayHandle) -> FormatSet {
        <Self as compositor_kernel_graphic_render_contract_base::contract::DisplayBackend>::bind_display(
            self,
            display_handle,
        )
    }

    fn supported_formats(&mut self) -> FormatSet {
        let mut binding = self.gpu_binding.borrow_mut();
        let StateDRMBinding { gpus, primary } = &mut *binding;
        let primary = *primary;
        compositor_kernel_gles_multigpu_bind_base::bind::texture_formats(gpus, &primary)
    }

    fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        let mut binding = self.gpu_binding.borrow_mut();
        let StateDRMBinding { gpus, primary } = &mut *binding;
        let primary = *primary;
        compositor_kernel_gles_multigpu_bind_base::bind::import_dmabuf(gpus, &primary, dmabuf)
    }

    fn early_import(&mut self, surface: &WlSurface) {
        let mut binding = self.gpu_binding.borrow_mut();
        let StateDRMBinding { gpus, primary } = &mut *binding;
        let primary = *primary;
        compositor_kernel_gles_multigpu_bind_base::bind::early_import(gpus, &primary, surface);
    }

    fn sync_capable(&self) -> bool {
        // gles: not until EGL native fences are populated.
        false
    }

    fn export_render_fence(&mut self) -> Option<std::os::unix::io::OwnedFd> {
        // gles: implicit sync.
        None
    }
}
