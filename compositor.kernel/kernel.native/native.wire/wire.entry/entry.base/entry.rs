//! The native backend entry: assembles display + renderer, hooks syncobj,
//! initializes the compositor lifecycle, and registers all loop sources.
//! (Ex wire.rs `wire()` + `start()`, recomposed. WAYLAND_DISPLAY is now set
//! by the loader after socket creation — backends no longer touch process
//! environment.)
//!
//! Returns `NativeHandles` — THE integration surface for the main project:
//! `device.interface::apply` consumes the context handle for runtime
//! settings (mode changes, Law-7 enables); everything else is wired
//! internally. Assembly failures panic inside the assemble crates (crash
//! over fallback).

use compositor_kernel_native_context_render_base::render::NativeRenderContext;
use smithay::reexports::calloop::EventLoop;
use std::cell::RefCell;
use std::ffi::OsString;
use std::rc::Rc;
use compositor_orchestration_core_state_base::Loop;

/// The handles the main project integrates against (see
/// `native.device/device.interface`).
pub struct NativeHandles {
    pub ctx: Rc<RefCell<NativeRenderContext>>,
}

pub fn wire(
    _loop: &mut Loop,
    _wayland_socket_name: OsString,
    event_loop: &mut EventLoop<'static, Loop>,
) -> NativeHandles {
    info!("Backend initialization - Native");

    // ---- Display + renderer assembly (ex new()); panics internally on
    //      failure — a compositor without a display/renderer cannot run.
    trace!("native: assembling display (DRM/GBM) then renderer");
    let mut display = compositor_kernel_native_assemble_display_base::display::assemble();

    // Compile-time renderer override (the loader's `renderer-vulkan` feature
    // forwards here); preference decides otherwise. No fallback either way.
    let override_kind = if cfg!(feature = "renderer-vulkan") {
        Some(compositor_kernel_graphic_preference_renderer_rank::rank::RendererKind::Vulkan)
    } else {
        None
    };
    trace!("native: renderer override kind = {override_kind:?}");
    let renderer = compositor_kernel_native_assemble_renderer_base::renderer::assemble_with(
        &mut display,
        override_kind,
    );

    // Bind the RC cell into loop so it can register a DMABuf global.
    *_loop.inner.kernel.get_mut(&compositor_orchestration_core_state_base::state::GPU_BINDING_MUT) = Some(renderer.gpu_binding.clone());

    // logind client for lid-close suspend (None if logind is unavailable).
    *_loop
        .inner
        .kernel
        .get_mut(&compositor_orchestration_driver_logind_base::base::LOGIND_MUT) =
        match compositor_orchestration_driver_logind_base::base::LogindHandle::new() {
            Ok(h) => Some(h),
            Err(e) => {
                warn!("logind unavailable; lid-close suspend disabled: {e}");
                None
            }
        };

    // Hook syncobj impls; the support probe (`drm.syncobj/syncobj.device`)
    // records what the device can do for the explicit-sync path.
    let syncobj_eventfd =
        compositor_kernel_drm_syncobj_device_base::device::supports_eventfd(&display.drm_fd);
    info!("syncobj eventfd support: {syncobj_eventfd}");
    compositor_support_smithay_state_dmabuf_dispatch::dispatch::hook_syncobj::<compositor_support_smithay_dispatch_state_base::state::Dispatch>(
        &mut _loop.state,
        display.drm_fd.clone(),
    );

    info!("Backend initialization - wire backend to renderer and initialize renderer");

    // ---- Compositor lifecycle init through the contract (DisplayBackend shape).
    let mut contract =
        compositor_kernel_native_assemble_renderer_base::renderer::NativeContract {
            output: display.output.clone(),
            mode: display.mode,
            gpu_binding: renderer.gpu_binding.clone(),
        };
    let damage_tracker = compositor_orchestration_draw_state_lifecycle::lifecycle::initialize(
        _loop,
        &display.output.clone(),
        &_loop.inner.loader.display_handle.clone(),
        &mut contract,
    );

    // ---- Input stack (panics internally; a compositor without input cannot run).
    trace!("native: creating libinput stack for seat '{}'", display.seat_name);
    let libinput_context = compositor_kernel_input_libinput_factory_base::factory::create(
        display.session.clone(),
        &display.seat_name,
    );
    let libinput_source =
        compositor_kernel_input_loop_libinput_base::libinput::source(libinput_context.clone());

    info!("Backend initialization - Native.start()");

    // ---- The shared render context (ex start()).
    _loop.state.seat.libseat = Some(display.session.clone());

    // Renderer selection (native): `renderer` = "gles" | "vulkan", default
    // vulkan — compose via VulkanRenderer and scan out through the same
    // DrmOutput. The GLES multigpu is still used for the per-frame
    // iced/bevy/parallax prepare(). Failure aborts unless fallback is opted in.
    let env = compositor_developer_environment_config_base::base::get();
    let mut vulkan_mode = !env.renderer.eq_ignore_ascii_case("gles");
    let vulkan_fallback = env.renderer_fallback;
    // OPTION B (`local_render`): build the Vulkan renderer on the RENDER node (not the
    // default first-enumerated / scanout device), so the frame is composited on the
    // render GPU and crossed to the scanout card by the option-B present path
    // (`render.execute`: native Vulkan blit → GLES-cross fallback). Default: the
    // first-enumerated device (scanout card), byte-identical.
    let render_node = display.primary_gpu;
    let vk_new = || {
        if compositor_developer_environment_config_router::router::composite_on_render_node() {
            compositor_kernel_vulkan_renderer_core_base::renderer::VulkanRenderer::new_for_node(
                render_node,
            )
        } else {
            compositor_kernel_vulkan_renderer_core_base::renderer::VulkanRenderer::new_default()
        }
    };
    let mut vulkan = if vulkan_mode {
        match vk_new() {
            Ok(mut vk) => {
                // Hand the renderer the display's DRM fd so finish() takes the
                // KMS IN_FENCE path (render-completion sync_file via DRM syncobj)
                // instead of synchronous device_wait_idle.
                vk.set_drm_fd(display.drm_fd.clone());
                info!("native: renderer = vulkan (COMPOSITOR_RENDERER); KMS IN_FENCE enabled");
                Some(vk)
            }
            Err(e) if vulkan_fallback => {
                warn!("native: VulkanRenderer init failed ({e}); falling back to GLES (renderer_fallback)");
                vulkan_mode = false;
                None
            }
            Err(e) => abort!(
                "renderer=vulkan but VulkanRenderer init failed: {e} \
                 (set renderer_fallback=true to fall back to GLES)"
            ),
        }
    } else {
        info!("native: renderer = gles (COMPOSITOR_RENDERER)");
        None
    };

    // Record the active renderer once (after any GLES fallback). Producers skip
    // GLES-path-only resources (per-surface GlesTexture) when this is true.
    compositor_developer_stats_registry_base::base::set_compositor_prefers_dmabuf(vulkan_mode);

    // HDR (M5): opt-in via COMPOSITOR_HDR, Vulkan-only, and only on a
    // PQ-capable display. Until the full pipeline lands the path is incomplete;
    // this records capability + state for the developer tool (Statistics tab)
    // and gates later stages. SDR is the default and is untouched.
    let hdr_caps = display.hdr;
    let hdr_requested = env.hdr;
    let hdr_active = hdr_requested && hdr_caps.hdr_capable() && vulkan_mode;
    let hdr_transfer = if hdr_active {
        if hdr_caps.hdr.eotf_pq { "PQ" } else { "HLG" }
    } else {
        "SDR"
    };
    // Deep-color SDR (depth == 10) is independent of HDR: 10-bit scanout
    // with the normal sRGB transfer. Report it so the Statistics tab reflects the
    // real scanout depth.
    let deep_color = env.depth == 10;
    let color_format = if hdr_active {
        "10-bit PQ (BT.2020)"
    } else if deep_color {
        "10-bit sRGB"
    } else {
        "8-bit sRGB"
    };
    info!(
        "native HDR: requested={hdr_requested} capable={} active={hdr_active} deep_color={deep_color}",
        hdr_caps.hdr_capable()
    );
    // Switch the Vulkan renderer to the HDR composite path when active.
    if let Some(vk) = vulkan.as_mut() {
        vk.set_hdr_enabled(hdr_active);
    }
    compositor_developer_stats_registry_base::base::set_hdr_info(
        hdr_active,
        hdr_caps.hdr_capable(),
        hdr_transfer,
        hdr_caps.hdr.max_luminance.unwrap_or(0.0),
        hdr_caps.colorimetry.bt2020_rgb,
        color_format,
    );

    // The KMS card (device) the compositor scans out on — resolved from the scanout
    // card PATH so it is a CARD node whose `dev_id` matches the udev card enumeration
    // (the RENDER node has a DIFFERENT `dev_id`, so using it here would make
    // `assemble.secondary` fail to skip the primary card and re-open it). This is the
    // `dev_id` key shared by `OutputPipe::device`, `DeviceRender::node`, and vblank
    // routing. `device_path` is the scanout card for both desktop and split.
    let card_node = smithay::backend::drm::DrmNode::from_path(&display.device_path)
        .expect("scanout card path resolves to a DRM node");

    // OPTION B APPROACH B: a second Vulkan renderer on the SCANOUT card, used to blit
    // the render-node offscreen into the scanout buffer (a GPU untile). `None` disables
    // approach B → the frame path falls back to the GLES-cross copy (approach A).
    let vulkan_scanout = if vulkan_mode
        && compositor_developer_environment_config_router::router::composite_on_render_node()
    {
        match compositor_kernel_vulkan_renderer_core_base::renderer::VulkanRenderer::new_for_node(
            card_node,
        ) {
            Ok(mut vk) => {
                vk.set_drm_fd(display.drm_fd.clone());
                info!("option B (approach B): scanout-card Vulkan renderer ready ({card_node:?})");
                Some(vk)
            }
            Err(e) => {
                warn!(
                    "option B: scanout-card Vulkan init failed ({e}); approach B disabled — using \
                     the GLES-cross copy (approach A)"
                );
                None
            }
        }
    } else {
        None
    };

    let ctx_rc = Rc::new(RefCell::new(NativeRenderContext {
        display_handle: _loop.inner.loader.display_handle.clone(),
        outputs: vec![compositor_kernel_native_context_render_base::render::OutputPipe {
            device: card_node,
            crtc: display.pipe,
            mode: display.mode,
            output: display.output.clone(),
            damage_tracker,
            drm_output: Some(renderer.drm_output),
            hdr_caps,
            hdr_active,
            hdr_signalled: false,
            connector: display.connector.handle(),
            current_drm_mode: display.drm_mode,
            modes: display.connector.modes().to_vec(),
            mode_revert: None,
            global: None,
            vk_offscreen: None,
            in_flight: false,
        }],
        devices: vec![compositor_kernel_native_context_render_base::render::DeviceRender {
            node: card_node,
            drm_fd: display.drm_fd.clone(),
            drm_output_manager: renderer.drm_output_manager,
            vblank_token: None,
        }],
        gpu_binding: renderer.gpu_binding.clone(),
        libinput_context,
        tap_subscriptions: compositor_kernel_graphic_draw_plan_tap::tap::TapSubscriptions::new(),
        safety: compositor_kernel_graphic_preference_enable_safety::safety::get(),
        vulkan_mode,
        vulkan,
        vulkan_scanout,
        dark_tick: None,
    }));

    // ---- Advertised-mode snapshot for the settings Display panel (kernel → rim).
    //      The UI reads OUTPUT_MODES_SNAPSHOT directly. mHz = vrefresh*1000.
    {
        use compositor_orchestration_driver_output_base::base::{ModeInfo, OutputModesSnapshot};
        let to_info = |m: &smithay::reexports::drm::control::Mode| ModeInfo {
            width: m.size().0,
            height: m.size().1,
            refresh_mhz: m.vrefresh() * 1000,
        };
        *_loop
            .inner
            .kernel
            .get_mut(&compositor_orchestration_driver_output_base::base::OUTPUT_MODES_SNAPSHOT_MUT) =
            OutputModesSnapshot {
                // EDID identity "make model serial" — the per-monitor key the picker
                // selects with and the settings-editor persists.
                edid_key: display.identity.key(),
                current: Some(to_info(&display.drm_mode)),
                available: display.connector.modes().iter().map(to_info).collect(),
            };
    }

    // ---- Topology bookkeeping for the device authority.
    let registry = Rc::new(RefCell::new(
        compositor_kernel_gpu_registry_node_base::node::NodeRegistry::new(),
    ));
    {
        let mut reg = registry.borrow_mut();
        reg.add(display.primary_gpu.dev_id(), display.primary_gpu);
        reg.set_primary(display.primary_gpu);
    }
    let topology = Rc::new(RefCell::new(
        compositor_kernel_native_context_topology_base::topology::Topology::new(),
    ));
    {
        let mut topo = topology.borrow_mut();
        let dev_id = display.primary_gpu.dev_id();
        topo.register_device(dev_id, display.primary_gpu);
        topo.register_connector(
            dev_id,
            compositor_kernel_native_context_topology_base::topology::ConnectorEntry {
                handle: display.connector.handle(),
                kind: compositor_kernel_drm_connector_kind_base::kind::classify(&display.connector),
                pipe: Some(display.pipe),
                output: Some(display.output.clone()),
            },
        );
        // The hotplug diff baseline: the full connector state at assembly.
        topo.set_snapshot(dev_id, display.initial_snapshot.clone());
    }

    // ---- Initial display snapshot for the lid policy (external present? is the
    //      active output the internal panel?). Refreshed on hotplug by wire.plugin.
    {
        let active = display.connector.handle();
        let ctx = ctx_rc.borrow();
        let manager = ctx.primary_manager().borrow();
        let snap = compositor_kernel_native_context_display_base::base::compute(
            manager.device(),
            active,
        );
        drop(manager);
        drop(ctx);
        *_loop
            .inner
            .kernel
            .get_mut(&compositor_orchestration_driver_lid_base::base::DISPLAY_SNAPSHOT_MUT) = snap;
    }

    // ---- Full connected-monitor list for the settings preferred-monitor picker
    //      (kernel → rim). Lists the active connector plus connected-but-inactive
    //      monitors; refreshed on a live switch by `display.reconcile`.
    {
        use compositor_orchestration_driver_output_base::base::ModeInfo;
        let active_mode = ModeInfo {
            width: display.drm_mode.size().0,
            height: display.drm_mode.size().1,
            refresh_mhz: display.drm_mode.vrefresh() * 1000,
        };
        let ctx = ctx_rc.borrow();
        let manager = ctx.primary_manager().borrow();
        // Only the primary pipe exists at boot; secondaries are added by the hotplug
        // reconcile, which rewrites this snapshot with every lit pipe's current mode.
        let lit = [(display.connector.handle(), active_mode)];
        let snap = compositor_kernel_native_context_display_enumerate::enumerate::enumerate(
            manager.device(),
            display.connector.handle(),
            &lit,
        );
        drop(manager);
        drop(ctx);
        *_loop
            .inner
            .kernel
            .get_mut(&compositor_orchestration_driver_output_base::base::OUTPUTS_SNAPSHOT_MUT) = snap;
    }

    // ---- Loop sources.
    compositor_kernel_native_wire_session_base::session::register(
        event_loop,
        display.session_notifier,
        ctx_rc.clone(),
        |ctx, state| {
            // The watchdog's kick is the frame executor; the loop handle it
            // captures is the state's own. Outcome handling belongs to the
            // pacing layer — the watchdog only needs the kick.
            let handle = state.loop_handle.clone();
            let _ = compositor_kernel_native_render_execute_base::execute::execute(
                ctx, handle, state,
                compositor_kernel_native_render_execute_base::execute::RenderScope::All,
            );
        },
    );

    // Register this card's vblank source, keyed by its node so routing is correct
    // once a second card is added. Store the token on the DeviceRender so the source
    // can be torn down if the card is removed.
    let vblank_token = compositor_kernel_native_wire_frame_base::frame::register(
        event_loop,
        _loop,
        display.drm_notifier,
        card_node,
        ctx_rc.clone(),
    );
    if let Some(dev) = ctx_rc.borrow_mut().device_mut(&card_node) {
        dev.vblank_token = Some(vblank_token);
    }

    // Multi-GPU: open + drive every OTHER KMS card (each composites its own monitors
    // locally). ADDITIVE — a single-card system finds none and this is a no-op, so the
    // primary path above is untouched. Runtime-unverified without a real 2nd card.
    compositor_kernel_native_assemble_secondary_base::secondary::open_secondary_devices(
        &mut display.session,
        &display.seat_name,
        card_node.dev_id(),
        event_loop,
        _loop,
        &ctx_rc,
    );

    compositor_kernel_native_wire_input_base::input::register(
        event_loop,
        libinput_source,
        ctx_rc.clone(),
    );

    // Control-plane ping: drains the input-independent control-plane OFF the
    // libinput source, on its own loop iteration when pinged (via
    // `state.inner.ping_control()`) — the display request queues (output mode /
    // preferred-monitor switch / lid apply) and the one-shot lock engage. So a
    // settings-window mode change, a lid action, or a lock keybinding all apply
    // without waiting for the next input event, and none of them poll per frame.
    {
        let (ping, source) = smithay::reexports::calloop::ping::make_ping()
            .expect("control-plane ping creation failed");
        let ctx = ctx_rc.clone();
        event_loop
            .handle()
            .insert_source(source, move |_, _, state| {
                compositor_kernel_native_context_display_apply::apply::drain(state, &ctx);
                compositor_kernel_native_context_display_mode::mode::drain(state, &ctx);
                // Multi-output branch: the single-output active-switch transaction was
                // replaced by the set-reconciler (activate/deactivate/hotplug).
                compositor_kernel_native_context_display_reconcile::reconcile::drain_reconcile(state, &ctx);
                compositor_y5_lock_interface_base::interface::drain_engage(state);
            })
            .expect("control-plane ping source registration failed");
        _loop.inner.control_ping = Some(ping);
    }

    // Retained udev watch (panics internally if udev vanished post-snapshot).
    let watch = compositor_kernel_udev_loop_watch_base::watch::watch(&display.seat_name);
    compositor_kernel_native_wire_plugin_base::plugin::register(
        event_loop,
        watch,
        registry,
        topology,
        ctx_rc.clone(),
    );

    // Light up every OTHER connected monitor as an additional output (the primary
    // is already up from assembly). The set-reconciler adds one pipe per connected
    // connector not yet driven — the same path a runtime hotplug takes.
    compositor_kernel_native_context_display_reconcile::reconcile::reconcile(_loop, &ctx_rc);

    NativeHandles { ctx: ctx_rc }
}
