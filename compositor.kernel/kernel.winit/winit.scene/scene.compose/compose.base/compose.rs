//! The winit frame path. (Ex winit draw.scene/scene.rs, Phase 3 convergence:
//! pass presence comes from the frame plan, callbacks/housekeeping from
//! `compositor_kernel_graphic_draw_present_callbacks` — the duplicated refresh() is gone.)

use compositor_kernel_graphic_draw_plan_frame::frame::{plan, FramePass};
use compositor_kernel_graphic_draw_plan_tap::tap::{TapSubscriptions, POST_SCENE};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::{Element, RenderElement};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{Color32F, Frame, ImportDma, Renderer};
use smithay::backend::winit::WinitGraphicsBackend;
use smithay::desktop::Window;
use smithay::output::Output;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::utils::{Physical, Rectangle, Scale, Size, Transform};
use compositor_kernel_vulkan_renderer_core_base::renderer::VulkanRenderer;
use compositor_orchestration_draw_scene_frame::scene::Scene;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_graphic_capture_registry::{CaptureRegistry, OutputId};

/// The winit render context (ex winit.draw/draw.context, folded into the
/// scene composer that owns it).
pub struct WinitRenderContext {
    pub display_handle: DisplayHandle,
    pub output: Output,
    pub winit_backend: WinitGraphicsBackend<GlesRenderer>,
    pub damage_tracker: OutputDamageTracker,
    /// Law 5: taps fire only for active subscribers; registry presence IS the
    /// subscription (set when the capture registry initializes).
    pub tap_subscriptions: TapSubscriptions,

    /// Env-gated (`COMPOSITOR_RENDERER=vulkan`): drive the scene through the new
    /// `VulkanRenderer` instead of winit's GLES renderer. The scene is composed
    /// generically, rendered by Vulkan into `vulkan_target` (a dmabuf), then
    /// imported into winit's GLES context and blitted to the window (winit owns
    /// a GLES swapchain, so Vulkan output reaches the screen via dmabuf interop).
    pub vulkan_mode: bool,
    /// When Vulkan fails, fall back to GLES instead of aborting. Off by default
    /// (a failed `COMPOSITOR_RENDERER=vulkan` is a hard error); enable with
    /// `COMPOSITOR_RENDERER_FALLBACK=1` (or `gles`/`true`).
    pub vulkan_fallback: bool,
    /// The Vulkan renderer (built at wire time when vulkan_mode).
    pub vulkan: Option<VulkanRenderer>,
    /// Output-sized dmabuf target + its size; recreated on resize.
    pub vulkan_target: Option<(Dmabuf, Size<i32, Physical>)>,
}

pub fn draw(state: &mut Loop, context: &mut WinitRenderContext) {
    let (damage, visible) = compose(context, state);
    // Submits "damage" to backend refreshing necessary parts only. Depends on
    // individual elements implementations.
    compositor_kernel_winit_frame_submit_base::submit::submit(&mut context.winit_backend, damage);
    present(context, state, visible);
}

fn compose(
    context: &mut WinitRenderContext,
    state: &mut Loop,
) -> (Rectangle<i32, Physical>, Vec<Window>) {
    let monitor_size = context.winit_backend.window_size();
    let damage = Rectangle::from_size(monitor_size);
    // The winit output carries Transform::Flipped180 (winit/EGL framebuffers are
    // Y-flipped vs. DRM). The main scene's damage tracker reads this off the
    // output automatically; the manual picker/lock renders below must apply it
    // explicitly or they come out upside down relative to the rest of the frame.
    let output_transform = context.output.current_transform();

    // Stable capture id for this output — the SAME EDID-derived `OutputId::from_key`
    // the rim's capture requests use (they key off `active_output()` on every
    // backend), so entries match on winit too. A hardcoded `OutputId(0)` here never
    // matched the request's keyed id → capture silently produced nothing.
    let capture_output = OutputId::from_key(
        &compositor_orchestration_core_state_base::state::output_key(&context.output),
    );

    let (gles_renderer, mut gles_framebuffer) = context.winit_backend.bind().unwrap();

    // The capture registry is pre-created at startup (loader prewarm) from the
    // shared bevy context — never built mid-render. Its tap subscription lives on
    // this backend's render context (created during render), so subscribe exactly
    // once here: registry presence IS the tap (Law 5).
    if state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY).is_some()
        && !context.tap_subscriptions.is_active(POST_SCENE)
    {
        context.tap_subscriptions.subscribe(POST_SCENE);
        info!("winit: POST_SCENE tap subscribed");
    }

    // Phase 3 convergence: the compositor decides what this frame contains.
    let picker_active =
        state.inner.worlds.active_id() == compositor_y5_picker_system_base::base::PICKER_WORLD;
    let frame_plan = plan(&state.inner.status, picker_active);
    let render_scene = frame_plan.has_pass(FramePass::Scene);
    let render_lock = frame_plan.has_pass(FramePass::Lock);
    let render_picker = frame_plan.has_pass(FramePass::Picker);
    let tap_post_scene =
        frame_plan.has_tap(POST_SCENE) && context.tap_subscriptions.is_active(POST_SCENE);

    let mut visible_window: Vec<Window> = Vec::new();
    if render_scene {
        // GLES prepare phase (builds iced/bevy/parallax resources) — always on
        // the winit GlesRenderer, regardless of which renderer composes.
        let prepared =
            compositor_orchestration_draw_scene_frame::scene::prepare(state, gles_renderer, monitor_size);

        // In Vulkan mode the GLES renderer only runs prepare() and never renders
        // a frame, so its deferred texture/EGLImage destruction never drains and
        // leaks GPU memory. Drain it explicitly (harmless in GLES mode, where the
        // renderer's own render() does this). Mirrors the native path fix.
        if context.vulkan_mode {
            use smithay::backend::renderer::Renderer;
            let _ = gles_renderer.cleanup_texture_cache();
        }

        // The VulkanRenderer + target are created at wire time. Only reallocate
        // the dmabuf target here on resize.
        if context.vulkan_mode {
            let need_target = context
                .vulkan_target
                .as_ref()
                .map(|(_, s)| *s != monitor_size)
                .unwrap_or(true);
            if need_target {
                if let Some(vk) = context.vulkan.as_ref() {
                    match vk.create_output_target((monitor_size.w.max(1), monitor_size.h.max(1))) {
                        Ok(d) => context.vulkan_target = Some((d, monitor_size)),
                        Err(e) if context.vulkan_fallback => {
                            warn!("winit: vulkan target realloc failed ({e}); falling back to GLES (COMPOSITOR_RENDERER_FALLBACK)");
                            context.vulkan_mode = false;
                            // Runtime fallback to GLES: re-enable GLES-path resources.
                            compositor_developer_stats_registry_base::base::set_compositor_prefers_dmabuf(false);
                        }
                        Err(e) => abort!(
                            "vulkan target realloc failed on resize: {e} \
                             (set COMPOSITOR_RENDERER_FALLBACK=1 to fall back to GLES)"
                        ),
                    }
                }
            }
        }

        if context.vulkan_mode {
            // --- Vulkan present path ---
            let vk = context.vulkan.as_mut().unwrap();
            let scene = compositor_orchestration_draw_scene_frame::scene::scene::<VulkanRenderer>(
                state,
                vk,
                monitor_size,
                prepared,
            );
            visible_window = scene.visible_window;

            let (dmabuf, _) = context.vulkan_target.as_mut().unwrap();
            let full = Rectangle::<i32, Physical>::from_loc_and_size((0, 0), monitor_size);

            // Render the scene into the dmabuf via Vulkan; finish() returns the
            // render-completion SyncPoint (a real sync_file fence in the default
            // async path; already-signaled in legacy). The GLES blit below waits
            // on it before sampling.
            // Diagnostic gate for the Vulkan present path (set Y5_VK_DIAG):
            //   "vk"   – Vulkan clears the dmabuf to solid MAGENTA and draws no
            //            scene elements. If the window shows magenta, the whole
            //            Vulkan→dmabuf→GLES-import→blit chain works and the
            //            problem is the scene drawing. If black, the dmabuf
            //            interop or blit is broken.
            //   "blit" – the GLES blit clears solid GREEN and does NOT sample the
            //            Vulkan texture. If green shows, the GLES blit/present
            //            itself is fine (so a black "vk"/normal run means the
            //            Vulkan-rendered dmabuf samples black = interop broken).
            let diag = compositor_developer_environment_config_base::base::get()
                .vk_diag
                .as_str();
            if !diag.is_empty() {
                trace!("winit: vk_diag={diag}");
            }

            let mut vk_sync: Option<smithay::backend::renderer::sync::SyncPoint> = None;
            match smithay::backend::renderer::Bind::bind(vk, dmabuf) {
                Ok(mut vk_fb) => {
                    if let Ok(mut frame) = vk.render(&mut vk_fb, monitor_size, Transform::Normal) {
                        let clear = if diag == "vk" {
                            Color32F::new(1.0, 0.0, 1.0, 1.0) // magenta
                        } else {
                            Color32F::new(0.1, 0.1, 0.1, 1.0)
                        };
                        let _ = frame.clear(clear, &[full]);
                        if diag != "vk" {
                            let scale = Scale::from(1.0);
                            // Scene elements are front-to-back (smithay
                            // convention); draw back-to-front so the background
                            // sits underneath.
                            for element in scene.Element.iter().rev() {
                                let _ = element.draw(
                                    &mut frame,
                                    element.src(),
                                    element.geometry(scale),
                                    &[full],
                                    element.opaque_regions(scale).iter().as_slice(),
                                    None,
                                );
                            }
                        }
                        if let Ok(sp) = frame.finish() {
                            vk_sync = Some(sp);
                        }
                    }
                }
                Err(e) => error!("winit: vulkan bind target failed: {e}"),
            }

            // Import the Vulkan-rendered dmabuf into winit's GLES context and
            // blit it full-screen to the window (winit presents via GLES).
            match gles_renderer.import_dmabuf(dmabuf, None) {
                Ok(tex) => {
                    let src = Rectangle::<f64, smithay::utils::Buffer>::from_loc_and_size(
                        (0.0, 0.0),
                        (monitor_size.w as f64, monitor_size.h as f64),
                    );
                    // Wait on the Vulkan render fence BEFORE starting the GLES
                    // frame (a CPU wait on the sync_file; instant for legacy's
                    // already-signaled point). Doing this inside the frame via
                    // Frame::wait re-runs make_current and corrupts the in-
                    // progress EGL window surface (BAD_SURFACE → BadAlloc).
                    if let Some(ref sp) = vk_sync {
                        let _ = sp.wait();
                    }
                    if let Ok(mut frame) =
                        gles_renderer.render(&mut gles_framebuffer, monitor_size, Transform::Normal)
                    {
                        let blit_clear = if diag == "blit" {
                            Color32F::new(0.0, 1.0, 0.0, 1.0) // green
                        } else {
                            Color32F::new(0.0, 0.0, 0.0, 1.0)
                        };
                        let _ = frame.clear(blit_clear, &[full]);
                        if diag != "blit" {
                            let _ = Frame::render_texture_from_to(
                                &mut frame,
                                &tex,
                                src,
                                full,
                                &[full],
                                &[],
                                Transform::Flipped180,
                                1.0,
                            );
                        }
                        let _ = frame.finish();
                    }
                }
                Err(e) => error!("winit: GLES import of vulkan dmabuf failed: {e}"),
            }

            // Post-scene capture tap (Vulkan winit path).
            // - Window / world-region capture: render the captured windows
            //   directly into the entry with the Vulkan renderer (which holds
            //   their buffers), so off-screen windows are captured and no chrome
            //   leaks in.
            // - Screen-region / full-screen capture: blit the just-presented
            //   window framebuffer (it's screen-space by definition).
            if tap_post_scene {
                if let Some(job) =
                    compositor_y5_graphic_capture_interface::render::window_render_job(state)
                {
                    let backdrop =
                        compositor_y5_graphic_capture_interface::render::capture_backdrop(
                            state, &job,
                        );
                    let dmabuf = state
                        .inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY)
                        .as_ref()
                        .and_then(|r| r.entry_dmabuf(job.entry_id));
                    if let (Some(mut dmabuf), Some(vk)) = (dmabuf, context.vulkan.as_mut()) {
                        compositor_y5_graphic_capture_interface::render::draw_windows_into_bg(
                            vk,
                            &mut dmabuf,
                            job.size,
                            &job.windows,
                            job.scale,
                            backdrop,
                        );
                    }
                } else if let Some(registry) = &mut state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY) {
                    registry.tick(
                        &state.inner.environment.GPU.as_str(),
                        gles_renderer,
                        capture_output,
                        &gles_framebuffer,
                        monitor_size,
                    );
                }
            }
        } else {
            // --- GLES present path (default) ---
            let scene = compositor_orchestration_draw_scene_frame::scene::scene::<GlesRenderer>(
                state,
                gles_renderer,
                monitor_size,
                prepared,
            );
            visible_window = scene.visible_window;

            context
                .damage_tracker
                .render_output(
                    gles_renderer,
                    &mut gles_framebuffer,
                    0,
                    &scene.Element,
                    [0.1, 0.1, 0.1, 1.0],
                )
                .unwrap();

            // Post-scene tap (GLES winit path). Window/world targets render
            // their windows into the entry (off-screen capable, chrome-free);
            // screen/full-screen targets blit the framebuffer.
            if tap_post_scene {
                if let Some(job) =
                    compositor_y5_graphic_capture_interface::render::window_render_job(state)
                {
                    let backdrop =
                        compositor_y5_graphic_capture_interface::render::capture_backdrop(
                            state, &job,
                        );
                    if let Some(mut dmabuf) = state
                        .inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY)
                        .as_ref()
                        .and_then(|r| r.entry_dmabuf(job.entry_id))
                    {
                        compositor_y5_graphic_capture_interface::render::draw_windows_into_bg(
                            gles_renderer,
                            &mut dmabuf,
                            job.size,
                            &job.windows,
                            job.scale,
                            backdrop,
                        );
                    }
                } else if let Some(registry) = &mut state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY) {
                    registry.tick(
                        &state.inner.environment.GPU.as_str(),
                        gles_renderer,
                        capture_output,
                        &gles_framebuffer,
                        monitor_size,
                    );
                }
            }
        }
    }

    // World-selection screen. Render the bevy sphere-of-cells through the GLES
    // window in either render mode (the bevy instance renders to GLES textures
    // regardless of the compositing renderer, and winit always presents via
    // GLES), clearing to a dark backdrop behind it.
    if render_picker {
        // Advance an in-flight video capture: the scene `per_frame` encoder pump
        // doesn't run while the picker owns the frame, so drive it here (the tap
        // below refreshes the capture entry with the picker each frame).
        compositor_y5_graphic_capture_interface::interface::overlay_per_frame(state);
        let prepared =
            compositor_y5_picker_scene_frame::frame::prepare(state, gles_renderer, monitor_size);
        let scene = compositor_y5_picker_scene_frame::frame::scene::<GlesRenderer>(
            state,
            gles_renderer,
            monitor_size,
            prepared,
        );
        // Render manually (full clear + draw), NOT via the shared damage tracker:
        // it's shared with the main scene, so its stale damage history leaves
        // uncleared regions when switching worlds (white flicker). Same reason
        // lock renders manually below. Elements are front-to-back, so draw rev.
        let mut frame = gles_renderer
            .render(&mut gles_framebuffer, monitor_size, output_transform)
            .unwrap();
        let full = Rectangle::<i32, Physical>::from_loc_and_size((0, 0), monitor_size);
        let scale = Scale::from(1.0);
        let _ = frame.clear([0.04, 0.05, 0.10, 1.0].into(), &[full]);
        for element in scene.Element.iter().rev() {
            let _ = element.draw(
                &mut frame,
                element.src(),
                element.geometry(scale),
                &[full],
                element.opaque_regions(scale).iter().as_slice(),
                None,
            );
        }
        frame.finish().unwrap();

        // Post-picker capture tap: keep an in-flight capture recording the
        // world-picker overlay while the user navigates worlds. Window/world-
        // region captures render their windows into the entry; screen/full-screen
        // captures blit the just-rendered picker framebuffer.
        if tap_post_scene {
            if let Some(job) =
                compositor_y5_graphic_capture_interface::render::window_render_job(state)
            {
                let backdrop =
                    compositor_y5_graphic_capture_interface::render::capture_backdrop(state, &job);
                if let Some(mut dmabuf) = state
                    .inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY)
                    .as_ref()
                    .and_then(|r| r.entry_dmabuf(job.entry_id))
                {
                    compositor_y5_graphic_capture_interface::render::draw_windows_into_bg(
                        gles_renderer,
                        &mut dmabuf,
                        job.size,
                        &job.windows,
                        job.scale,
                        backdrop,
                    );
                }
            } else if let Some(registry) = &mut state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY) {
                registry.tick(
                    &state.inner.environment.GPU.as_str(),
                    gles_renderer,
                    capture_output,
                    &gles_framebuffer,
                    monitor_size,
                );
            }
        }
    }

    let _scene_lock = if render_lock {
        // Render lock manually without damage handling.
        let lock_prepared =
            compositor_y5_lock_scene_frame::frame::prepare(state, gles_renderer, monitor_size);
        let scene = compositor_y5_lock_scene_frame::frame::scene(
            state,
            gles_renderer,
            monitor_size,
            lock_prepared,
        );

        // CHECK carried: former frames are not cleared when the previous scene
        // is no longer rendered, and the capture registry seems to keep
        // capturing after its handle drops.
        if render_scene {
            let mut frame = gles_renderer
                .render(&mut gles_framebuffer, monitor_size, output_transform)
                .unwrap();

            // Damage full screen.
            let full = Rectangle::<i32, Physical>::from_loc_and_size((0, 0), monitor_size);

            let scale = Scale::from(1.0);
            for element in &scene.Element {
                element
                    .draw(
                        &mut frame,
                        element.src(),
                        // Scale for lock scene shouldn't matter. It is usually
                        // the effect of zoom, etc.
                        element.geometry(scale),
                        &[full],
                        element.opaque_regions(scale).iter().as_slice(),
                        // CHECK carried: what's expected here.
                        None,
                    )
                    .unwrap();
            }
            frame.finish().unwrap();
        } else {
            context
                .damage_tracker
                .render_output(
                    gles_renderer,
                    &mut gles_framebuffer,
                    0,
                    &scene.Element,
                    [0.1, 0.1, 0.1, 1.0],
                )
                .unwrap();
        }

        Some(scene)
    } else {
        None
    };

    (damage, visible_window)
}

/// Frame callbacks + housekeeping via the shared compositor crates, then ask
/// winit for the next redraw (winit has no hardware page-flip; presentation
/// is immediate).
fn present(context: &mut WinitRenderContext, state: &mut Loop, visible: Vec<Window>) {
    compositor_kernel_graphic_draw_present_callbacks::callbacks::send_window_frames(
        state,
        &context.output,
        &visible,
    );
    compositor_kernel_graphic_draw_present_callbacks::callbacks::send_layer_frames(state, &context.output);
    compositor_kernel_graphic_draw_present_callbacks::callbacks::housekeeping(state);

    compositor_kernel_winit_frame_submit_base::submit::request_redraw(&mut context.winit_backend);
}
