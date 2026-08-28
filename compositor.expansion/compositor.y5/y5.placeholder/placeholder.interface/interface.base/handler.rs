use smithay::wayland::xdg_activation::XdgActivationTokenData;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_placeholder_protocol_base::message::{PlaceholderAction, PlaceholderMessage};
use compositor_introspection_execution_launch_build_container::build::{
    container_needs_start, request_from_plan_in_container,
};
use compositor_introspection_execution_launch_policy::policy::REQUIRE_PID;
use compositor_kernel_execution_driver_executor_base::executor::EXECUTOR;
use compositor_y5_placeholder_record_base::placeholder::PlaceholderLaunchToken;

pub fn delegate(
    _loop: &mut Loop,
    message: compositor_y5_placeholder_protocol_base::message::PlaceholderMessage,
) {
    match message.action {
        PlaceholderAction::Save(newplan) => {
            // Get the handle and dispatch the message
            let record = _loop.inner.placeholder_mut()
                .modify_visible(&message.uuid, move |placeholder| {
                    placeholder.launch = newplan
                });

            if record.is_none() {
                return;
            }

            let (record, handle) = {
                let (record, handle) = record.unwrap();
                (record.clone(), handle.clone())
            };
            // The user edited a launcher's plan → persist the edit IMMEDIATELY.
            compositor_support_system_persist_mark_base::base::mark_world(_loop.inner.worlds.spawn_target(), true);

            if let Some(registry) = &mut _loop.inner.surface_mut().registry {
                // Push the canonical plan back through the registry, NOT through
                // `instance_mut(..).runtime_mut()`: with iced off-thread the
                // instance lives on the worker and there is no local runtime to
                // borrow, so the reach-through returns `None`. `dispatch_message`
                // queues inline or ships the message to the worker as appropriate.
                if let Err(e) = registry.dispatch_message(
                    handle,
                    compositor_y5_placeholder_surface_base::PlaceholderMessage::UpdatePlan(
                        Box::new(record.launch.clone()),
                    ),
                ) {
                    warn!("placeholder save: plan push-back failed: {e}");
                }
            }
        }
        PlaceholderAction::Erase() => {
            if let Some((_, handle)) = _loop.inner.placeholder_mut().erase_visible(&message.uuid) {
                // Capture the draw-order id before `destroy` consumes the handle.
                let drawable_id = uuid::Uuid::from_u128(handle.id.0 as u128);
                if let Some(ref mut registry) = _loop.inner.surface_mut().registry {
                    registry.destroy(handle);
                }
                // DrawOrder GC: the placeholder surface is world-space (registered).
                _loop.inner.remove_drawable(drawable_id);
                // Dismissing a placeholder is a deletion → persist IMMEDIATELY.
                compositor_support_system_persist_mark_base::base::mark_world(_loop.inner.worlds.spawn_target(), true);
            }
        }
        PlaceholderAction::Launch() => launch(_loop, message.uuid, false),
        PlaceholderAction::LaunchStartingContainer() => launch(_loop, message.uuid, true),
    }
}

/// Launch the placeholder's plan.
///
/// `start_container` is the user's answer to the prompt this raises when the
/// plan targets a stopped container: `false` on the first (plain) Launch, which
/// is the press that puts the prompt up, and `true` when it comes back around
/// via `LaunchStartingContainer`. That split is why a Launch can never start a
/// container without being asked.
fn launch(_loop: &mut Loop, uuid: uuid::Uuid, start_container: bool) {
    let synt = _loop.inner.placeholder_mut().synthesizer_registry.clone();

    // Read the re-entrancy flag BEFORE the mutation, not inside the
    // closure: a `move` closure copies a `bool` capture, so writing the
    // previous value out through it left `was_launch` false forever and
    // the guard never fired — every press re-launched the app and minted
    // a fresh token, orphaning the token the first window will carry.
    let was_launch = _loop
        .inner
        .placeholder()
        .visible
        .iter()
        .any(|(ph, _)| ph.uuid == uuid && ph.launching);

    if was_launch {
        info!("ERR: Launch called while launching");
        return;
    }

    let record = _loop.inner.placeholder_mut()
        .modify_visible(&uuid, |placeholder| {
            placeholder.launching = true;
            placeholder.launch_at = Some(std::time::Instant::now());
            placeholder.launch_started_container = start_container;
        });

    let Some((record, handle)) = record else { return; };
    let (record, handle) = (record.clone(), handle.clone());

    // Containerised plan whose container is down: ask before starting it, and
    // do nothing else this press. Clear `launching` first — the user may answer
    // no, and a placeholder stuck mid-launch would refuse every later press. The
    // confirmed answer arrives as its own action and re-enters here.
    if !start_container {
        if let Some(container) = container_needs_start(&record.launch) {
            _loop.inner.placeholder_mut().modify_visible(&uuid, |ph| {
                ph.launching = false;
                ph.launch_at = None;
                ph.launch_started_container = false;
            });
            if let Some(registry) = &mut _loop.inner.surface_mut().registry {
                let prompt = compositor_y5_placeholder_surface_base::PlaceholderMessage::ConfirmContainerStart { container };
                if let Err(e) = registry.dispatch_message(handle, prompt) {
                    warn!("placeholder launch: container prompt failed: {e}");
                }
            }
            return;
        }
    }

    // Mint the XDG activation token on the calloop thread (Wayland
    // resource); it is the request's correlation token and goes in the
    // child env. The faithful base env is injected by the Executor.
    let app_id = record.launch.application_data.meta.meta.app_id.clone();
    let token_data = XdgActivationTokenData { app_id, ..XdgActivationTokenData::default() };
    let (token, _) = _loop.state.xdg_activation.xdg_activation.create_external_token(token_data);
    let token_str = String::from(token.as_str());
    let extra_env = [
        ("XDG_ACTIVATION_TOKEN".to_string(), token_str.clone()),
        ("DESKTOP_STARTUP_ID".to_string(), token_str.clone()),
    ];

    // Claim the session id HERE, on the click — not when the launch outcome
    // comes back.
    //
    // Dispatch is off-thread, so `executor.launch` returns before the process
    // exists and the outcome (with its pid) arrives some frames later. A client
    // that starts quickly connects and asks for its session id in that window,
    // finds no claim registered, and is minted a fresh uuid — losing the identity
    // the placeholder restores under, permanently, because the client then
    // persists the new one. A small test binary wins that race every time.
    //
    // Registering with no pid works because the activation token is a predicate
    // in its own right: it is already in the child's environment, which is where
    // `resolve` looks first. `on_executed` upserts the pid into this same claim
    // when it lands, so the pid-tree fallback still works for a client that
    // consumed the token before we could read it.
    compositor_support_smithay_state_session_claim::claim::register(
        None,
        uuid,
        record.session.as_ref().map(|s| s.session_id.clone()),
        token_str.clone(),
    );

    // `_in_container` is a no-op for a plan with no container attributes, so
    // this is the single build path for host and containerised launches alike.
    let req = match request_from_plan_in_container(
        &record.launch, &synt, &extra_env, token_str.clone(), Some(uuid), start_container,
    ) {
        Ok(req) => req,
        Err(e) => {
            warn!("launch build failed: {e}");
            return;
        }
    };
    // Launch via the kernel executor driver. Inline dispatch returns the
    // outcome (incl. the PID) synchronously; off-thread returns `None`.
    let immediate = if let Some(executor) = _loop.inner.kernel.get(&EXECUTOR).as_ref() {
        executor.launch(req)
    } else {
        warn!("launch executor unavailable; launch dropped");
        return;
    };

    // The activation token is known synchronously (we just minted it), so
    // arm restoration NOW — even for off-thread dispatch — so a fast-mapping
    // window still matches on the token before the Executed event arrives.
    // The PID is folded in only if available immediately (inline); otherwise
    // the Executed listener fills it in later by correlation (idempotent).
    let immediate_pid = immediate.as_ref().and_then(|o| o.pid);
    let mut restoration = Some(PlaceholderLaunchToken {
        token: token_str,
        child: if REQUIRE_PID { immediate_pid } else { None },
    });
    _loop.inner.placeholder_mut().modify_visible(&uuid, move |placeholder| {
        if let Some(t) = restoration.take() {
            placeholder.restoration = Some(t);
        }
    });
    info!("Launch!");
}
