use smithay::wayland::xdg_activation::XdgActivationTokenData;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_placeholder_protocol_base::message::{PlaceholderAction, PlaceholderMessage};
use compositor_introspection_execution_launch_build::build::request_from_plan;
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
            compositor_support_system_persist_mark_base::base::mark_world(_loop.inner.worlds.active_id(), true);

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
                // Dismissing a launcher tile is a deletion → persist IMMEDIATELY.
                compositor_support_system_persist_mark_base::base::mark_world(_loop.inner.worlds.active_id(), true);
            }
        }
        PlaceholderAction::Launch() => {
            let synt = _loop.inner.placeholder_mut().synthesizer_registry.clone();

            let mut was_launch = false;
            let record = _loop.inner.placeholder_mut()
                .modify_visible(&message.uuid, move |placeholder| {
                    was_launch = placeholder.launching;
                    placeholder.launching = true
                });

            if was_launch {
                info!("ERR: Launch called while launching");
                return;
            }
            let Some((record, _handle)) = record else { return; };
            let record = record.clone();

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

            let req = match request_from_plan(&record.launch, &synt, &extra_env, token_str.clone(), Some(message.uuid)) {
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
            _loop.inner.placeholder_mut().modify_visible(&message.uuid, move |placeholder| {
                if let Some(t) = restoration.take() {
                    placeholder.restoration = Some(t);
                }
            });
            info!("Launch!");
        }
    }
}
