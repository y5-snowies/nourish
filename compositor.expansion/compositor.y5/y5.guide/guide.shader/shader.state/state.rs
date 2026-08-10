//! What the inline shader editor shows, resolved from the ACTIVE world.
//!
//! Separate from the reconciler so the reconciler is only surface lifecycle. This
//! is the read side of the same per-world `Two` slot the Settings World tab reads;
//! both go through `message::shader_props`, so a variable is offered as the same
//! control in both panels.

use compositor_orchestration_core_state_base::Loop;
use compositor_y5_guide_shader_view::ShaderSnapshot;

/// Resolve the active world's background shader into the editor's snapshot.
///
/// Properties come from the LOADED bundle when there is one — it holds the union
/// the running shader is indexed by, costs nothing to read, and cannot be a stale
/// copy. `properties_for` (which re-reads the bundle off disk) is the fallback for
/// a selection whose element has not been built yet.
pub fn snapshot(state: &Loop) -> ShaderSnapshot {
    let two = state
        .inner
        .worlds
        .active()
        .storage()
        .try_get(&compositor_background_two_storage_base::base::BG_TWO);
    let current = two
        .and_then(|t| t.background_shader.clone())
        .or_else(compositor_model_stats_registry_base::base::background_shader_default);
    let overrides = two.map(|t| t.params.clone()).unwrap_or_default();
    let props = match (two.and_then(|t| t.props()), &current) {
        (Some(p), _) => p.to_vec(),
        (None, Some(sel)) => compositor_pipeline_bundle_load_base::properties_for(sel),
        (None, None) => compositor_pipeline_bundle_builtin_base::builtin_props(),
    };
    ShaderSnapshot {
        name: current.unwrap_or_default(),
        about: two.and_then(|t| t.bundle()).map(about).unwrap_or_default(),
        props: compositor_configurator_settings_surface_message::message::shader_props(
            &props, &overrides,
        ),
    }
}

/// One line of what the bundle IS. The Settings World tab has room for the full
/// properties card; this panel has room for the two facts that change how the
/// variables under it behave — who draws the world band, and which thread runs it.
fn about(
    cp: &compositor_pipeline_build_pipeline_base::pipeline::CompiledPipeline,
) -> String {
    use compositor_pipeline_build_place_base::place::Offload;
    use compositor_pipeline_abi_seam_base::base::WorldOwn;
    let owns = match cp.owns {
        WorldOwn::Engine => "engine band",
        WorldOwn::Windows => "shader draws windows",
        WorldOwn::World => "shader draws the world band",
    };
    let place = match cp.offload {
        Offload::None => "compositor thread",
        _ => "background thread",
    };
    format!("{} pass · {owns} · {place}", cp.before.len() + cp.after.len())
}
