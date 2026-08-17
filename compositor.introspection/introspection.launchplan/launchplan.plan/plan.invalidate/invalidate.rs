//! Refresh the attributes made suspect by one the user just changed.
//!
//! Two layers, and only the first is mandatory:
//!
//! 1. **The graph is declared by the attributes themselves.** Each
//!    `AttributeDescriptor` carries `invalidated_by`, so the edge lives beside
//!    the attribute that owns it. This pass just walks the descriptors the
//!    registry already enumerates for the active handler — there is no central
//!    table to fall behind, and a new derived attribute cannot be added
//!    without its edge.
//!
//! 2. **Re-derivation is optional, per source.** A source with a re-deriver
//!    gets its dependents recomputed; one without still gets them REPORTED as
//!    suspect. That is the useful half on its own — the terminal handler maps
//!    app_id to a kind through `starts_with` and an `Unknown(String)`
//!    catch-all, so choosing a kind can never tell us the new executable, and
//!    no amount of machinery will change that. Saying "this is stale" is the
//!    honest answer, and it costs one line of declaration.
//!
//! Overrides are never touched. Only the inferred layer is refreshed; a value
//! the user typed is their decision. The refreshed hints appear as options in
//! the attribute's picker instead.

use compositor_introspection_extraction_window_base::attributes::{AppId, ContainerName};
use compositor_introspection_extraction_window_base::hints::extract::push_desktop_hints;
use compositor_introspection_extraction_window_base::{
    AttributeDescriptor, Confidence, HandlerRegistry, HintAttribute, Meta, SourceMethod,
};
use compositor_introspection_extraction_window_hints_container_query::query;
use compositor_introspection_inference_hint_base::all_descriptors_for;
use compositor_introspection_launchplan_plan_base::LaunchPlan;
use compositor_introspection_launchplan_plan_container::container::container_of;

/// One dependent the change touched.
pub struct Refreshed {
    pub attribute: &'static str,
    /// False when no re-deriver exists for the source, so the value is merely
    /// reported as suspect rather than recomputed.
    pub recomputed: bool,
}

/// Refresh everything `changed` invalidates. Returns what was touched so the
/// UI can say why a field moved.
pub fn on_changed(
    plan: &mut LaunchPlan,
    changed: &str,
    registry: &HandlerRegistry,
) -> Vec<Refreshed> {
    let dependents: Vec<AttributeDescriptor> = all_descriptors_for(registry, plan.active_handler)
        .into_iter()
        .filter(|d| d.invalidated_by.contains(&changed))
        .collect();
    if dependents.is_empty() {
        return Vec::new();
    }

    let recomputed = re_derive(plan, changed);

    dependents
        .into_iter()
        .map(|d| Refreshed { attribute: d.key, recomputed })
        .collect()
}

/// Recompute what `changed` feeds, if we know how. `false` means the
/// dependents are only reported.
///
/// Every arm here needs no `MetaNode`: it re-derives from an attribute VALUE
/// plus the filesystem or the container runtime. That is what lets
/// invalidation behave identically after a restart, where the process
/// snapshot is gone.
fn re_derive(plan: &mut LaunchPlan, changed: &str) -> bool {
    match changed {
        "app_id" => refresh_from_app_id(plan),
        "container_id" => refresh_container_name(plan),
        _ => false,
    }
}

/// Re-run the desktop-entry extractor against the CURRENT app_id.
///
/// Calls the real extractor with a synthetic `Meta` carrying only the app_id,
/// which is the single field `push_desktop_hints` reads. Re-deriving by hand
/// would duplicate knowledge that would then drift; this cannot.
fn refresh_from_app_id(plan: &mut LaunchPlan) -> bool {
    let Some(app_id) = plan.current::<AppId>() else { return false };

    // Drop only the hints THIS extractor produced. Filtering on the source
    // method keeps unrelated hints for the same attribute — a display name
    // that came from `/proc/<pid>/comm`, say — which a blanket removal by
    // attribute name would have destroyed.
    plan.application_data
        .hints
        .items
        .retain(|item| item.source.method != SourceMethod::DesktopEntry);

    let probe = Meta { app_id: Some(app_id), ..Meta::default() };
    push_desktop_hints(&probe, &mut plan.application_data.hints);
    true
}

/// Re-resolve the container name for the current container id.
fn refresh_container_name(plan: &mut LaunchPlan) -> bool {
    plan.application_data
        .hints
        .items
        .retain(|item| item.attr_name != ContainerName::name());

    let Some(id) = container_of(plan) else { return false };
    let Some(name) = query::name_for(&id) else { return false };
    plan.application_data.hints.push::<ContainerName>(
        name,
        SourceMethod::ProcCgroup,
        "re-resolved from the container id",
        Confidence::High,
    );
    true
}
