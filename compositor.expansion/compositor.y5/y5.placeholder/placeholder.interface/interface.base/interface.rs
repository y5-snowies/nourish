use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::Window;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::utils::{Logical, Point, Rectangle, Size};
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::{compositor, fractional_scale};
use std::collections::HashMap;

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use uuid::Uuid;
use compositor_introspection_launchplan_plan_base::LaunchPlan;
use compositor_introspection_restoration_state_base::{PendingRestoration, match_window};
use compositor_introspection_sampler_window_base::sampler::SampleBatch;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_core_state_base::{Loop, Transform};
use compositor_y5_surface_protocol_base::protocol::{SurfaceMessage, SurfaceMessageType};
use compositor_y5_window_interface_record::window::LoopWindow;
use compositor_y5_placeholder_protocol_base::message::PlaceholderAction;
use compositor_y5_placeholder_protocol_base::message::PlaceholderAction::Launch;
use compositor_y5_placeholder_record_base::placeholder::{Placeholder, PlaceholderVisible};
use compositor_y5_placeholder_surface_base::{PlaceholderMessage, PlaceholderUi};

// Whenever a new window is created it must be attached to a placeholder.
//
// CHECK: Unrestore(unset the flag, erase the token) restoring placeholders if they have stalled for over 1 minute.
// CHECK: GC For the tokens

pub fn on_window_map_initial(state: &mut Loop, window: Window) -> bool {
    if window.toplevel().is_none() {
        return false;
    }

    let window_data_0 =
        window.application(&state.inner.space_state().state, &state.inner.loader.display_handle);

    if let Some(data) = window_data_0 {
        if let Some(uuid) = window.uuid() {
            if let Some(sampler) = state.inner.kernel.get(&compositor_orchestration_driver_introspection_base::base::SAMPLER) {
                if let Some(pid) = data.meta.meta.pid {
                    sampler.register(uuid, pid, data.meta.clone());
                }
            }
        }

        // CHECK: Handling Better-  Displayname = gamescope-wl,
        if let Some(title) = &data.meta.meta.title {
            if title.eq("gamescope") {
                if let Some(wl) = window.wl_surface() {
                    compositor::with_states(wl.as_ref(), |states| {
                        states.data_map.insert_if_missing_threadsafe(||{
                            compositor_support_smithay_state_fractional_base::state::NestedCompositorSurface{}
                        });

                        fractional_scale::with_fractional_scale(states, |fs| {
                            fs.set_preferred_scale(1.0);
                        });
                    })
                }
            }
        }
    }
    // 1. Check for restoration, attach to an existing placeholder
    // ---
    let window_data_0 =
        window.application(&state.inner.space_state().state, &state.inner.loader.display_handle);
    let mut window_plan_0: Option<_> = None;
    let mut restored_ph: Option<Uuid> = None;

    if let Some(window_data_0) = window_data_0 {
        // On first-commit:
        let candidate_token = window.activation();

        let candidate_token_string = if let Some(candi) = &candidate_token {
            Some(candi.token.as_str())
        } else {
            None
        };

        let mut pending_restoration: Vec<PendingRestoration> = vec![];
        for (ph, _) in &state.inner.placeholder_mut().visible {
            // A placeholder is a match candidate if it's mid-launch (token/PID
            // restoration) OR has capture-armed attributes (adopt-on-map). A
            // capture-only candidate carries no token and pid `-1`, so neither
            // the token nor the PID-tree signal can spuriously bind it.
            let is_launching = ph.launching && ph.restoration.is_some();
            let is_capture_armed = !compositor_introspection_launchplan_plan_capture::capture::capture_keys(&ph.launch).is_empty();
            if !is_launching && !is_capture_armed {
                continue;
            }

            let (activation_env, launched_pid) = if let Some(restore) = &ph.restoration {
                let mut activation_env = HashMap::new();
                activation_env.insert(
                    String::from(compositor_introspection_restoration_state_base::token::ACTIVATION_TOKEN_ENV),
                    restore.token.clone(),
                );
                activation_env.insert(
                    String::from(compositor_introspection_restoration_state_base::token::STARTUP_ID_ENV),
                    restore.token.clone(),
                );
                (activation_env, restore.child.map(|w| w as i32).unwrap_or(-1))
            } else {
                (HashMap::new(), -1)
            };

            pending_restoration.push(PendingRestoration {
                id: ph.uuid,
                plan: ph.launch.clone(),
                launched_pid,
                activation_env,
            });
        }

        // println!("CHecking pending restoration");
        if let Some(placeholder_id) = match_window(
            pending_restoration.as_slice(),
            &window_data_0.meta.clone(),
            &window_data_0.hints.clone(),
            candidate_token_string,
            &state.inner.placeholder_mut().restoration_registry,
        ) {
            restored_ph = Some(placeholder_id);
            // CHECK: Update placeholder state to retain placeholder_id.
            // Remove token from registry.
            if let Some(candidate_token) = candidate_token {
                state
                    .state
                    .xdg_activation
                    .xdg_activation
                    .remove_token(&candidate_token.token);
            }
        }

        window_plan_0 = Some(compositor_introspection_launchplan_plan_base::LaunchPlan::new(window_data_0))
    }

    let mut placeholder = if let Some(placeholder_id) = restored_ph {
        /// Erase restored PH
        let (restored_ph, restored_ph_handle) = state.inner.placeholder_mut()
            .erase_visible(&placeholder_id)
            .unwrap_or_else(|| abort!("Restored PH to exist."));

        // Clears out the handle.
        if let Some(ref mut registry) = state.inner.surface_mut().registry {
            registry.destroy(restored_ph_handle);
        }
        // Maps window surface.
        state.inner.space_state_mut().state.map_element(
            window.clone(),
            Point::new(restored_ph.position.0, restored_ph.position.1),
            true,
        );
        // Register in the draw-order authority too (restore is a map path).
        if let Some(uuid) = window.uuid() {
            state.inner.register_drawable(uuid, compositor_support_world_order_track_base::base::DrawLayer::CONTENT);
        }

        let toplevel = window.toplevel().unwrap_or_else(|| abort!("reform expects toplevels only."));
        toplevel.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Resizing);
            state.size = Some(Size::new(restored_ph.size.0, restored_ph.size.1));
        });

        // At this point, window lifecycle may still attempt to place.
        // Check order.
        // Slight 'flicker' possible because it maps it already. This lifecycle event should act as the initial mapping.

        toplevel.send_configure();

        // Destory handle
        // Now- Keeps UUID
        Placeholder {
            uuid: placeholder_id,
            size: restored_ph.size,
            position: restored_ph.position,
            launch: Some(restored_ph.launch),
            launch_session: window_plan_0,
            session_time: Instant::now(),
            persistent: true,
        }
    } else {
        // Otherwise, create a placeholder and attach to the window
        let mut placeholder = Placeholder {
            uuid: Uuid::now_v7(),
            size: (100, 100), // Just sane defaults so it doesnt zero out. however this must be an no-op
            position: (0, 0),
            launch: window_plan_0,
            launch_session: None,
            session_time: Instant::now(),
            persistent: false,
        };
        placeholder
    };

    // grab the window UUID.
    let window_uuid = window.uuid().unwrap_or_else(|| abort!("Windows to have UUID"));
    info!("Insert PH, Window UUID: {:?}", window_uuid);
    state.inner.placeholder_mut().insert(placeholder, window_uuid);
    // Persist this world's placeholders (incl. not-yet-visible ones). Inserting a
    // placeholder is a discrete, important event → IMMEDIATE (the per-frame sample
    // transform updates below stay debounced).
    compositor_support_system_persist_mark_base::base::mark_world(state.inner.worlds.active_id(), true);

    if restored_ph.is_some() {
        return true;
    }

    return false;
    // The window is initially mapped, and the placeholder should track the data
}

pub fn on_window_destroy(state: &mut Loop, uuid: Uuid, renderer: &mut GlesRenderer) {
    if let Some(sampler) = state.inner.kernel.get(&compositor_orchestration_driver_introspection_base::base::SAMPLER) {
        sampler.unregister(uuid);
    }
    info!("Destroy PH, Window UUID: {:?}", uuid);

    // The non-trivial cases:
    // 1. Destroy called twice  ( logically shouldn't happen. surface destroy should be a cleanup event )
    // 2. Destroy called for subsurfaces even though they arent toplevels, which makes it call twice ( logically shouldn't happen, and explicitly checks for the existence of WindowData before, and WindowData explicitly requests toplevel. so it shouldnt enter here )

    // More trivially:
    // 3. The on_window_map_initial not inserting a new record..
    // 4. The store doesn't mutate correctly.
    // 5. The UUID is somewat confusing PH uuid with window UUID.
    // 6. A surface is destroyed before doing new toplevel

    // The window is destroyed. the placeholder should become visible.
    // this shouldn't erase the placeholder at all.
    //
    if !state.inner.placeholder_mut().map.contains_key(&uuid) {
        return;
    }
    // there is a small exception: placeholders that were not previously saved, i.e they werent from a launchplan, should be erased if they lived for less than 1 minute.
    let ph = state.inner.placeholder_mut().erase(&uuid);
    // Erasing a placeholder is a discrete, important event → persist IMMEDIATELY.
    compositor_support_system_persist_mark_base::base::mark_world(state.inner.worlds.active_id(), true);

    if !ph.persistent && ph.session_time.elapsed().lt(&Duration::from_secs(10)) {
        // Discard it all completely.
        return;
    }

    spawn_visible(state, renderer, ph);
}

/// Build the visible iced launcher surface for a placeholder and register it in
/// the spawn-target world's `visible` set. Shared by window-destroy (live window →
/// tile) and restore (disk → tile). No-op if the placeholder has no launch plan.
pub fn spawn_visible(state: &mut Loop, renderer: &mut GlesRenderer, ph: Placeholder) {
    let Some(plan) = ph.launch.clone() else {
        return; // no launch plan → nothing to relaunch; discard.
    };
    let loc_logical: Point<i32, Logical> = Point::new(ph.position.0, ph.position.1);
    let size_logical: Size<i32, Logical> = Size::new(ph.size.0, ph.size.1);
    let t: Transform = (Rectangle::new(loc_logical, size_logical), state.size_ctx_all()).into();

    // Assign unique ID to the placeholder. this must continue from previous placeholder when it was retained
    let application_registry = state.inner.placeholder().application_registry.clone();
    let handle = compositor_y5_surface_draw_handle::handle::load(
        state,
        renderer,
        PlaceholderUi::new(plan, ph.launch_session.clone(), application_registry),
        t.into_storage_rect_physical(),
        compositor_y5_surface_draw_handle::handle::IcedSpace::World,
        compositor_orchestration_draw_layer_base::base::Layer::SCENE.bits(),
    );

    let tx = state.inner.surface_mut().surface_message_buffer_channel.0.clone();
    let ph_uuid = ph.uuid;
    state.inner.surface_mut()
        .registry
        .as_mut()
        .unwrap()
        .instance_mut(handle)
        .unwrap()
        .runtime_mut()
        .set_message_handler(move |message: &PlaceholderMessage| __dispatch(ph_uuid, message, &tx));

    state.inner.placeholder_mut().push_visible(ph, handle);
}

/// Promote the spawn-target world's disk-restored placeholders into visible
/// launcher tiles, now that the rim has a renderer. Cheap no-op when none are
/// pending; defers a frame if the iced registry isn't up yet (early startup).
pub fn promote_restored(state: &mut Loop, renderer: &mut GlesRenderer) {
    if state.inner.placeholder().pending_restore.is_empty() {
        return;
    }
    if state.inner.surface().registry.is_none() {
        return; // iced not initialised yet — retry next frame.
    }
    let pending = std::mem::take(&mut state.inner.placeholder_mut().pending_restore);
    for ph in pending {
        spawn_visible(state, renderer, ph);
    }
}

// CHECK : Change to native watch events instead of polling
pub fn on_window_sample(state: &mut Loop, sample: &SampleBatch) {
    //
    for item in &sample.results {
        let Some(data) = &item.data else { continue };

        let Some(_) = state.inner.placeholder_mut().map.get_mut(&item.uuid) else {
            continue;
        };

        // println!("Received sample for active placeholder");

        state.inner.placeholder_mut().modify(&item.uuid, |placeholder| {
            if let Some(ref existing) = placeholder.launch_session {
                placeholder.launch_session = Some(LaunchPlan {
                    application_data: data.clone(),
                    ..existing.clone()
                });
            } else if let Some(ref existing) = placeholder.launch {
                placeholder.launch = Some(LaunchPlan {
                    application_data: data.clone(),
                    ..existing.clone()
                });
            } else {
                placeholder.launch = Some(LaunchPlan::new(data.clone()));
            }
        });
    }
    // Sampler refresh runs continuously — persist the placeholders DEBOUNCED so
    // the inferred-hint changes survive a restart without spamming the disk.
    if !sample.results.is_empty() {
        compositor_support_system_persist_mark_base::base::mark_world(state.inner.worlds.active_id(), false);
    }
}

// Generally unsafe. Commited state for size takes a few frames.
pub fn invalidate_geometry(state: &mut Loop, window: Window) {
    let window_uuid = window.uuid().unwrap_or_else(|| abort!("Windows to have UUID"));
    let geometry = state
        .inner.space_state()
        .state
        .element_geometry(&window)
        .unwrap_or_else(|| abort!("window geometry to be available"));
    let position = geometry.loc;
    let mut size = geometry.size;

    state.inner.placeholder_mut().modify(&window_uuid, |placeholder| {
        placeholder.size.0 = size.w;
        placeholder.size.1 = size.h;
        placeholder.position.0 = position.x;
        placeholder.position.1 = position.y;
    });
}

pub fn set_visible_geometry(
    state: &mut Loop,
    uuid: Uuid,
    geometry: (Option<(i32, i32)>, Option<(i32, i32)>),
) {
    let (position, size) = geometry;

    let handle = state.inner.placeholder_mut()
        .modify_visible(&uuid, |placeholder| {
            if let Some(size) = size {
                placeholder.size.0 = size.0;
                placeholder.size.1 = size.1;
            }

            if let Some(position) = position {
                placeholder.position.0 = position.0;
                placeholder.position.1 = position.1;
            }
        });

    let Some((_, handle)) = handle else {
        return;
    };

    let handle = handle.id;
    let Some(registry) = &mut state.inner.surface_mut().registry else {
        return;
    };

    if let Some(size) = size {
        registry.request_resize_by_id(handle, Size::new(size.0, size.1));
    }
    if let Some(position) = position {
        registry.set_location_by_id(handle, Point::new(position.0, position.1));
    }
}

pub fn set(
    state: &mut Loop,
    window: Window,
    size: Option<Size<i32, Logical>>,
    position: Option<Point<i32, Logical>>,
) {
    let window_uuid = window.uuid().unwrap_or_else(|| abort!("Windows to have UUID"));

    state.inner.placeholder_mut().modify(&window_uuid, |placeholder| {
        if let Some(size) = size {
            placeholder.size.0 = size.w;
            placeholder.size.1 = size.h;
        }
        if let Some(position) = position {
            placeholder.position.0 = position.x;
            placeholder.position.1 = position.y;
        }
    });
}

fn __dispatch(
    uuid: Uuid,
    msg: &compositor_y5_placeholder_surface_base::message::PlaceholderMessage,
    tx: &Sender<SurfaceMessage>,
) {
    let dispatch = match msg {
        PlaceholderMessage::LaunchClicked => Some(PlaceholderAction::Launch()),
        PlaceholderMessage::SaveClicked { updated_plan } => {
            Some(PlaceholderAction::Save(updated_plan.as_ref().clone()))
        }
        PlaceholderMessage::DismissClicked {} => Some(PlaceholderAction::Erase()),
        _ => None,
    };

    if dispatch.is_none() {
        return;
    }

    let dispatch = dispatch.unwrap();
    let _ = tx.send(SurfaceMessage {
        message: SurfaceMessageType::Placeholder(
            compositor_y5_surface_protocol_base::placeholder::message::PlaceholderMessage {
                uuid,
                action: dispatch,
            },
        ),
    });
}
