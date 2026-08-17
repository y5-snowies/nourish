//! Container-aware launch building: the same request the host path produces,
//! rewritten to run inside the container the window came from.
//!
//! Layered ON TOP of `launch.build` rather than folded into it, so the host
//! path stays free of any container knowledge and this crate can depend on both
//! the builder and the podman wrapper without a cycle.

use uuid::Uuid;

use compositor_introspection_extraction_window_hints_container_query::query;
use compositor_introspection_launchplan_plan_base::{LaunchPlan, SynthesizerRegistry};
use compositor_introspection_execution_launch_build::build::request_from_plan;
use compositor_introspection_execution_launch_types::types::LaunchRequest;

/// The container this plan should be launched into. Re-exported from the
/// shared plan query so the launch path and the placeholder UI cannot drift
/// apart on what "has a container" means — they did, and the tile ended up
/// showing a container badge for a launch that built a broken command.
///
/// A disabled attribute reads as absent (that is what `current` does), so
/// unticking Container name/id in the placeholder's settings is how a user
/// forces a host launch. `None` — no container — is the normal case.
pub use compositor_introspection_launchplan_plan_container::container::container_of;

/// `Some(container)` when this plan targets a container that exists but is NOT
/// running, so the caller must ask before starting it: starting a container
/// reruns its entrypoint, well beyond "open this window again".
///
/// `None` covers no-container and already-running, and also the cases we cannot
/// determine (podman absent, container removed) — prompting to start something
/// unstartable helps nobody, so the launch proceeds and fails loudly instead.
///
/// BLOCKING: it shells out to the container runtime. Call it from a discrete
/// user action, never from a per-frame or per-map path.
pub fn container_needs_start(plan: &LaunchPlan) -> Option<String> {
    let container = container_of(plan)?;
    match query::is_running_blocking(&container) {
        Some(false) => Some(container),
        _ => None,
    }
}

/// Build a launch request, TAGGED with the plan's container when it has one.
/// Identical to `request_from_plan` for a plan with no container attributes,
/// so callers need only this one entry point.
///
/// The `podman exec` rewrite itself happens later, in the Executor — see
/// [`LaunchRequest::container`] for why it cannot happen here.
pub fn request_from_plan_in_container(
    plan: &LaunchPlan,
    synthesizers: &SynthesizerRegistry,
    extra_env: &[(String, String)],
    token: String,
    correlation: Option<Uuid>,
    start_container: bool,
) -> Result<LaunchRequest, std::io::Error> {
    let mut req = request_from_plan(plan, synthesizers, extra_env, token, correlation)?;
    req.container = container_of(plan);
    req.start_container = req.container.is_some() && start_container;
    Ok(req)
}
