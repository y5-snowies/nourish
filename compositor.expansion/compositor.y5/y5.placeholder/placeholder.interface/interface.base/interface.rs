use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::Window;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::utils::{Logical, Point, Rectangle, Size};
use std::collections::HashMap;

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use uuid::Uuid;
use compositor_introspection_launchplan_plan_base::LaunchPlan;
use compositor_introspection_extraction_window_base::hints::extract::push_toplevel_icon_hints;
use compositor_introspection_restoration_state_base::{PendingRestoration, SessionKey, match_window};
use compositor_introspection_sampler_window_base::sampler::SampleBatch;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_core_state_base::{Loop, Transform};
use compositor_y5_surface_protocol_base::protocol::{SurfaceMessage, SurfaceMessageType};
use compositor_y5_window_interface_record::window::LoopWindow;
use compositor_y5_placeholder_protocol_base::message::PlaceholderAction;
use compositor_y5_placeholder_protocol_base::message::PlaceholderAction::Launch;
use compositor_y5_placeholder_record_base::placeholder::{Placeholder, PlaceholderVisible};
use compositor_y5_placeholder_surface_base::breakpoint::clamp_size;
use compositor_y5_placeholder_surface_base::{PlaceholderMessage, PlaceholderUi};
use compositor_support_smithay_state_window_shell::shell;
use compositor_support_smithay_state_window_ident::ident;

// Whenever a new window is created it must be attached to a placeholder.
//
// CHECK: Unrestore(unset the flag, erase the token) restoring placeholders if they have stalled for over 1 minute.
// CHECK: GC For the tokens

/// `restore`: whether this window may be attached to an existing placeholder.
///
/// `false` for a window an `xdg_toplevel_drag_v1` is already carrying. Everything
/// ABOVE the restoration step still runs — the `NoDisplay` latch and the sampler
/// registration are owed to every toplevel that maps, and the introspection they
/// need is only resolvable here — but a torn-off tab must not be bound to a
/// remembered placeholder. It is under the user's cursor at this instant, so
/// there is nothing to restore it to.
/// How long a launch stays a match candidate.
///
/// `launching` is set when the placeholder is clicked and never cleared — a launch that
/// produces no window (a single-instance app that just focused an existing one, a
/// crash, a splash that never maps) leaves the placeholder armed indefinitely. Long
/// enough to cover a cold start; short enough that the next unrelated window does
/// not get adopted by it.
pub(crate) const LAUNCH_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

/// The same, for a launch that had to START its container first.
///
/// Only that case: a `podman exec` into a container already up is as quick as a
/// host launch and gets the normal grace. Starting one is different — `podman start` then `podman exec`, with an image pull and whatever the
/// container's own init does in between. Thirty seconds is a plausible time for a
/// host binary to map its first window and nowhere near enough for that, and a
/// placeholder that stops matching mid-start hands its window to a fresh placeholder,
/// losing the position and session identity the placeholder carried.
///
/// Matched to the session claim's TTL so the two predicates that associate a
/// window with a launch expire together rather than leaving a window that matches
/// the placeholder but is minted a different session id.
const CONTAINER_LAUNCH_GRACE: std::time::Duration = std::time::Duration::from_secs(120);

pub fn on_window_map_initial(state: &mut Loop, window: Window, restore: bool) -> bool {
    let window_data_0 =
        window.application(&state.inner.space_state().state, &state.inner.loader.display_handle);

    if let Some(data) = window_data_0 {
        // `NoDisplay=true` means no menu will ever offer this program: a portal
        // backend, a MIME handler, a session helper. The user did not launch this
        // window and cannot launch it again, so it must not leave a placeholder.
        //
        // Latched here rather than at destroy because by then the process is gone
        // and its desktop entry is no longer resolvable — and latched from THIS
        // function rather than beside the tearing tag because `window.application`
        // walks /proc and scans every XDG applications dir, and the answer is
        // already in hand right here.
        let no_display = data
            .best_value::<compositor_introspection_extraction_window_base::attributes::NoDisplay>()
            .unwrap_or(false);
        // Both halves matter when this misfires: `no_display=false` with an app_id
        // that looks like a service means the desktop entry did not resolve at all
        // (`find_by_app_id` matches on the filename stem, and a client is free to
        // report a bus name instead), not that the entry said it was displayable.
        trace!(
            "map: app_id={:?} entry_resolved={} no_display={no_display}",
            data.meta.meta.app_id,
            data.has::<compositor_introspection_extraction_window_base::attributes::DesktopEntryPath>()
        );
        // `NoDisplay` is protocol-agnostic — it comes from the desktop entry the
        // window's process resolves to — and now applies to X11 windows too, since
        // `_NET_WM_PID` gives them a real process to resolve. X11 adds a second
        // source the wayland side has no equivalent for: the window's own
        // `_NET_WM_WINDOW_TYPE` / modal / override-redirect declaration
        // (`ident::is_ephemeral_x11`), which is how a menu, tooltip, notification, splash
        // or drag icon says what it is.
        if no_display || ident::is_ephemeral_x11(&window) {
            // Marks both homes; see `mark::mark_window` for why a caller holding a
            // `Window` must not choose between them.
            compositor_support_smithay_state_ephemeral_mark::mark::mark_window(&window);
        }
        if let Some(uuid) = window.uuid() {
            if let Some(sampler) = state.inner.kernel.get(&compositor_orchestration_driver_introspection_base::base::SAMPLER) {
                if let Some(pid) = data.meta.meta.pid {
                    sampler.register(uuid, pid, data.meta.clone());
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
    let mut candidate_session: Option<SessionKey> = None;
    // Every key the client declared, for MATCHING. `candidate_session` above is
    // the CURRENT one and is what a placeholder records; the two differ whenever the
    // client retired the restored name before mapping (Chrome, every run).
    let mut candidate_session_keys: Vec<SessionKey> = vec![];
    // Settings → Misc, read live: "off" | "on" | "all_worlds". Off suppresses the
    // declared-identity match entirely (the launch signals still apply); all_worlds
    // lets a placeholder in ANY world claim the window, and restores it into that world.
    let capture_mode = state.inner.preference.session_capture.clone();
    let capture_all_worlds = capture_mode == "all_worlds";
    // Placeholders offered by a world other than the spawn target: placeholder id -> world.
    // Only consulted in `all_worlds`, and only ever populated with SESSION-bearing
    // placeholders — a launch/capture signal describes a spawn that happened in one world
    // and has no meaning in another.
    let mut foreign_tile_world: HashMap<Uuid, Uuid> = HashMap::new();
    // App name for the cross-world notice, captured before the plan consumes the data.
    let mut notice_app: Option<String> = None;

    if let Some(window_data_0) = window_data_0 {
        // On first-commit:
        let candidate_activations = window.activations();
        // The client-declared `xdg_session_management_v1` identity, if any. Read
        // here rather than inferred later: `restore_toplevel` had to precede the
        // first commit, so it is already on the surface by now.
        candidate_session = window.session();
        // The identity is still RECORDED when capture is off — only the match is
        // suppressed — so turning the setting back on works for placeholders left behind
        // while it was off, instead of starting from nothing.
        if capture_mode != "off" {
            candidate_session_keys = window.session_keys();
        }
        // Captured here because `window_data_0` is consumed into the LaunchPlan
        // below, and re-deriving it costs a /proc walk plus an XDG scan.
        notice_app = window_data_0
            .meta
            .meta
            .app_id
            .clone()
            .or_else(|| window_data_0.meta.meta.title.clone());

        let candidate_token_strings: Vec<&str> =
            candidate_activations.iter().map(|a| a.token.as_str()).collect();

        let mut pending_restoration: Vec<PendingRestoration> = vec![];
        for (ph, _) in &state.inner.placeholder_mut().visible {
            // A placeholder is a match candidate if it's mid-launch (token/PID
            // restoration), has capture-armed attributes (adopt-on-map), OR
            // knows a session identity. A capture-only candidate carries no
            // token and pid `-1`, so neither the token nor the PID-tree signal
            // can spuriously bind it.
            //
            // The session arm is what lets a placeholder claim a window NOBODY here
            // launched — the client came back on its own and re-declared which
            // window it is. The other two can only ever claim our own spawns.
            // Bounded by `launch_at`: nothing ever clears `launching`, so a placeholder
            // whose launch produced no window stays a match candidate forever and
            // can adopt an unrelated app that opens much later. The bound applies
            // ONLY to this predicate — a capture-armed or session-bearing placeholder
            // qualifies on its own terms below and is not time-limited, because
            // those signals identify the window rather than a launch in flight.
            let grace = match ph.launch_started_container {
                true => CONTAINER_LAUNCH_GRACE,
                false => LAUNCH_GRACE,
            };
            let fresh = ph.launch_at.is_some_and(|at| at.elapsed() < grace);
            let is_launching = ph.launching && ph.restoration.is_some() && fresh;
            let is_capture_armed = !compositor_introspection_launchplan_plan_capture::capture::capture_keys(&ph.launch).is_empty();
            if !is_launching && !is_capture_armed && ph.session.is_none() {
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
                session: ph.session.clone(),
                launch_at: ph.launch_at,
            });
        }

        // ALL WORLDS: offer every OTHER world's session-bearing placeholders as candidates
        // too. Restricted to the session arm on purpose — `is_launching` and
        // `is_capture_armed` describe a spawn this world is waiting for, and letting
        // them reach across worlds would let any world's stale launch adopt a window
        // that merely opened somewhere else. A declared identity names the window
        // itself, so it is the only signal that travels.
        if capture_all_worlds && !candidate_session_keys.is_empty() {
            let host = state.inner.worlds.spawn_target();
            for world in state.inner.worlds.ids() {
                if world == host {
                    continue;
                }
                let Some(store) = state
                    .inner
                    .worlds
                    .get(world)
                    .storage()
                    .try_get(&compositor_y5_placeholder_system_base::base::PLACEHOLDER)
                else {
                    continue;
                };
                for (ph, _) in &store.visible {
                    if ph.session.is_none() {
                        continue;
                    }
                    pending_restoration.push(PendingRestoration {
                        id: ph.uuid,
                        plan: ph.launch.clone(),
                        launched_pid: -1,
                        activation_env: HashMap::new(),
                        session: ph.session.clone(),
                        launch_at: ph.launch_at,
                    });
                    foreign_tile_world.insert(ph.uuid, world);
                }
            }
        }

        // println!("CHecking pending restoration");
        // `restore` gates only the MATCH. A carried window still falls through to
        // the branch below that creates a fresh placeholder and records it —
        // skipping that would leave a live window with no record at all, and a
        // window with no record leaves no placeholder when it is eventually closed.
        let matched = restore
            .then(|| {
                match_window(
                    pending_restoration.as_slice(),
                    &window_data_0.meta.clone(),
                    &window_data_0.hints.clone(),
                    &candidate_token_strings,
                    candidate_session_keys.as_slice(),
                    &state.inner.placeholder_mut().restoration_registry,
                )
            })
            .flatten();
        if let Some(placeholder_id) = matched {
            restored_ph = Some(placeholder_id);
            // The bootstrap pid→placeholder claim has served its purpose (or was
            // never needed); drop it so a later, unrelated client cannot inherit
            // this placeholder's session id.
            compositor_support_smithay_state_session_claim::claim::release(placeholder_id);
            // CHECK: Update placeholder state to retain placeholder_id.
            // Remove token from registry.
            // ALL of them: a window may be named by more than one token — which is the
            // whole point for a single-instance app, whose second window arrives through
            // a token its own process never had in its environment — and the launch one
            // is not necessarily the newest. Leaving the others behind strands exactly
            // what the reachability sweep cannot collect, since a client-minted token is
            // exempt there by design.
            for activation in &candidate_activations {
                state
                    .state
                    .xdg_activation
                    .xdg_activation
                    .remove_token(&activation.token);
            }
        }

        window_plan_0 = Some(compositor_introspection_launchplan_plan_base::LaunchPlan::new(window_data_0))
    }

    // Which world owns this map. The spawn target, except when an `all_worlds`
    // match handed the window to a placeholder living somewhere else — then EVERY step
    // below (placeholder erase, iced handle, space, draw order, the new record) has to
    // address that world instead, or the window is mapped into one world holding
    // another's placeholder slot.
    let host_world = state.inner.worlds.spawn_target();
    let target_world = restored_ph
        .and_then(|id| foreign_tile_world.get(&id).copied())
        .unwrap_or(host_world);
    let cross_world = target_world != host_world;

    let mut placeholder = if let Some(placeholder_id) = restored_ph {
        /// Erase restored PH
        let (restored_ph, restored_ph_handle) = state.inner.placeholder_of_mut(target_world)
            .erase_visible(&placeholder_id)
            .unwrap_or_else(|| abort!("Restored PH to exist."));

        // The placeholder holds a slot in the draw-order authority (its
        // z-position within the CONTENT tier). Capture its drawable id before
        // `destroy` consumes the handle so the restored window can inherit that
        // exact slot below (id derived reversibly from the iced HandleId, matching
        // `handle::load`).
        let placeholder_drawable = uuid::Uuid::from_u128(restored_ph_handle.id.0 as u128);

        // Clears out the handle — in the placeholder's OWN world, since the iced registry
        // that minted it is per-world.
        if let Some(ref mut registry) = state
            .inner
            .worlds
            .get_mut(target_world)
            .storage_mut()
            .get_mut(&compositor_y5_surface_system_base::base::SURFACE_MUT)
            .registry
        {
            registry.destroy(restored_ph_handle);
        }
        // Maps window surface into the placeholder's world.
        state.inner.space_of_mut(target_world).state.map_element(
            window.clone(),
            Point::new(restored_ph.position.0, restored_ph.position.1),
            true,
        );
        // MOVE, not copy. `drain_protocol` already mapped this window into the HOST
        // space at (0,0) so commits could find it (`wire.rs`, the `new_toplevels`
        // drain). For a same-world restore the `map_element` above just repositions
        // that entry, but across worlds it creates a SECOND one — the window then
        // exists in both spaces and is drawn, hit-tested and persisted by each.
        // Unmapped after the map, mirroring `picker.world/world.delete`.
        if cross_world {
            state.inner.space_of_mut(host_world).state.unmap_elem(&window);
        }
        // Register in the draw-order authority (restore is a map path). Hand the
        // window the placeholder's EXACT slot (tier + z-position) so it draws
        // where the placeholder was instead of popping to the top of CONTENT — and this
        // GCs the placeholder's otherwise-dangling entry. Fall back to a normal top
        // insert only if the placeholder had no slot (e.g. never registered).
        if let Some(uuid) = window.uuid() {
            use compositor_support_world_order_track_base::base as order;
            let order_mut = state
                .inner
                .worlds
                .get_mut(target_world)
                .storage_mut()
                .get_mut(&order::DRAW_ORDER_MUT);
            if !order_mut.reassign(
                order::ComponentId(placeholder_drawable),
                order::ComponentId(uuid),
            ) {
                order_mut.insert_top(order::ComponentId(uuid), order::DrawLayer::CONTENT);
            }
        }

        let restored_size = Size::new(restored_ph.size.0, restored_ph.size.1);
        shell::stage(&window, restored_size, true);

        // Dialog/child windows (a set xdg `parent` / X11 `WM_TRANSIENT_FOR`) size
        // themselves — leave them `Auto`; lock+grace the rest to the restored size
        // (mirrors `_initial_mapped`).
        if shell::has_parent(&window) {
            compositor_y5_camera_transform_translate::slot::set_expected_auto(&window);
        } else {
            compositor_y5_camera_transform_translate::slot::set_expected_size(&window, restored_size);
            compositor_support_smithay_state_compositor_place::arm_size_propagation(&window, restored_size);
        }

        // At this point, window lifecycle may still attempt to place.
        // Check order.
        // Slight 'flicker' possible because it maps it already. This lifecycle event should act as the initial mapping.

        shell::send(&window);

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
            // Prefer what the client just declared over what we had stored —
            // `xdg_toplevel_session_v1.rename` is allowed to re-key a toplevel,
            // and re-declaring is the only way we learn about it.
            session: candidate_session.clone().or(restored_ph.session),
            // Re-stamped from the LIVE window, not carried from the restored
            // record: the same placeholder can capture an X11 window one run and a
            // wayland one the next (a toolkit switching backends is the ordinary
            // case), and the tint has to say what is in front of the user now.
            from_x11: window.is_x11(),
            // Live too, and for the same reason. A restored record carries none —
            // pixels are not persisted — so this is the only moment it can be taken.
            icon_pixels: window_icon_pixels(&window),
        }
    } else {
        // Otherwise, create a placeholder and attach to the window
        let mut placeholder = Placeholder {
            uuid: Uuid::now_v7(),
            // The layout floor, not an arbitrary number: this is a real size
            // the placeholder can be rendered at if the window never reports one.
            size: clamp_size((0, 0)),
            position: (0, 0),
            launch: window_plan_0,
            launch_session: None,
            session_time: Instant::now(),
            persistent: false,
            // A window we did not restore, but if its client declared an
            // identity we record it now: that is what makes the NEXT close /
            // reopen cycle an exact match instead of a guess.
            session: candidate_session.clone(),
            from_x11: window.is_x11(),
            icon_pixels: window_icon_pixels(&window),
        };
        placeholder
    };

    // grab the window UUID.
    let window_uuid = window.uuid().unwrap_or_else(|| abort!("Windows to have UUID"));
    info!("Insert PH, Window UUID: {:?}", window_uuid);
    state.inner.placeholder_of_mut(target_world).insert(placeholder, window_uuid);
    // Persist the placeholders of the world that actually took the window (incl.
    // not-yet-visible ones). Inserting a placeholder is a discrete, important
    // event → IMMEDIATE (the per-frame sample transform updates below stay
    // debounced).
    // The HOST world (spawn target) for the ordinary path — the world the
    // placeholder was inserted into above — never `active_id()`, which is the
    // lock or picker overlay while one is up; only a cross-world adoption
    // redirects it, since that is the world whose placeholder list changed.
    let persist_world = match cross_world {
        true => target_world,
        false => state.inner.worlds.spawn_target(),
    };
    compositor_support_system_persist_mark_base::base::mark_world(persist_world, true);

    // The window went somewhere the user is not looking. Tell them, or it reads as
    // a launch that silently did nothing.
    if cross_world {
        let app = notice_app.clone().unwrap_or_else(|| String::from("A window"));
        compositor_y5_notify_state_base::base::announce(
            state.inner.kernel_channels(),
            format!("{app} restored window on another world"),
        );
        info!("placeholder: {app} restored into world {target_world} (placeholder from another world)");
    }

    if restored_ph.is_some() {
        return true;
    }

    return false;
    // The window is initially mapped, and the placeholder should track the data
}

/// A window WITHDREW: leave a placeholder for it, and leave the window's record alone.
///
/// The difference from [`on_window_destroy`] is the whole point. A destroy MOVES the
/// record out of `map` and into `visible`, uuid and all, which is what makes a window and
/// its placeholder mutually exclusive — and what lets `PlaceholderState::modify` abort on
/// a missing record rather than skip. A withdrawn X11 window is still alive and may map
/// again, so its record has to stay where it is, and the placeholder has to be a SECOND
/// thing with an identity of its own.
///
/// So the record is COPIED and the copy gets a fresh uuid. Nothing is re-keyed: the
/// window keeps the uuid it has had since it mapped, and persistence sees a new row
/// rather than a renamed one — which matters, because a rename is not a thing the
/// placeholder document can express.
///
/// The consequence, and it is the intended one: each withdrawal leaves its own
/// placeholder. A client that hides and shows repeatedly leaves one per cycle, exactly as
/// a client that closed and reopened a window would.
pub fn on_window_withdraw(
    state: &mut Loop,
    uuid: Uuid,
    renderer: &mut GlesRenderer,
    discard_placeholder: bool,
) {
    // NOT unregistered from the sampler, unlike a destroy: the process is still running
    // and the window may come back, so introspection has something to sample.
    let Some(world) = state.inner.world_of_placeholder(uuid) else {
        return;
    };
    let Some(mut ph) = state.inner.placeholder_of_mut(world).clone_record(&uuid) else {
        return;
    };
    info!("Withdraw PH, Window UUID: {:?}", uuid);
    // The same two refusals a destroy makes, read off the COPY before it is re-keyed.
    if discard_placeholder {
        return;
    }
    if !ph.persistent && ph.session_time.elapsed().lt(&Duration::from_secs(10)) {
        return;
    }
    ph.uuid = Uuid::now_v7();
    let hosted = world == state.inner.worlds.spawn_target();
    compositor_support_system_persist_mark_base::base::mark_world(world, true);
    if !hosted {
        state.inner.placeholder_of_mut(world).pending_restore.push(ph);
        return;
    }
    // `predecessor` is still the WINDOW's uuid: the placeholder inherits its draw-order
    // slot, which the window gives up while it is hidden and takes back on a remap.
    spawn_visible(state, renderer, ph, Some(uuid));
}

pub fn on_window_destroy(
    state: &mut Loop,
    uuid: Uuid,
    renderer: &mut GlesRenderer,
    discard_placeholder: bool,
) {
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
    // The record lives in the world the window was MAPPED in, which is not
    // necessarily the one on screen now — a window is free to exit while the user
    // is looking at another world. Resolving against `spawn_target` here searched
    // the wrong world, found nothing, and dismissed the placeholder the window had
    // earned, leaving its record orphaned in the origin world for good measure.
    let Some(world) = state.inner.world_of_placeholder(uuid) else {
        return;
    };
    let hosted = world == state.inner.worlds.spawn_target();
    // there is a small exception: placeholders that were not previously saved, i.e they werent from a launchplan, should be erased if they lived for less than the grace below (10s).
    let ph = state.inner.placeholder_of_mut(world).erase(&uuid);
    // Erasing a placeholder is a discrete, important event → persist IMMEDIATELY.
    // The record's OWN world, not the active one — they differ whenever a window
    // exits while the user is elsewhere.
    compositor_support_system_persist_mark_base::base::mark_world(world, true);

    // No placeholder wanted. Either the user asked for none (Shift-close from the
    // selection toolbar) or the window was never theirs to begin with — a modal
    // dialog, or a `NoDisplay=true` entry such as the portal file chooser. Both
    // arrive as surface user data read by the wire layer.
    if discard_placeholder {
        return;
    }

    if !ph.persistent && ph.session_time.elapsed().lt(&Duration::from_secs(10)) {
        // Discard it all completely.
        return;
    }

    // The placeholder belongs to the window's own world, at the position it was closed
    // at. Building it needs the renderer AND the world's iced registry, neither of
    // which applies to a world that is not on screen — so hand it to that world's
    // `pending_restore`, which `promote_restored` drains on the first frame that
    // world is the spawn-target. Same deferral the disk-restore path already uses.
    if !hosted {
        state.inner.placeholder_of_mut(world).pending_restore.push(ph);
        compositor_support_system_persist_mark_base::base::mark_world(world, true);
        return;
    }

    // Window's draw-order slot is still live (GC runs after this); placeholder inherits it.
    spawn_visible(state, renderer, ph, Some(uuid));
}

/// Build the visible iced launcher surface for a placeholder and register it in
/// the spawn-target world's `visible` set. Shared by window-destroy (live window →
/// placeholder) and restore (disk → placeholder). No-op if the placeholder has no launch plan.
///
/// `predecessor` is the captured window's UUID: the placeholder inherits its exact
/// draw-order slot (mirror of the restore path). `None` for disk-restored placeholders.
pub fn spawn_visible(
    state: &mut Loop,
    renderer: &mut GlesRenderer,
    mut ph: Placeholder,
    predecessor: Option<Uuid>,
) {
    let Some(plan) = ph.launch.clone() else {
        return; // no launch plan → nothing to relaunch; discard.
    };
    // Clamp HERE, at the boundary, rather than inside the placeholder store:
    // storage stays idempotent and callers own their geometry. This is the one
    // point where a record becomes a rendered placeholder, so it is where a size the
    // UI has no design for has to be corrected — a placeholder inheriting a tiny
    // window's geometry, or rehydrated from a record written before the floor
    // existed. The same `ph` is moved into `push_visible` below, so the
    // surface and the stored record cannot disagree.
    ph.size = clamp_size(ph.size);
    let loc_logical: Point<i32, Logical> = Point::new(ph.position.0, ph.position.1);
    let size_logical: Size<i32, Logical> = Size::new(ph.size.0, ph.size.1);
    let t: Transform = (Rectangle::new(loc_logical, size_logical), state.size_ctx_all()).into();

    // Assign unique ID to the placeholder. this must continue from previous placeholder when it was retained
    let application_registry = state.inner.placeholder().application_registry.clone();
    let handle = compositor_y5_surface_draw_handle::handle::load(
        state,
        renderer,
        PlaceholderUi::new(
            plan,
            ph.launch_session.clone(),
            ph.session.is_some(),
            ph.from_x11,
            ph.icon_pixels.clone(),
            application_registry,
        ),
        t.into_storage_rect_physical(),
        compositor_y5_surface_draw_handle::handle::IcedSpace::World,
        compositor_orchestration_draw_layer_base::base::Layer::SCENE.bits(),
    );

    // `handle::load` inserted the placeholder at CONTENT top; hand it the captured
    // window's exact slot instead. `reassign` drops the placeholder's fresh entry before
    // it finds the window, so re-insert on a miss (window never registered).
    if let Some(win_uuid) = predecessor {
        let placeholder_drawable = uuid::Uuid::from_u128(handle.id.0 as u128);
        if !state.inner.reassign_drawable(win_uuid, placeholder_drawable) {
            state.inner.register_drawable(
                placeholder_drawable,
                compositor_support_world_order_track_base::base::DrawLayer::CONTENT,
            );
        }
    }

    // The placeholder paints its whole rect with an opaque background, so it
    // occludes anything fully behind it — e.g. a previous placeholder retained
    // at the same placeholder. Marking it lets the registry skip rasterizing/compositing
    // the covered one entirely (see `set_opaque_occluder_by_id`).
    state.inner.surface_mut()
        .registry
        .as_mut()
        .unwrap()
        .set_opaque_occluder_by_id(handle.id, true);

    let tx = state.inner.surface_mut().surface_message_buffer_channel.0.clone();
    let ph_uuid = ph.uuid;
    state.inner.surface_mut()
        .registry
        .as_mut()
        .unwrap()
        .set_message_handler(handle, move |message: &PlaceholderMessage| __dispatch(ph_uuid, message, &tx));

    state.inner.placeholder_mut().push_visible(ph, handle);
}

/// Promote the spawn-target world's disk-restored placeholders into visible
/// placeholders, now that the rim has a renderer. Cheap no-op when none are
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
        spawn_visible(state, renderer, ph, None); // disk-restored: no predecessor window
    }
}

/// Keep the session store and the placeholders that carry session identities in
/// step with each other. Runs every frame; both halves are cheap no-ops at rest.
///
/// **Seeding** (once): the store's remembered half — which names exist in which
/// session — lives in the protocol state and dies with the compositor, while the
/// placeholder carrying the same identity is persisted per world. After a restart
/// the two disagree, so `restore_toplevel` finds the session unknown, skips the
/// `restored` event, and the client treats a returning window as new — losing the
/// state the protocol exists to carry. Seeding from the persisted placeholders
/// restores the remembered half before any client can ask.
///
/// **Renames**: `xdg_toplevel_session_v1.rename` re-keys an identity mid-run. The
/// wire layer records it (it cannot reach placeholders); this applies it to every
/// placeholder holding the old key, in EVERY world — the window may have been
/// mapped in one world while its placeholder lives in another, and a stale name stops the
/// client matching on its next run.
///
/// Persistence is marked LAZY: rename cadence is the client's to choose, and an
/// immediate write per rename would let a chatty client drive the disk.
pub fn reconcile_sessions(state: &mut Loop) {
    static SEEDED: std::sync::Once = std::sync::Once::new();
    SEEDED.call_once(|| {
        let mut seen: Vec<(String, String)> = vec![];
        for world in state.inner.worlds.ids() {
            let Some(ph) = state
                .inner
                .worlds
                .get(world)
                .storage()
                .try_get(&compositor_y5_placeholder_system_base::base::PLACEHOLDER)
            else {
                continue;
            };
            let from_map = ph.map.values().filter_map(|rc| rc.borrow().session.clone());
            let from_visible = ph.visible.iter().filter_map(|(v, _)| v.session.clone());
            let from_pending = ph.pending_restore.iter().filter_map(|p| p.session.clone());
            seen.extend(
                from_map
                    .chain(from_visible)
                    .chain(from_pending)
                    .map(|k| (k.session_id, k.name)),
            );
        }
        for (session_id, name) in seen {
            state.state.session.remember(&session_id, &name);
        }
    });

    let renames = std::mem::take(&mut state.state.session_live.renames);
    for (session_id, from, to) in renames {
        let mut touched: Vec<uuid::Uuid> = vec![];
        for world in state.inner.worlds.ids() {
            let ph = state.inner.placeholder_of_mut(world);
            let matches = |k: &Option<SessionKey>| {
                k.as_ref().is_some_and(|k| k.session_id == session_id && k.name == from)
            };
            let mut hit = false;
            for rc in ph.map.values() {
                let mut record = rc.borrow_mut();
                if matches(&record.session) {
                    record.session = Some(SessionKey { session_id: session_id.clone(), name: to.clone() });
                    hit = true;
                }
            }
            for (visible, _) in ph.visible.iter_mut() {
                if matches(&visible.session) {
                    visible.session = Some(SessionKey { session_id: session_id.clone(), name: to.clone() });
                    hit = true;
                }
            }
            for pending in ph.pending_restore.iter_mut() {
                if matches(&pending.session) {
                    pending.session = Some(SessionKey { session_id: session_id.clone(), name: to.clone() });
                    hit = true;
                }
            }
            if hit {
                touched.push(world);
            }
        }
        // The store's remembered half follows too, so the next boot seeds the new
        // name rather than the one the client has stopped using.
        state.state.session.remember(&session_id, &to);
        for world in touched {
            compositor_support_system_persist_mark_base::base::mark_world(world, false);
        }
        info!("session: renamed '{from}' -> '{to}' in session {session_id}");
    }
}

// CHECK : Change to native watch events instead of polling
pub fn on_window_sample(state: &mut Loop, sample: &SampleBatch) {
    // Which worlds a batch actually touched. A batch is not scoped to the focused
    // world, so persisting `active_id` would leave another world's refreshed
    // hints unsaved.
    let mut modified: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    for item in &sample.results {
        let Some(data) = &item.data else { continue };

        // Establish the owning world FIRST. Two reasons, in order of importance:
        // it is what makes the `modify` below safe (that call ABORTS on a missing
        // record — deliberately, as the invariant "a sample belongs to a record"),
        // and no record anywhere means everything after it — cloning the sample,
        // reading the window's surface — is work for nothing.
        let Some(world) = owning_world(state, item.uuid) else {
            continue;
        };

        // The sampler thread has no surface, so its hints carry no toplevel icon.
        // Add it HERE — applying a batch runs on the compositor thread, where the
        // window is reachable — so a sample result is complete for anything
        // reading it. Ordering still holds: the desktop entry resolved on the
        // sampler thread is already in `hints`, so the toplevel's name stays a
        // fallback behind it.
        let mut data = data.clone();
        if let Some(icon) = window_icon(state, item.uuid) {
            push_toplevel_icon_hints(&icon, &mut data.hints);
        }
        // The pixel half, onto the RECORD rather than the hints — see
        // `Placeholder::icon_pixels`. Refreshed here as well as at capture because a
        // client may set its icon after mapping, and this is the pass that notices.
        let pixels = window_of(state, item.uuid).as_ref().and_then(window_icon_pixels);

        modified.insert(world);
        modify_in(state, world, &item.uuid, |placeholder| {
            if pixels.is_some() {
                placeholder.icon_pixels = pixels.clone();
            }
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
    for world in modified {
        compositor_support_system_persist_mark_base::base::mark_world(world, false);
    }
}

/// The world whose placeholder map holds `uuid`, if any.
///
/// Samples are NOT scoped to the focused world: a batch is flushed on the
/// sampler's own cadence and can land while the user is elsewhere, and the world
/// that owns the window is the one whose record has to move. Reading through the
/// focus accessor — as the check here used to — silently drops those.
///
/// This is the precondition for [`modify_in`], which asserts rather than tolerates
/// a missing record.
fn owning_world(state: &Loop, uuid: Uuid) -> Option<Uuid> {
    state.inner.worlds.ids().into_iter().find(|id| {
        state
            .inner
            .worlds
            .get(*id)
            .storage()
            .try_get(&compositor_y5_placeholder_system_base::base::PLACEHOLDER)
            .is_some_and(|placeholders| placeholders.map.contains_key(&uuid))
    })
}

/// Apply `action` to the placeholder for `uuid` in `world`.
///
/// Both accessors here ABORT rather than fall back: the storage slot must exist
/// on a world that has placeholders, and `PlaceholderState::modify` aborts on a
/// missing record. That is the intended shape — the record's existence is an
/// invariant a caller establishes with [`owning_world`], and a silent no-op here
/// would turn a broken invariant into data quietly going missing. The world-scoped
/// mirror of what `placeholder_mut().modify(..)` does for the focused world.
fn modify_in(state: &mut Loop, world: Uuid, uuid: &Uuid, action: impl FnMut(&mut Placeholder)) {
    state
        .inner
        .worlds
        .get_mut(world)
        .storage_mut()
        .get_mut(&compositor_y5_placeholder_system_base::base::PLACEHOLDER_MUT)
        .modify(uuid, action);
}

/// The live window carrying `uuid`, searched across EVERY world for the same
/// reason [`owning_world`] is.
fn window_of(state: &Loop, uuid: Uuid) -> Option<Window> {
    for id in state.inner.worlds.ids() {
        let host = state
            .inner
            .worlds
            .get(id)
            .storage()
            .try_get(&compositor_support_world_host_space_base::base::SPACE)?;
        if let Some(window) = host.inner.state.elements().find(|w| w.uuid() == Some(uuid)) {
            return Some(window.clone());
        }
    }
    None
}

/// The pixel half of a window's own icon, for the placeholder record.
///
/// Split from [`window_icon`] rather than folded into it because the two halves go to
/// different places for different reasons: the NAME is a hint, so it is persisted and
/// re-resolved against the icon theme, while the pixels are session state the record
/// holds directly. `read` is the same call for both shells — `xdg_toplevel_icon_v1`
/// buffers for wayland, `_NET_WM_ICON` for X11 — so nothing here branches on protocol.
fn window_icon_pixels(
    window: &Window,
) -> Option<compositor_introspection_extraction_window_base::IconPixels> {
    compositor_y5_window_interface_record::window::LoopWindow::toplevel_icon(window)?.pixels
}

/// The live toplevel icon of the window carrying `uuid`, NAMED half only.
///
/// The pixel buffers are deliberately dropped here: a placeholder record outlives
/// its window, and a decoded icon is client state that would then be retained —
/// unresolvable, unpersistable and unbounded — for as long as the placeholder exists.
/// A name survives all of that and re-resolves against the icon theme on demand.
fn window_icon(state: &Loop, uuid: Uuid) -> Option<compositor_introspection_extraction_window_base::ToplevelIcon> {
    let icon = window_of(state, uuid)?.toplevel_icon()?;
    Some(compositor_introspection_extraction_window_base::ToplevelIcon {
        name: icon.name,
        pixels: None,
    })
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

    let (w, h) = clamp_size((size.w, size.h));
    state.inner.placeholder_mut().modify(&window_uuid, |placeholder| {
        placeholder.size.0 = w;
        placeholder.size.1 = h;
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
    let size = size.map(clamp_size);

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

    let size = size.map(|s| clamp_size((s.w, s.h)));
    state.inner.placeholder_mut().modify(&window_uuid, |placeholder| {
        if let Some(size) = size {
            placeholder.size.0 = size.0;
            placeholder.size.1 = size.1;
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
        PlaceholderMessage::ContainerStartConfirmed => {
            Some(PlaceholderAction::LaunchStartingContainer())
        }
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
