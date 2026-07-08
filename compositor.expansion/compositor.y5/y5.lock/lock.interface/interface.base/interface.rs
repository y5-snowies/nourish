use std::time::Duration;
use std::time::Instant;

use crate::surface;
use crate::three;
use smithay::backend::input::KeyState;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::input::keyboard::FilterResult;
use smithay::input::keyboard::ModifiersState;
use smithay::reexports::calloop::timer::TimeoutAction;
use smithay::reexports::calloop::timer::Timer;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::SERIAL_COUNTER;
use smithay::utils::{Physical, Point, Scale, Size};
use compositor_orchestration_core_state_base::state::Status;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_graphic_capture_registry::{CaptureSource, OutputId};
use compositor_y5_lock_state_base::state::{LockActiveCapture, LockActiveState};
use compositor_monitor_compositor_iced_base::IcedHandle;

/// LOGICAL lock engage — renderer-free, so a lock holds even when the compositor
/// is dark (no output → no render). The keybinding handler sets
/// `Status::Locked{pending:true}` SYNCHRONOUSLY and schedules this on `insert_idle`
/// (NOT a per-frame flag). The VISUAL (lock screen + capture backdrop + bevy) is
/// built lazily by [`lock_visual`] on the next real frame. Reads `sleep` from the
/// status the handler already set.
/// Drain the one-shot lock-engage request. The lock/sleep keybindings set
/// `Status::Locked{pending}` + `lock_engage` synchronously (they can't call this
/// crate — it depends back on the keyboard crates) and wake the control-plane
/// ping; this runs from that ping (off the input AND render paths), so the lock
/// engages event-driven, not on a per-frame/per-dispatch poll.
pub fn drain_engage(state: &mut Loop) {
    if !state.inner.lock_engage {
        return;
    }
    state.inner.lock_engage = false;
    lock_logical(state);
}

pub fn lock_logical(state: &mut Loop) {
    let compositor_orchestration_core_state_base::state::Status::Locked { .. } = state.inner.status
    else {
        // Status must already be set by the handler; nothing to engage otherwise.
        return;
    };
    // Already engaged (active set) → idempotent no-op (e.g. double idle).
    if state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).active.is_some() {
        return;
    }

    // Stop any in-progress capture before the lock takes its own snapshot, so the
    // capture overlays don't bleed into the lock background.
    compositor_y5_graphic_capture_interface::interface::stop_and_discard(state);

    compositor_y5_navigator_interface_base::interface::lock(state);

    // Empty visual — filled by `lock_visual` once a renderer is available.
    let active = LockActiveState {
        bevy: None,
        capture: LockActiveCapture::None,
        surface: vec![],
        surface_input: None,
        fold_started: false,
    };

    // Locking IS a world switch: session systems get on_disable, lock systems
    // on_enable. The Status enum stays alongside until the legacy scene/input
    // selection migrates onto the active world (then it dissolves).
    {
        let (worlds, kernel) = (&mut state.inner.worlds, &state.inner.kernel);
        worlds.switch(compositor_y5_lock_system_base::base::LOCK_WORLD, kernel);
    }

    state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).active = Some(active);

    deactivate_scene(state);

    // Pending→done after PERIOD, on a loop timer (NOT the per-frame render hook).
    let delay = Duration::from_secs_f64(compositor_y5_lock_state_transition::transition::PERIOD);
    let _ = state.loop_handle.insert_source(Timer::from_duration(delay), |_, _, state| {
        lock_done_logical(state);
        TimeoutAction::Drop
    });
}

/// VISUAL lock build — needs the GLES renderer, so it runs from the lock render
/// path (`lock.scene/scene.frame::prepare`), lazily: the lock-screen surface +
/// capture backdrop are built once `active` exists, and the bevy morph once the
/// lock is no longer pending. When dark this simply doesn't run; the visuals
/// appear on the first frame after a monitor returns.
pub fn lock_visual(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    let pending = match state.inner.status {
        compositor_orchestration_core_state_base::state::Status::Locked { pending, .. } => pending,
        _ => return,
    };
    if state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).active.is_none() {
        return;
    }

    let need_surface = state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).active.as_ref().unwrap().surface_input.is_none();
    if need_surface {
        let mut surface_element = vec![];
        let mut surface_input: Option<
            IcedHandle<compositor_y5_lock_interface_surface::view::LockSurface>,
        > = None;
        if let Some(surface) = surface::create(state, renderer, size) {
            surface_element.push(surface.id);
            surface_input = Some(surface);
        }
        let output_id = OutputId::from_key(&state.inner.active_output_key());
        let capture = if let Some(registry_capture) = state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY_MUT).as_mut() {
            if let Some(cap) = registry_capture
                .request(&state.inner.environment.GPU.as_str(), renderer, CaptureSource::OutputFramebuffer(output_id))
                .ok()
            {
                LockActiveCapture::Capture(cap)
            } else {
                LockActiveCapture::None
            }
        } else {
            LockActiveCapture::None
        };
        let active = state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).active.as_mut().unwrap();
        active.surface = surface_element;
        active.surface_input = surface_input;
        active.capture = capture;
    }

    // Create the morph while the originating session is STILL rendered (this may
    // run during `pending`): the snapshot plane warms up (cast import + first
    // render) ON TOP of the live desktop, so there is never a frame where the
    // session is hidden but the plane is not yet up. It samples the LIVE output
    // capture, so during the warm-up it mirrors the still-visible desktop. Gated
    // on the capture being live (not on the iced surface) — the plane's own
    // `render_ready` shader gate keeps it transparent until the cast is imported,
    // by which point the capture entry has been filled at least once.
    let need_bevy = {
        let active = state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).active.as_ref().unwrap();
        active.bevy.is_none() && matches!(active.capture, LockActiveCapture::Capture(_))
    };
    if need_bevy {
        let bevy = three::create(state, renderer, size);
        state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).active.as_mut().unwrap().bevy = bevy;
    }

    // Start the fold exactly once, at the transition where the session scene is
    // dropped (`!pending`). By now the opaque snapshot plane already covers the
    // screen, so the fold begins with no gap. (Dark-boot case: the morph was
    // created just above at `!pending`; the fold then starts on the next frame.)
    if !pending {
        let (handle, started) = {
            let active = state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).active.as_ref().unwrap();
            (active.bevy, active.fold_started)
        };
        if let (Some(handle), false) = (handle, started) {
            three::start_fold(state, handle);
            state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).active.as_mut().unwrap().fold_started = true;
        }
    }
}

fn deactivate_scene(_loop: &mut Loop) {
    // Before dropping focus, release held (non-modifier) keys to the focused client so a client
    // that tracks its own keyboard state doesn't get stuck key-down on re-focus after the lock.
    compositor_orchestration_seat_keyboard_input::keyboard::release_held_keys(_loop);

    let keyboard = _loop.state.seat.seat.get_keyboard().unwrap();
    let serial = SERIAL_COUNTER.next_serial();

    // Deactivate keyboard focus
    keyboard.set_focus(&mut _loop.state, Option::<WlSurface>::None, serial);

    _loop.inner.space_state().state.elements().for_each(|window| {
        window.set_activated(false);
        window.toplevel().unwrap().send_pending_configure();
    });

    // CHECK: Ice Registry should be per scene, as well as space, etc.
    //        this is due to occur when space is encapsulated ( layers feature ).
    // CHECK: IcedRegistry has pointer_grab. it should probably be released as well.
    if let Some(registry) = _loop.inner.surface_mut().registry.as_mut() {
        // Iced deactivation:
        registry.set_keyboard_focus(None);
        // registry.dispatch_button(None, button, true);
    }

    clear_keyboard(_loop);
}
fn clear_keyboard(_loop: &mut Loop) {
    let keyboard = &_loop.state.seat.seat.get_keyboard().unwrap();
    let serial = SERIAL_COUNTER.next_serial();

    let now = _loop.inner.start_time.elapsed().as_millis() as u32;
    for key in keyboard.pressed_keys() {
        // confirm accessor name in your tree
        keyboard.input::<(), _>(
            &mut _loop.state,
            key, // Keycode
            KeyState::Released,
            SERIAL_COUNTER.next_serial(),
            now,
            |_, _, _| FilterResult::Intercept(()), // update internal state, don't forward
        );
    }

    // 3. Clear modifier state directly (belt-and-suspenders).
    keyboard.set_modifier_state(ModifiersState::default());
}

/// Pending→done transition — renderer-free, run from the loop timer armed in
/// [`lock_logical`] (NOT the per-frame render hook). Marks the lock complete and
/// creates the PAM worker (a calloop source) so authentication works even while
/// dark. The bevy morph is built later by [`lock_visual`] (it needs a renderer).
fn lock_done_logical(state: &mut Loop) {
    let compositor_orchestration_core_state_base::state::Status::Locked { sleep, time, pending } =
        state.inner.status
    else {
        return;
    };
    if !pending {
        return; // already done (idempotent)
    }

    if state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).active.is_none() {
        abort!("Lock done while not locked ( no active set )");
    }

    // Progress to locked complete
    state.inner.status = compositor_orchestration_core_state_base::state::Status::Locked {
        pending: false,
        sleep: false,
        time,
    };

    // set lock time on ParallaxBackground (the session world being locked ==
    // spawn_target; locking only moved `active` to LOCK_WORLD).
    let session = state.inner.worlds.spawn_target();
    if let Some(ref mut instance) = state.inner.worlds.get_mut(session).storage_mut().get_mut(&compositor_background_two_storage_base::base::BG_TWO_MUT).instance {
        instance.lock_time = Some(Instant::now());
        instance.pan = (0.0, 0.0);
        instance.zoom = 1.0;
    }

    state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).pam = crate::pam::make_pam(&state.loop_handle);

    // It was set as sleep, so insert a timer to callback after period to lock
    if sleep {
        // CHECK: THis isn't the actual period for the animation of morph
        let period = compositor_y5_lock_state_transition::transition::PERIOD * 2.0;
        let delay = Duration::from_secs_f64(period);
        let timer_source = Timer::from_duration(delay);

        // 2. Insert it into the loop handle
        let _ = state
            .loop_handle
            .insert_source(timer_source, |deadline, _metadata, state| {
                // IT should block shortcuts already.
                let Some(tty) = &mut state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).tty else {
                    return TimeoutAction::Drop;
                };
                let _ = tty.suspend();

                TimeoutAction::Drop
            });
    }
}

pub fn unlock_fail(state: &mut Loop) {
    info!("Unlock");
    match state.inner.status {
        compositor_orchestration_core_state_base::state::Status::Locked { .. } => {}
        _ => return,
    }

    let handle = state
        .inner
        .worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT)
        .active
        .as_ref()
        .unwrap()
        .surface_input
        .unwrap();
    let registry = state.inner.surface_mut().registry.as_mut().unwrap();
    registry.dispatch_message(
        handle,
        compositor_y5_lock_interface_surface::message::LockMessage::AuthFailed(String::from(
            "Invalid",
        )),
    );
}
pub fn unlock(state: &mut Loop) {
    info!("Unlock");
    match state.inner.status {
        compositor_orchestration_core_state_base::state::Status::Locked { .. } => {}
        _ => return,
    }

    let reg = state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).pam.as_ref().and_then(|w| Some(w.1));
    if let Some(reg) = reg {
        state.loop_handle.remove(reg);
    }

    state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).pam = None;

    let destroy_ids = {
        let active = &state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).active.as_ref().unwrap();
        active.surface.clone()
    };

    if let Some(registry) = state.inner.surface_mut().registry.as_mut() {
        for item in destroy_ids {
            registry.destroy_by_id(item);
        }
    }

    let destroy_ids = {
        let active = &state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).active.as_ref().unwrap();
        active.bevy.and_then(|w| Some(w.id)).clone()
    };

    // The morph instance lives in the LOCK world's OWN registry now.
    if let Some(registry) = state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().try_get_mut(&compositor_background_three_system_base::base::BG_THREE_MUT).and_then(|b| b.registry.as_mut()) {
        if let Some(bevy) = destroy_ids {
            registry.destroy_by_id(bevy);
        }
    }

    let session = state.inner.worlds.spawn_target();
    if let Some(bg) = &mut state.inner.worlds.get_mut(session).storage_mut().get_mut(&compositor_background_two_storage_base::base::BG_TWO_MUT).instance {
        bg.lock_time = None;
    }

    state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).active = None;
    // state.inner.status = Status::Unlock {
    //     time: Instant::now(),
    // };

    state.inner.status = Status::Running;
    // Unlocking switches back to the session world (kept intact while locked):
    // spawn_target still names it, since locking only moved `active`.
    {
        let (worlds, kernel) = (&mut state.inner.worlds, &state.inner.kernel);
        let session = worlds.spawn_target();
        worlds.switch(session, kernel);
    }

    clear_keyboard(state);
    compositor_y5_navigator_interface_base::interface::unlock(state);

    // for window in _loop.inner.space_state().state.elements() {
    //     window.set_activated(false);
    //     if let Some(toplevel) = window.toplevel() {
    //         toplevel.send_pending_configure();
    //     }
    // }
}
