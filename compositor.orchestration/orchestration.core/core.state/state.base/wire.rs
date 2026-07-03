use crate::Loop;
use crate::state::{Orchestrator, StateDRMBinding};
use smithay::backend::renderer::ImportDma;
use smithay::desktop::{Space, Window};
use smithay::utils::{Logical, Physical, Point, Rectangle};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::ToplevelSurface;
use std::ops::DerefMut;
use std::sync::{Arc, Mutex};
use compositor_y5_camera_transform_translate::transform::Transform;
use compositor_support_smithay_dispatch_state_base::state::Dispatch;
use compositor_support_smithay_dispatch_wire_base::wire::Wire;
use compositor_support_smithay_dispatch_wire_trait::wire_trait::WireTrait;
use compositor_support_smithay_state_xdg_activation_dispatch::wire::ActivationDetails;
use compositor_y5_window_interface_record::data::WindowData;
use compositor_y5_window_lifecycle_event::event::WindowLifecycleEvent;
use compositor_y5_window_lifecycle_state::lifecycle::WindowLifecycle;

impl WireTrait for Orchestrator {
    fn host_space(&self) -> &compositor_support_smithay_state_space_base::state::SpaceState {
        self.space_state()
    }
    fn host_space_mut(&mut self) -> &mut compositor_support_smithay_state_space_base::state::SpaceState {
        self.space_state_mut()
    }

    fn initialize_surface_data(&mut self, window: Window) {
        let uuid = uuid::Uuid::now_v7();
        let user_data = window.user_data().get::<WindowData>();
        if user_data.is_some() {
            abort!("new_toplevel: WindowData is set")
        }
        // Window basic data ( UUID )
        window
            .user_data()
            .insert_if_missing_threadsafe(|| WindowData { UUID: uuid });

        // Place the uuid into surface as well. required for ondestroy.
        let surface = window.toplevel().unwrap_or_else(|| abort!("toplevels only")).wl_surface();
        info!("initialize_surface_Data: {:?}", uuid);
        with_states(surface, |states| {
            let inserted = states
                .data_map
                .insert_if_missing_threadsafe(|| Mutex::new(WindowData { UUID: uuid }));
            if !inserted {
                // CHECK: Its interesting behaviour:
                // Windows (and their user_data) can completely recreated without a surface re-creation.
                // CHECK: THis means some stuff: The windowdata could be a mismatch between whats actually set in window.
                // Therefore must be updated here.
                // CHECK: But it still wont solve discrepancies with what PH expects, etc. as surface is destroyed and recreated.
                // panic!("Duplication of UUID set in surface.");
                // Replace the entire value:
                let slot = states.data_map.get::<Mutex<WindowData>>().unwrap();
                *slot.lock().unwrap() = WindowData { UUID: uuid };
            }
        });

        // Skip setting it. Activation probably occurs after toplevel as its part of the activation requirements?.
        // Extract activation details. optional
        // let activation = with_states(surface, |states| {
        //     let data = states.data_map.get::<ActivationDetails>().cloned();
        //     data
        // });

        // Activation obtain, place inside window data. Can use surface here if needed. on destroy it should remove the token if it wasnt already. important.
        // if let Some(activation) = activation {
        //     window
        //         .user_data()
        //         .insert_if_missing_threadsafe(|| activation);
        // }
    }

    fn destroy_surface_data(&mut self, surface: ToplevelSurface) {
        let activation_details = with_states(surface.wl_surface(), |states| {
            states.data_map.get::<ActivationDetails>().cloned()
        });

        info!("destroy_surface_data...");
        with_states(surface.wl_surface(), |states| {
            let data = states
                .data_map
                .get::<Mutex<WindowData>>()
                .unwrap_or_else(|| abort!("toplevels to have window data"))
                .lock()
                .unwrap()
                .UUID
                .clone();
            info!("destroy_surface_data: {:?}", data);
            self.window_lifecycle_mut()
                .incoming
                .push(WindowLifecycleEvent::Destroyed(data, activation_details));
        });
    }

    fn place_window(&mut self, window: Window, geometry: Rectangle<i32, Logical>) {
        // Side effect- because of the dispatcher wiring problem, the place_window returns a location rather than calling map_element.
        // However this function should be treated as if it called the initial space map_element call.
        self.window_lifecycle_mut()
            .incoming
            .push(WindowLifecycleEvent::InitialMap(window));
    }

    fn fullscreen_request(&mut self, window: Window, fullscreen: bool) {
        self.window_lifecycle_mut()
            .incoming
            .push(WindowLifecycleEvent::Fullscreen(window, fullscreen));
    }

    fn apply_pointer(&mut self, storage_point: Point<f64, Logical>) {
        // Read the hosted space's output geometry, then drop the borrow before
        // mutating camera/pointer (both live in the same world storage).
        let (mode_size, scale) = {
            // The cursor's output (via current/cursor resolution), not the primary —
            // a pointer warp onto a secondary monitor must project through THAT
            // monitor's mode/scale to land at the right physical position.
            let output = self.current_output();
            let mode = output.current_mode().unwrap_or_else(|| abort!("output has a current mode"));
            (mode.size, output.current_scale().fractional_scale())
        };
        let mode = smithay::output::Mode { size: mode_size, refresh: 0 };
        let camera = &self.camera().transform;

        let ctx = compositor_y5_camera_transform_translate::transform::Context::new(
            (camera.position.x, camera.position.y),
            camera.zoom,
            (mode.size.w as f64, mode.size.h as f64),
            scale,
        );

        let warp_phys: Point<f64, Physical> = {
            let t: Transform = (storage_point, ctx).into();
            t.into()
        };
        let pointer = self.pointer_mut();
        pointer.motion.x = warp_phys.x;
        pointer.motion.y = warp_phys.y;

        // Reset for camera as well.
        self.camera_mut().position_previous = pointer.motion;
    }

    fn dmabuf_import(
        &mut self,
        dispatch: &mut Dispatch,
        _global: &smithay::wayland::dmabuf::DmabufGlobal,
        _dmabuf: smithay::backend::allocator::dmabuf::Dmabuf,
        notifier: smithay::wayland::dmabuf::ImportNotifier,
    ) -> Option<(
        smithay::backend::allocator::dmabuf::Dmabuf,
        smithay::wayland::dmabuf::ImportNotifier,
    )> {
        let Some(gpu_ref) = self.kernel.get(&crate::state::GPU_BINDING).as_ref() else {
            return Some((_dmabuf, notifier));
        };

        // Borrow once, hold the guard for the whole operation.
        let mut binding = gpu_ref.borrow_mut();

        // Split borrow: extract a mutable reference to `gpus` and an immutable
        // reference to `primary` from the same guard. The compiler can do this
        // because field accesses are disjoint.
        let StateDRMBinding { gpus, primary, .. } = &mut *binding;

        let mut renderer = match gpus.single_renderer(primary) {
            Ok(r) => r,
            Err(err) => {
                warn!(
                    "Failed to acquire renderer, falling through to default DMABuf import: err={err:?}"
                );
                // Already moved..
                return Some((_dmabuf, notifier));
            }
        };

        match renderer.import_dmabuf(&_dmabuf, None) {
            Ok(_) => {
                if let Err(err) = notifier.successful::<Dispatch>() {
                    warn!(
                        "Dmabuf explicit imported successfully, butclient disappeared while signaling dmabuf import success: err={err:?}"
                    );
                } else {
                    // tracing::info!("Dmabuf explicit imported successfully");
                }
                // // Remains unused
                // let _ = notifier.successful::<Dispatch>();
            }
            Err(err) => {
                warn!("Failed to import client dmabuf: err={err:?}");
                notifier.failed();
            }
        }

        None // we handled it
    }
}
// Problem:
// Also, calling request_activation is not set until new top level. place window needs to check the placeholder restoration, not top_level.
// the activation (request_activation) is for focus and raising. like clicking a link sends a dbus to activate a new window in chrome. it is not necessarily init behavior.
// this can be great afterwards for navigator
//
// OK. Here is what I've done:
// 1. request_activation now wired. it sets surface data with the activation token. I assume it is called before new top level. When a restoration is deleted/consumed, i remove its relevant token.
//
// pub struct ActivationDetails{
//     token: XdgActivationToken,
//     token_data: XdgActivationTokenData,
// }
//
// pub fn request_activation<WireObject: DispatchWire>(
//     dispatch: &mut Dispatch<WireObject>,
//     surface: WlSurface,
//     token: XdgActivationToken,
//     token_data: XdgActivationTokenData,
// ) {
//     with_states(
//         &surface,
//         |states| {
//             let inserted = states
//                 .data_map
//                 .insert_if_missing_threadsafe(|| ActivationDetails{
//                     token,
//                     token_data,
//                 });
//             if !inserted {
//                 panic!("Duplication of token data set in surface."); // Dev safeguards
//             }
//         },
//     );
// }
// 2. When the surface is destroyed, the token is cleared:
//          xdg_activation.remove_token(&activation.token); // <-- a cloned activation_token based on toplevel surface data.
//    GC: Important. However hard to manage in this scope. Clients may have their own tokens and it may be used for actual activation stuff like raising the window.
// 3.
//
// CHECK: SKipped for now. GC later. important nevertheless. Tokens remain in store and clients may use their own tokens.,
// 3. Attached a 1 minute timer to calloop to retain only non stale tokens. If a token did never activate then it must be removed since its surface is non existent.
// external tokens: if a single toplevel has only a single token, it may not be accurate.
// So no GC for now.

// When a restoration is deleted/consumed, i remove its relevant token.
