#![allow(irrefutable_let_patterns)]
pub mod activation_env;
mod event_loop;
mod wayland;
pub mod wgpu;

use compositor_developer_debug_instance_record::{info, trace, warn};

use crate::wgpu::initialize_wgpu_context;
use smithay::reexports::calloop::channel as cl_channel;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, Mode, PostAction};
use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
use std::sync::{Arc, mpsc};
use std::time::Instant;
use compositor_introspection_extraction_window_base::default_registry;
use compositor_introspection_sampler_window_base::sampler::{SampleBatch, SampleResult, Sampler};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::{Loader, Orchestrator as State};
use compositor_support_smithay_dispatch_wire_trait::wire_trait::WireTrait;
// App-launch executor (kernel.execution driver) — all worker/reaper/channel
// wiring is encapsulated behind `block_sigchld` + `install`.
use compositor_kernel_execution_driver_executor_install::install as launch_executor;

/// The shipped version, baked in at COMPILE time from the repo-root `VERSION` file —
/// the single human-owned constant (`MAJOR.MINOR.PATCH`). `include_str!` makes cargo
/// track that file, so changing it forces a rebuild and the embedded number can never
/// drift from the released artifact. CI overwrites this file (never committed) with the
/// auto-incremented patch before building; a plain local build embeds whatever it holds.
/// See ci/scripts/version.sh for how the patch is derived.
pub const VERSION: &str = include_str!("../../../../../VERSION").trim_ascii();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // FIRST: parse the single COMPOSITOR_ENVIRONMENT JSON into the process-global
    // config. This must run before anything else — including logging, which reads
    // `log_level` from it — and panics immediately if the var is unset or any
    // required field is missing/malformed. It is the ONLY place env config is read.
    compositor_developer_environment_config_base::base::init();

    // Aggregate the opt-in experimental `gpu_*` flags (experimental.json). Lenient:
    // a missing/invalid file leaves the flag set empty, so defaults are unchanged.
    // Read here, right after config; unrecognized flags are warned once below.
    compositor_developer_environment_experimental_base::base::init();

    // Block SIGCHLD before any thread spawns, so the launch reaper's signalfd is
    // the sole consumer (no-op under the Direct backend).
    launch_executor::block_sigchld();

    let environment = compositor_orchestration_environment_type_base::base::Get();
    compositor_support_library_debug_client_base::init_logging();

    // Start the developer logging process (fan-in buffer + drain/print + gRPC stream).
    // Levels come from COMPOSITOR_LOG_LEVEL.
    compositor_developer_log_process_main::spawn();
    info!("y5_compositor {VERSION}");

    // Now that logging is up, surface the experimental flag state (the crate itself
    // has no logging dep, so it can't warn at parse time).
    {
        use compositor_developer_environment_experimental_base::base as experimental;
        let gpu_flags = experimental::get();
        if !gpu_flags.is_empty() {
            info!("experimental gpu flags active: {gpu_flags:?}");
        }
        if gpu_flags.contains(experimental::GpuFlags::NEGOTIATE_FORMATS) {
            // Reserved: the bridge's wgpu engine format is fixed (Bgra8UnormSrgb) and
            // the content is alpha-blended, so Argb8888 is the only correct fourcc.
            warn!("gpu_negotiate_formats has no effect: the bridge render format is fixed");
        }
        for f in experimental::unknown() {
            warn!("ignoring unrecognized experimental flag: {f:?}");
        }
    }

    // Arm the persistence engine (spawn its writer thread) before any world flushes.
    compositor_support_system_persist_engine_base::base::init();
    info!(
        "developer log online: gpu={} desktop={}",
        environment.GPU, environment.DesktopName
    );

    info!("Environment: {:?}", environment);

    // Record the renderer/sync-relevant config for the developer Statistics tab.
    // Built from the parsed config (not the environment), keeping the COMPOSITOR_*
    // display names the viewer already shows.
    let e = compositor_developer_environment_config_base::base::get();
    let env_flags: Vec<(String, String)> = vec![
        ("COMPOSITOR_RENDERER".to_string(), e.renderer.clone()),
        ("COMPOSITOR_RENDERER_SYNC".to_string(), e.renderer_sync.clone()),
        ("COMPOSITOR_RENDERER_FALLBACK".to_string(), e.renderer_fallback.to_string()),
        ("COMPOSITOR_HDR".to_string(), e.hdr.to_string()),
        ("COMPOSITOR_DEPTH".to_string(), e.depth.to_string()),
        ("COMPOSITOR_VRR".to_string(), e.vrr.to_string()),
        ("COMPOSITOR_RENDER_NODE".to_string(), e.render_node.clone()),
        ("COMPOSITOR_LOG_LEVEL".to_string(), e.log_level.clone()),
        (
            "EXPERIMENTAL_GPU_FLAGS".to_string(),
            format!("{:?}", compositor_developer_environment_experimental_base::base::get()),
        ),
    ];
    compositor_developer_stats_registry_base::base::set_env_flags(env_flags);

    info!("Create an event loop");
    // Creates Smithay event loop
    let (mut event_loop, display) = event_loop::create()?;

    info!("Create the wayland socket");
    // Create a wayland socket
    let wayland_socket = wayland::create_socket();
    // let wayland_socket_proprietary = wayland::create_socket_proprietary();

    info!(
        "Create the wayland socket {:?}",
        wayland_socket.name.clone(),
        // wayland_socket_proprietary.name.clone()
    );

    // activation_env::push_wayland_session_env(wayland_socket.name.to_str().unwrap());

    let mut nested = false;
    #[cfg(all(feature = "backend-winit", not(feature = "backend-native")))]
    {
        nested = true;
    }

    // Session compositor (native backend): scrub WAYLAND_DISPLAY / DISPLAY from
    // OUR OWN environment before ANY GPU init. The Mesa `VK_LAYER_MESA_device_select`
    // Vulkan layer, inside `vkEnumeratePhysicalDevices`, connects to $WAYLAND_DISPLAY
    // and does a BLOCKING wl_display roundtrip to discover the "current" compositor's
    // GPU — while holding the Vulkan loader's global mutex. We are the compositor, not
    // a client; worse, an inherited *stale* WAYLAND_DISPLAY (leaked into the systemd
    // user-manager env by a prior session's `announce_session` and reused as our own
    // not-yet-listening socket name) makes that roundtrip block forever. Two Vulkan
    // instances come up concurrently at startup — the `VulkanRenderer` on this thread
    // and the wgpu context on a worker — so the worker stalls inside the layer holding
    // the loader mutex and this thread's `vkCreateInstance` deadlocks on it: the
    // compositor hangs BEFORE the event loop starts (no seat/DRM-master involvement —
    // confirmed by backtrace). First-boot login has no WAYLAND_DISPLAY so it works;
    // every login after a successful session inherits the stale one and hangs. We
    // re-export WAYLAND_DISPLAY to our REAL socket for children later, after the
    // backend is wired. Nested/winit dev keeps it: there the host display is live, so
    // the layer's roundtrip is both wanted and non-blocking.
    if !nested {
        unsafe {
            std::env::remove_var("WAYLAND_DISPLAY");
            std::env::remove_var("DISPLAY");
        }
        info!("cleared inherited WAYLAND_DISPLAY/DISPLAY before GPU init (native session compositor)");
    }

    info!("Creating the loop loader");
    // Create loader properties to add to state
    let state_loader = Loader {
        socket_name: wayland_socket.name.clone(),
        // socket_name_proprietary: wayland_socket_proprietary.name.clone(),
        loop_signal: event_loop.get_signal(),
        display_handle: display.handle(),
    };

    // Set up the rpc transport
    let rpc_transport = compositor_remote_transport_server_base::transport::create(event_loop.handle());
    let rpc_transport_tx_base = rpc_transport.broadcast_transmit.clone();

    info!("Initializing loop state");

    let (bevy_context_rx, iced_context_rx) = initialize_wgpu_context();

    // The injection point: the loader assembles the world set (update order
    // matters: navigator eases -> camera applies -> backgrounds react) and
    // hands it to the orchestrator. KernelData is populated after Loop::new.
    let mut kernel_data = compositor_support_system_storage_slot_base::base::Storage::new();
    // Shared iced + bevy GPU contexts live in the kernel (driver data). The async
    // wgpu init runs on a background thread; the loader block-waits its result
    // below (after backend wire, so context creation overlaps with it) and fills
    // these slots ONCE, then prewarms every world's registry. Seeded `None` here
    // so the slots exist; the receivers stay local until the wait.
    kernel_data.insert(&compositor_y5_surface_system_base::base::ICED_CONTEXT, None);
    kernel_data.insert(&compositor_background_three_system_base::base::BEVY_CONTEXT, None);
    let worlds = {
        // WT2: world kinds (document/ARCHITECTURE.md → "Window tracking"). The
        // main world is SPATIAL (owns the window Space + is the spawn-target);
        // lock/select are OVERLAY worlds (no space). The loader injects the
        // concrete system set; the builder stamps the kind.
        let mut worlds = compositor_orchestration_world_manager_base::manager::WorldManager::new(
            compositor_support_world_kind_build_base::base::spatial(
                compositor_orchestration_world_manager_base::manager::MAIN_WORLD,
                "main",
                vec![
                    Box::new(compositor_y5_navigator_system_base::base::NavigatorSystem),
                    Box::new(compositor_y5_camera_system_base::base::CameraSystem::default()),
                    Box::new(compositor_background_two_system_base::base::TwoSystem),
                    Box::new(compositor_background_three_system_base::base::ThreeSystem),
                    Box::new(compositor_y5_window_system_base::base::WindowSystem),
                    Box::new(compositor_y5_surface_system_base::base::SurfaceSystem),
                    Box::new(compositor_y5_canvas_system_base::base::CanvasSystem),
                    Box::new(compositor_orchestration_seat_system_pointer::base::PointerSystem),
                    Box::new(compositor_y5_placeholder_system_base::base::PlaceholderSystem),
                    Box::new(compositor_y5_launcher_system_base::base::LauncherSystem),
                    // Owns the window-selection slot (SELECT) + applies SELECT_REQUEST.
                    Box::new(compositor_y5_select_system_base::base::SelectSystem),
                    // Re-anchors the selection toolbar under the cursor on selection change.
                    Box::new(compositor_y5_select_overlay_system::base::SelectionOverlaySystem),
                    // Owns the window-grouping slot (GROUP).
                    Box::new(compositor_y5_group_system_base::base::GroupSystem),
                    // Seeds the overview-mode slot (Super+Tab overlay).
                    Box::new(compositor_y5_overview_system_base::base::OverviewSystem),
                ],
                &kernel_data,
            ),
            &kernel_data,
        );
        worlds.add(compositor_support_world_kind_build_base::base::overlay(
            compositor_orchestration_world_manager_base::manager::LOCK_WORLD,
            "lock",
            vec![
                // The lock world OWNS its bevy registry (prewarmed from the shared
                // context), rather than borrowing the session world's.
                Box::new(compositor_background_three_system_base::base::ThreeSystem),
                Box::new(compositor_y5_lock_system_base::base::LockSystem),
            ],
            &kernel_data,
        ));
        // World 2 (PICKER): the world-selection screen — an OVERLAY world like
        // lock. SUPER+K switches the active binding here; cancelling/choosing a
        // cell switches back. See compositor_y5_picker_system_base::PICKER_WORLD.
        worlds.add(compositor_support_world_kind_build_base::base::overlay(
            compositor_orchestration_world_manager_base::manager::PICKER_WORLD,
            "picker",
            vec![
                // Own parallax background (a NEW instance, distinct from the
                // active world's) drawn behind the sphere.
                Box::new(compositor_background_two_system_base::base::TwoSystem),
                // The picker world OWNS its bevy registry (prewarmed from the
                // shared context) — the sphere scene is proprietary to it.
                Box::new(compositor_background_three_system_base::base::ThreeSystem),
                Box::new(compositor_y5_picker_system_base::base::PickerSystem),
            ],
            &kernel_data,
        ));
        // TEMPORARY (sanity test): two extra SPATIAL worlds for the Super+Alt+1/2/3
        // world-switch shortcuts, so world delegation can be exercised before real
        // world selection exists. Same system set as main minus ThreeSystem (its
        // bevy context_rx is one-shot; the 3D scene is a stub anyway). TwoSystem IS
        // included — it now tolerates an absent BG_THREE (try_get == not locked) —
        // so the test worlds get a real per-world parallax background, which makes
        // per-world background delegation visible in the switch test.
        let test_systems = || -> Vec<Box<dyn compositor_support_system_trait_system_base::base::System>> {
            vec![
                Box::new(compositor_y5_navigator_system_base::base::NavigatorSystem),
                Box::new(compositor_y5_camera_system_base::base::CameraSystem::default()),
                Box::new(compositor_background_two_system_base::base::TwoSystem),
                Box::new(compositor_y5_window_system_base::base::WindowSystem),
                Box::new(compositor_y5_surface_system_base::base::SurfaceSystem),
                Box::new(compositor_y5_canvas_system_base::base::CanvasSystem),
                Box::new(compositor_orchestration_seat_system_pointer::base::PointerSystem),
                Box::new(compositor_y5_placeholder_system_base::base::PlaceholderSystem),
                Box::new(compositor_y5_launcher_system_base::base::LauncherSystem),
                Box::new(compositor_y5_select_system_base::base::SelectSystem),
                Box::new(compositor_y5_select_overlay_system::base::SelectionOverlaySystem),
                Box::new(compositor_y5_group_system_base::base::GroupSystem),
                Box::new(compositor_y5_overview_system_base::base::OverviewSystem),
            ]
        };
        let w2 = worlds.add(compositor_support_world_kind_build_base::base::spatial(uuid::Uuid::now_v7(), "test-2", test_systems(), &kernel_data));
        let w3 = worlds.add(compositor_support_world_kind_build_base::base::spatial(uuid::Uuid::now_v7(), "test-3", test_systems(), &kernel_data));
        (worlds, [compositor_orchestration_world_manager_base::manager::MAIN_WORLD, w2, w3])
    };
    let (worlds, test_world_ids) = worlds;
    kernel_data.insert(&compositor_orchestration_core_state_base::state::TEST_WORLDS, test_world_ids);

    let inner = State::new(
        environment.clone(),
        nested,
        state_loader,
        rpc_transport.broadcast_transmit,
        kernel_data,
        worlds,
    );
    // Initialize loop state
    let mut state = Loop::new(inner, &display.handle(), None, event_loop.handle());

    // KernelData: hand systems the smithay wiring handles (read-only tokens).
    {
        let pointer = state.state.seat.seat.get_pointer().expect("seat factory adds a pointer");
        let keyboard = state.state.seat.seat.get_keyboard().expect("seat factory adds a keyboard");
        compositor_orchestration_smithay_data_base::data::populate(
            &mut state.inner.kernel,
            display.handle(),
            event_loop.handle(),
            pointer,
            keyboard,
        );
    }

    // Recreate scene worlds persisted in the `world` table under their saved UUIDs
    // (the picker world's build already loaded the registry into its slot). Each
    // rebuilt world rehydrates its own per-world state + placeholders.
    compositor_y5_picker_world_restore::base::restore_worlds(&mut state);

    // Legacy-bus receivers (TRANSITIONAL, document/ARCHITECTURE.md): deferred
    // cross-module notifications whose handlers still take &mut Loop. Each
    // registration moves into a world system as its state migrates.
    // Selection reacts to group updates by clearing itself (was a synchronous
    // group -> select call; the group module only announces what happened).
    state.inner.bus.register(
        &compositor_y5_group_interface_base::protocol::event::GROUP_UPDATED,
        |l, _event| compositor_y5_select_interface_base::clear(l),
    );

    let wayland_socket_name_default_subprocess = wayland_socket.name.clone();
    let wayland_socket_name_default_subprocess_2 = wayland_socket.name.clone();
    let wayland_socket_name_for_children = wayland_socket.name.clone();

    info!("Hooking up wayland to the loop");
    // Register wayland socket in Smithay event loop.
    wayland::register(wayland_socket, &mut event_loop);
    // wayland::register(wayland_socket_proprietary, &mut event_loop, true);

    // You also need to add the display itself to the event loop, so that client events will be processed by wayland-server.
    event_loop
        .handle()
        .insert_source(
            Generic::new(display, Interest::READ, Mode::Level),
            |_, display, state| {
                // Safety: we don't drop the display
                unsafe {
                    // `D = Dispatch` now (document/SMITHAY_DECOUPLING.md): the
                    // wayland dispatch type is the protocol state field, not the
                    // whole Loop.
                    display.get_mut().dispatch_clients(&mut state.state).unwrap();
                }
                // Apply the world effects this cycle's protocol handlers recorded
                // (map/commit/fullscreen/layer/dmabuf) — see SMITHAY_DECOUPLING.md.
                // Same iteration, synchronous; the handlers stayed world-free.
                state.drain_protocol();
                Ok(PostAction::Continue)
            },
        )
        .unwrap();

    info!("Creating renderer");

    #[cfg(feature = "backend-native")]
    {
        info!("starting loader...");
        // The returned handles are the backend's integration surface (see
        // compositor_kernel_native_device_interface_base): the main project applies
        // runtime device settings through them.
        let _backend_handles = compositor_kernel_native_wire_entry_base::entry::wire(
            &mut state,
            wayland_socket_name_default_subprocess,
            &mut event_loop,
        );
        info!("starting loader OK");
    }

    #[cfg(all(feature = "backend-winit", not(feature = "backend-native")))]
    {
        compositor_kernel_winit_wire_entry_base::entry::wire(
            &mut state,
            wayland_socket_name_default_subprocess,
            &mut event_loop,
        );
    }

    // After udev and winit have initialized, activate the environment
    //
    info!("Backend initialization - Complete");

    // Initial seat-pointer placement. The cursor renders at the seat's world
    // location (`(0,0)` -> screen center), but the relative-motion accumulator
    // (`PointerState.motion`) defaults to physical top-left; without this the
    // first mouse move would accumulate from the corner and the cursor would jump
    // there. This must run AFTER the backend maps the output — `apply_pointer`
    // re-projects world `(0,0)` through the live camera + output geometry, which
    // don't exist when the seat is constructed. Single-output, so it runs once.
    state.inner.apply_pointer(smithay::utils::Point::from((0.0, 0.0)));

    // ---------------------------------------------------------------------
    // Driver instances: PRE-CREATE + ASSERT, never construct during render.
    //
    // The wgpu/Vulkan contexts are built on a background thread (a Vulkan/wayland
    // requirement). Rather than stashing the channel and polling it every frame —
    // lazily building GPU registries mid-render the moment the context happens to
    // land — block here until both contexts arrive (they were created in parallel
    // with the backend wire above), store them in the kernel ONCE, then build the
    // capture registry and prewarm every world's iced + bevy registry. From the
    // first frame onward these instances are guaranteed present; the render path
    // asserts them rather than constructing them.
    {
        const WGPU_INIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
        let iced_ctx = iced_context_rx
            .recv_timeout(WGPU_INIT_TIMEOUT)
            .unwrap_or_else(|e| compositor_developer_debug_instance_record::abort!("iced wgpu context never arrived: {e:?}"));
        let bevy_ctx = std::sync::Arc::new(
            bevy_context_rx
                .recv_timeout(WGPU_INIT_TIMEOUT)
                .unwrap_or_else(|e| compositor_developer_debug_instance_record::abort!("bevy wgpu context never arrived: {e:?}")),
        );
        info!("wgpu contexts received — pre-creating driver registries");

        *state.inner.kernel.get_mut(&compositor_y5_surface_system_base::base::ICED_CONTEXT_MUT) = Some(iced_ctx);
        *state.inner.kernel.get_mut(&compositor_background_three_system_base::base::BEVY_CONTEXT_MUT) = Some(bevy_ctx.clone());

        // Capture registry — kernel driver data shared by every backend.
        *state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY_MUT) =
            Some(compositor_y5_graphic_capture_registry::CaptureRegistry::new(bevy_ctx));

        // Prewarm every world's per-world registries (each helper no-ops where its
        // slot is absent — e.g. an overlay world without SurfaceSystem). Covers the
        // static worlds AND any disk-restored worlds rebuilt above.
        for id in state.inner.worlds.ids() {
            compositor_y5_surface_system_base::base::ensure_registry(
                state.inner.worlds.get_mut(id).storage_mut(),
                &state.inner.kernel,
            );
            compositor_background_three_system_prewarm::prewarm::ensure_registry(
                state.inner.worlds.get_mut(id).storage_mut(),
                &state.inner.kernel,
            );
        }

        // Assert the essentials are live: the main world hosts both iced surfaces
        // and the 3D background. A miss here means prewarm wiring is broken.
        let main = state.inner.worlds
            .get_mut(compositor_orchestration_world_manager_base::manager::MAIN_WORLD)
            .storage_mut();
        if main
            .try_get_mut(&compositor_y5_surface_system_base::base::SURFACE_MUT)
            .and_then(|s| s.registry.as_ref())
            .is_none()
        {
            compositor_developer_debug_instance_record::abort!("main world iced registry missing after prewarm");
        }
        if main
            .try_get_mut(&compositor_background_three_system_base::base::BG_THREE_MUT)
            .and_then(|t| t.registry.as_ref())
            .is_none()
        {
            compositor_developer_debug_instance_record::abort!("main world bevy registry missing after prewarm");
        }
        info!("driver registries pre-created and asserted present");
    }

    // Optional VulkanRenderer hardware self-test (feature `vulkan-validate`).
    // Independent of the active backend — it builds its own VkInstance and
    // renders one frame to an exported dmabuf, so it validates the new Vulkan
    // renderer end-to-end on the GPU even under the winit (gles) backend.
    #[cfg(feature = "vulkan-validate")]
    {
        info!("vulkan-validate: running VulkanRenderer self-test...");
        match compositor_kernel_vulkan_renderer_core_base::renderer::VulkanRenderer::validate() {
            Ok(proof) => info!("vulkan-validate: OK — {proof}"),
            Err(e) => {
                use compositor_developer_debug_instance_record::error;
                error!("vulkan-validate: FAILED — {e}");
            }
        }
    }

    // Now that the backend is wired, advertise OUR socket as WAYLAND_DISPLAY for
    // child processes. This must happen AFTER the backend wire(): the winit
    // backend nests into the host compositor at init and reads WAYLAND_DISPLAY
    // to find it, so overwriting it earlier would point winit at our own
    // (not-yet-serving) socket. The native backend doesn't nest, so post-wire is
    // correct for both. Children spawn below (announce_session), after this.
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &wayland_socket_name_for_children) };
    info!("WAYLAND_DISPLAY set to {:?} for child processes", wayland_socket_name_for_children);

    // After WlrLayerShellState::new and event loop is running:
    compositor_orchestration_environment_interface_lifecycle::lifecycle::announce_session(
        wayland_socket_name_default_subprocess_2.to_str().unwrap(),
        &environment.DesktopName,
    );

    // move to loop factory ( it can spawn. )
    let (results_tx, results_rx) = cl_channel::channel::<SampleBatch>();
    let sampler = Sampler::spawn(Arc::new(default_registry()), results_tx);
    *state.inner.kernel.get_mut(&compositor_orchestration_driver_introspection_base::base::SAMPLER_MUT) = Some(sampler);

    // Register the receiver as a calloop source:
    event_loop
        .handle()
        .insert_source(results_rx, |event, _metadata, state: &mut Loop| {
            let cl_channel::Event::Msg(result) = event else {
                return;
            };
            // let SampleResult { uuid, data, sampled_at: _ } = result;

            compositor_orchestration_draw_state_lifecycle::lifecycle::sampler_result(state, result);
        })
        .unwrap_or_else(|e| compositor_developer_debug_instance_record::abort!("register sampler results source: {e:?}"));

    // App-launch executor (kernel.execution): builds the Executor driver, stores
    // it as driver data, and wires its calloop sources (off-thread worker outcome
    // receiver + SIGCHLD reaper). Each completed launch is broadcast by
    // orchestration as the general per-world `Executed` event.
    launch_executor::install(&mut state, &event_loop.handle());

    // Launch the compositor-owned input method configured in `preferences.json`
    // (`ime: { exec, args }`); unset ⇒ none is launched. Must be AFTER WAYLAND_DISPLAY is exported
    // (above) so it connects to OUR socket; the spawned process group is the ONLY client
    // authorized to bind the input-method / virtual-keyboard globals (see `text.input.launch`).
    compositor_support_smithay_state_text_input_launch::launch::launch(
        state.inner.preference.ime.clone(),
    );

    // Sampling heartbeat — a sparing, multi-level demo of live developer logs (so the
    // viewer shows activity over time and its level filters can be exercised). Remove when
    // not demoing.
    // std::thread::spawn(|| {
    //     let mut tick: u64 = 0;
    //     loop {
    //         std::thread::sleep(std::time::Duration::from_secs(4));
    //         tick += 1;
    //         info!("heartbeat tick={tick}");
    //         if tick % 3 == 0 {
    //             trace!("heartbeat detail: {tick} ticks, ~{}s uptime", tick * 4);
    //         }
    //         if tick % 5 == 0 {
    //             warn!("heartbeat milestone: {tick} ticks elapsed");
    //         }
    //     }
    // });

    // Sampler::Drop closes its internal registration channel, which causes
    // the thread to exit cleanly when state's sampler field is dropped.
    info!("Event Loop start");
    event_loop.run(None, &mut state, move |_| {
        // Is now running. (Input-independent control-plane work — display drains,
        // lock engage — is ping-driven via `state.inner.ping_control()`, drained
        // in the backend's control-plane ping source, NOT polled per dispatch.)
    })?;

    Ok(())
}
