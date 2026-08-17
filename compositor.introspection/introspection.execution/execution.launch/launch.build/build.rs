//! `request_from_plan` — synthesize the command and layer the overlays.

use uuid::Uuid;

use compositor_introspection_extraction_window_base::attributes::{EnvOverlay, ExecArgs, ExecProgram, WorkingDirectory};
use compositor_introspection_launchplan_plan_base::exec::{sanitise_unit_name, short_random};
use compositor_introspection_launchplan_plan_base::{LaunchPlan, SynthesizerRegistry};
use compositor_introspection_launchplan_plan_query::query;
use compositor_introspection_execution_launch_types::types::LaunchRequest;

/// Resolve `plan` into a ready-to-spawn request. `extra_env` is layered last
/// (e.g. the activation token). `correlation` ties the eventual outcome back to
/// an originator (a placeholder uuid) or `None`.
pub fn request_from_plan(
    plan: &LaunchPlan,
    synthesizers: &SynthesizerRegistry,
    extra_env: &[(String, String)],
    token: String,
    correlation: Option<Uuid>,
) -> Result<LaunchRequest, std::io::Error> {
    // Same selection as the old execute_with_env: handler synthesis, else generic.
    let from_synth = plan.active_handler.and_then(|id| synthesizers.get(id)).and_then(|s| s.synthesize(plan));
    let cmd = match from_synth {
        Some(c) => c,
        None => query::generic_command(plan.current::<ExecProgram>(), plan.current::<ExecArgs>())?,
    };

    // argv + any env/cwd the synthesizer baked into the Command.
    let mut argv = vec![cmd.get_program().to_string_lossy().into_owned()];
    argv.extend(cmd.get_args().map(|a| a.to_string_lossy().into_owned()));
    let mut env: Vec<(String, String)> = cmd
        .get_envs()
        .filter_map(|(k, v)| v.map(|v| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned())))
        .collect();
    let mut working_dir = cmd.get_current_dir().map(|p| p.to_string_lossy().into_owned());

    // Overlay in the historical order: plan working-dir overrides, then the plan
    // env overlay, then the caller's extras.
    if let Some(wd) = plan.current::<WorkingDirectory>() {
        working_dir = Some(wd.to_string_lossy().into_owned());
    }
    for pair in plan.current::<EnvOverlay>().unwrap_or_default() {
        env.push((pair.key, pair.value));
    }
    env.extend(extra_env.iter().cloned());

    let basename = argv.first().map(String::as_str).unwrap_or("app");
    let unit = format!("y5-app-{}-{}", sanitise_unit_name(basename), short_random());

    // Host launch; `launch.build.container` fills the container fields in when
    // the plan targets one.
    Ok(LaunchRequest {
        argv, env, working_dir, token, unit, correlation,
        container: None,
        start_container: false,
    })
}
