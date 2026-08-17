use smithay::backend::renderer::gles::GlesRenderer;
use compositor_introspection_launchplan_plan_base::exec::{sanitise_unit_name, short_random};
use compositor_orchestration_core_state_base::Loop;
use compositor_introspection_execution_launch_types::types::LaunchRequest;
use compositor_kernel_execution_driver_executor_base::executor::EXECUTOR;
use compositor_y5_launcher_protocol_message::message::{
    ExternalAction, InternalAction, LauncherMessage, Source,
};

pub fn handle(
    _loop: &mut Loop,
    renderer: &mut GlesRenderer,
    message: compositor_y5_launcher_protocol_message::message::LauncherMessage,
) {
    match message.message {
        Source::Internal(Type) => match Type {
            InternalAction::Start => {
                compositor_y5_launcher_interface_base::interface::start(_loop, renderer);
            }
        },
        Source::External(Message) => {
            match Message {
                ExternalAction::Start { id, bin, args, direction: _ } => {
                    let unit = format!("y5-app-{}-{}", sanitise_unit_name(&id), short_random());
                    let mut argv = vec![bin.to_string_lossy().into_owned()];
                    argv.extend(args);

                    // Plain launcher tile: no correlation (nothing restores it),
                    // no extra env — the Executor injects the faithful base env
                    // (WAYLAND_DISPLAY, XDG_CURRENT_DESKTOP, …).
                    let req = LaunchRequest {
                        argv,
                        env: Vec::new(),
                        working_dir: None,
                        token: String::new(),
                        unit,
                        correlation: None,
                        container: None,
                        start_container: false,
                    };
                    if let Some(executor) = _loop.inner.kernel.get(&EXECUTOR).as_ref() {
                        executor.launch(req);
                    } else {
                        warn!("launch executor unavailable; launch dropped");
                    }

                    let Some(handle) = _loop.inner.launcher_mut().handle else {
                        return;
                    };
                    let Some(ref mut registry) = _loop.inner.surface_mut().registry else {
                        return;
                    };
                    registry.destroy(handle);
                    _loop.inner.launcher_mut().handle = None;
                }
                ExternalAction::Exit => {
                    let Some(handle) = _loop.inner.launcher_mut().handle else {
                        return;
                    };
                    let Some(ref mut registry) = _loop.inner.surface_mut().registry else {
                        return;
                    };

                    registry.destroy(handle);
                    _loop.inner.launcher_mut().handle = None;
                }
            }
        }
    }
}
