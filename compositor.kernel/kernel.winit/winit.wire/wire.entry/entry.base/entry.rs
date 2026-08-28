//! The winit backend entry. (Ex winit wire.rs `wire()` + `start()`.
//! WAYLAND_DISPLAY is now set by the loader after socket creation.)

use compositor_kernel_winit_scene_compose_base::compose::WinitRenderContext;
use compositor_kernel_graphic_render_contract_base::contract::{RenderContract, RendererId};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::renderer::{ImportDma, ImportEgl};
use smithay::output::{Mode, Output};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::DisplayHandle;
use std::ffi::OsString;
use compositor_orchestration_core_state_base::Loop;

/// The winit contract object handed to `lifecycle::initialize` (the
/// pre-existing DisplayBackend shape) — ex winit state.rs `Backend` impl.
pub struct WinitContract {
    pub output: Output,
    pub mode: Mode,
    pub winit: compositor_kernel_winit_window_factory_base::factory::WinitWindow,
}

impl compositor_kernel_graphic_render_contract_base::contract::DisplayBackend for WinitContract {
    fn load(&mut self) -> (&Output, &Mode) {
        (&self.output, &self.mode)
    }

    fn bind_display(&mut self, display_handle: &DisplayHandle) -> FormatSet {
        let renderer = self.winit.winit_backend.renderer();

        if renderer.bind_wl_display(display_handle).is_ok() {
            info!("EGL Hardware Acceleration bridge initialized for clients.");
        } else {
            warn!("Clients will not be able to use Hardware Acceleration.");
        }

        renderer.dmabuf_formats()
    }
}

impl RenderContract for WinitContract {
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
        // The (fourcc x modifier) set the EGL context can sample from.
        self.winit
            .winit_backend
            .renderer()
            .egl_context()
            .dmabuf_texture_formats()
            .clone()
    }

    fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        let ok = self
            .winit
            .winit_backend
            .renderer()
            .import_dmabuf(dmabuf, None)
            .is_ok();
        trace!("winit: import_dmabuf -> {ok}");
        ok
    }

    fn early_import(&mut self, _surface: &WlSurface) {
        // Single renderer, single node: render-time import is already the
        // authoritative path; there is nothing to pre-stage.
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

pub fn wire(_loop: &mut Loop, _wayland_socket_name: OsString, event_loop: &mut EventLoop<Loop>) {
    info!("Backend initialization - Winit");
    let winit = compositor_kernel_winit_window_factory_base::factory::create()
        .expect("winit backend initialization failed");

    info!("Backend initialization - wire backend to renderer and initialize renderer");

    let output = winit.output.clone();
    let mode = winit.mode;
    let display_handle = _loop.state.output.display_handle.clone();

    let mut contract = WinitContract {
        output: output.clone(),
        mode,
        winit,
    };
    let damage_tracker = compositor_orchestration_draw_state_lifecycle::lifecycle::initialize(
        _loop,
        &output.clone(),
        &display_handle.clone(),
        &mut contract,
    );
    info!("winit: lifecycle initialized, damage tracker ready");

    let winit = contract.winit;
    // Renderer selection: `renderer` = "gles" | "vulkan". Default is VULKAN
    // (drives the scene through VulkanRenderer → dmabuf → blitted to winit's GLES
    // window); set renderer="gles" for the pure-GLES path.
    let env = compositor_model_environment_config_base::base::get();
    let vulkan_mode = !env.renderer.eq_ignore_ascii_case("gles"); // default + "vulkan" → vulkan
    // Fall back to GLES on Vulkan failure only when explicitly opted in; by
    // default a failed Vulkan init aborts (so a broken Vulkan path is loud, not
    // silently masked by GLES).
    let vulkan_fallback = env.renderer_fallback;
    info!(
        "winit: renderer = {} (gles fallback: {})",
        if vulkan_mode { "vulkan" } else { "gles" },
        vulkan_fallback
    );

    let mut context = WinitRenderContext {
        display_handle,
        output,
        winit_backend: winit.winit_backend,
        damage_tracker,
        tap_subscriptions: compositor_kernel_graphic_draw_plan_tap::tap::TapSubscriptions::new(),
        vulkan_mode,
        vulkan_fallback,
        vulkan: None,
        vulkan_target: None,
    };

    // Build the Vulkan renderer + dmabuf target now (before the event loop), not
    // lazily in the frame callback: instance/device creation is slow and must
    // not run inside the winit redraw handler.
    if context.vulkan_mode {
        trace!("winit: building VulkanRenderer + output dmabuf target for present path");
        match compositor_kernel_vulkan_renderer_core_base::renderer::VulkanRenderer::new_default(_loop.inner.kernel.get(&compositor_kernel_graphic_format_registrar_base::registrar::FORMATS).clone()) {
            Ok(vk) => {
                let sz = context.winit_backend.window_size();
                match vk.create_output_target((sz.w.max(1), sz.h.max(1))) {
                    Ok(d) => {
                        context.vulkan_target = Some((d, sz));
                        context.vulkan = Some(vk);
                        info!("winit: Vulkan present path ready ({}x{})", sz.w, sz.h);
                    }
                    Err(e) if context.vulkan_fallback => {
                        warn!("winit: vulkan target alloc failed ({e}); falling back to GLES (COMPOSITOR_RENDERER_FALLBACK)");
                        context.vulkan_mode = false;
                    }
                    Err(e) => abort!(
                        "COMPOSITOR_RENDERER=vulkan but vulkan target alloc failed: {e} \
                         (set COMPOSITOR_RENDERER_FALLBACK=1 to fall back to GLES)"
                    ),
                }
            }
            Err(e) if context.vulkan_fallback => {
                warn!("winit: VulkanRenderer init failed ({e}); falling back to GLES (COMPOSITOR_RENDERER_FALLBACK)");
                context.vulkan_mode = false;
            }
            Err(e) => abort!(
                "COMPOSITOR_RENDERER=vulkan but VulkanRenderer init failed: {e} \
                 (set COMPOSITOR_RENDERER_FALLBACK=1 to fall back to GLES)"
            ),
        }
    }

    // Record the active renderer once (after any GLES fallback). Producers skip
    // GLES-path-only resources (per-surface GlesTexture) when this is true.
    compositor_model_stats_registry_base::base::set_compositor_prefers_dmabuf(
        context.vulkan_mode,
    );
    // And WHAT it can import — the native backend's counterpart, which this path was
    // missing. The off-thread producers allocate their own dmabufs and hand them here to
    // be sampled, so they negotiate against this set and cannot ask the renderer directly
    // (it lives behind a `&mut` on this thread). Publishing nothing did not mean "no
    // constraint" to `worker_modifiers`, it meant an EMPTY intersection: the bridge saw a
    // renderer offering 0 formats against wgpu's 30, negotiated nothing, and every iced
    // and bevy surface failed to allocate. Published here, after the GLES fallback has
    // been resolved, so it names the renderer that will actually sample.
    {
        let formats = match context.vulkan.as_ref() {
            Some(vk) => smithay::backend::renderer::ImportDma::dmabuf_formats(vk),
            None => smithay::backend::renderer::ImportDma::dmabuf_formats(
                context.winit_backend.renderer(),
            ),
        };
        info!("winit: compositor-importable dmabuf pairs: {}", formats.iter().count());
        // Nested: no DRM node of our own, so this registers against
        // UNSPECIFIED. Still a real registration — what matters is that the
        // advertisement can see it.
        _loop.inner.kernel.get(&compositor_kernel_graphic_format_registrar_base::registrar::FORMATS).register(
            compositor_kernel_graphic_format_registrar_base::registrar::Device::UNSPECIFIED,
            compositor_kernel_graphic_format_role_base::role::Role::Sample,
            formats,
            "winit renderer",
        );
    }

    // WHAT NESTED DOES NOT HAVE, said out loud.
    //
    // The format layer requires every role in `Role::ALL` to be REGISTERED before
    // it will answer anything, and it is deliberately not per-question: an answer
    // is not entitled to know which roles it happens to read. Nested composites
    // into a window on someone else's compositor — there is no scanout device and
    // no KMS plane — so these three have no answer here. Registering an empty set
    // states that; it reads identically to silence at every consumer (the term is
    // dropped either way) and differs only in that the layer can now tell "there
    // is none" from "it has not arrived yet".
    {
        use compositor_kernel_graphic_format_role_base::role::Role;
        let formats = _loop.inner.kernel.get(&compositor_kernel_graphic_format_registrar_base::registrar::FORMATS);
        formats.absent(Role::ScanoutEgl, "winit (nested: no scanout device)");
        formats.absent(Role::Render, "winit (nested: nothing renders into a scanout format)");
        formats.absent(Role::Plane, "winit (nested: no KMS plane)");
    }


    event_loop
        .handle()
        .insert_source(winit.winit_loop, move |ref event, _, state| {
            compositor_kernel_winit_input_route_base::route::route(event, state, &mut context);
        })
        .unwrap();

    // Control-plane ping: same role as the native backend's, minus the DRM
    // display drains (winit has no modeset). Here it drains the one-shot lock
    // engage when pinged (`state.inner.ping_control()`), off the input path and
    // without polling per frame, so the lock keybinding engages on winit too.
    {
        let (ping, source) = smithay::reexports::calloop::ping::make_ping()
            .expect("control-plane ping creation failed");
        event_loop
            .handle()
            .insert_source(source, move |_, _, state| {
                compositor_y5_lock_interface_base::interface::drain_engage(state);
            })
            .expect("control-plane ping source registration failed");
        _loop.inner.control_ping = Some(ping);
    }
    info!("winit backend wired: event source registered, entering loop");
}
