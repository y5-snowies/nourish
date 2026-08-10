//! Bodies of the gRPC `Shader` service: read what the active world is running,
//! list what it could run, switch it, and re-read the current one from disk.
//!
//! # Why these four
//!
//! They are the loop an external author needs and nothing more: see the current
//! state, discover the alternatives, select one, and — the one that is not
//! obvious — force a re-read after editing the files. A bundle is compiled once
//! at selection and then never restatted, so without [`reload`] an edit to a
//! loaded bundle has no effect until something else happens to invalidate it.
//!
//! # Everything goes through the world's own slot
//!
//! Not through a second activation path. `SettingsMessage::SetWorldShader` writes
//! exactly these three fields, and so does this — a remote caller and the settings
//! panel must not be able to leave the world in states the other cannot produce.
//!
//! The three writes are one operation:
//!
//! * `background_shader` — the selection, persisted per world;
//! * `instance = None`   — drop the compiled bundle so `TwoSystem::update`
//!   rebuilds on the next frame. THIS is what makes a change take effect;
//! * `mark_world`        — persist.
//!
//! # Active world only
//!
//! A background is a per-world setting and every one of these calls means "the
//! world the user is looking at". Taking a world id would be a second way to name
//! something the compositor already knows, and a stale one would silently edit a
//! world nobody can see.

use compositor_orchestration_core_state_base::Loop;
use compositor_remote_message_client_base::bind::shader::{
    ActivateRequest, ActivateResponse, ActiveRequest, ActiveResponse, Bundle, ListRequest,
    ListResponse, Prop, ReloadRequest, ReloadResponse,
};

use compositor_background_two_storage_base::base::{BG_TWO, BG_TWO_MUT};

/// A selection naming a compiled-in bundle cannot be edited on disk, and saying
/// so is most of what a caller needs from `path`.
fn embedded(selection: &str) -> bool {
    compositor_pipeline_bundle_embed_base::embed::bundle_of(selection).is_some()
}

/// Where a selection's sources are, or empty when there are none to edit.
fn path_of(selection: &str) -> String {
    if selection.is_empty() || embedded(selection) {
        return String::new();
    }
    compositor_pipeline_bundle_locate_base::resolve_ref(selection)
        .to_string_lossy()
        .into_owned()
}

pub fn active(_request: ActiveRequest, state: &mut Loop) -> ActiveResponse {
    let Some(two) = state.inner.worlds.active().storage().try_get(&BG_TWO) else {
        return ActiveResponse::default();
    };
    let selection = two.background_shader.clone().unwrap_or_default();
    let mut out = ActiveResponse {
        path: path_of(&selection),
        builtin: embedded(&selection),
        error: two.shader_error.clone().unwrap_or_default(),
        selection,
        ..Default::default()
    };

    // Graph shape and variables come from the LOADED bundle, not from the file:
    // what is reported is what is running. A selection that failed to compile
    // reports its `error` and an empty graph, which is the honest answer — the
    // desktop is showing the fallback.
    if let Some(cp) = two.bundle() {
        out.before_passes = cp.before.len() as u32;
        out.after_passes = cp.after.len() as u32;
        out.targets = cp.targets.len() as u32;
        out.windows = format!("{:?}", cp.owns).to_lowercase();
        out.requires = compositor_pipeline_abi_seam_base::base::Requirement::ALL
            .iter()
            .filter(|r| cp.requires.has(**r))
            .map(|r| r.to_string())
            .collect();
        out.props = cp
            .properties
            .iter()
            .map(|p| Prop {
                // The live value is the world's override when it has one, else the
                // declared default — the same resolution the settings panel does,
                // so the two never disagree about what a slider is showing.
                value: two
                    .params
                    .iter()
                    .find(|(n, _)| *n == p.name)
                    .map(|(_, v)| *v)
                    .unwrap_or_else(|| p.default.as_f32()),
                name: p.name.clone(),
                kind: p.default.kind().to_string(),
                label: p.label.clone().unwrap_or_default(),
                group: p.group.clone().unwrap_or_default(),
                default: p.default.as_f32(),
                min: p.min.unwrap_or_default(),
                max: p.max.unwrap_or_default(),
                step: p.step.unwrap_or_default(),
                choices: p.choices.clone(),
            })
            .collect();
    }
    out
}

pub fn list(_request: ListRequest, _state: &mut Loop) -> ListResponse {
    use compositor_pipeline_bundle_embed_base::embed;
    // Compiled-in first, then the folders, which is the order the picker shows
    // them and keeps the shipped set from being lost among however many bundles
    // the user has dropped in.
    let mut bundles: Vec<Bundle> = embed::BUNDLES
        .iter()
        .map(|name| {
            let selection = embed::id_of(name);
            Bundle {
                category: compositor_pipeline_bundle_load_base::category_for(&selection),
                selection,
                name: name.to_string(),
                builtin: true,
                path: String::new(),
                multipass: true,
            }
        })
        .collect();
    for name in compositor_pipeline_bundle_locate_base::list_bundles() {
        let dir = compositor_pipeline_bundle_locate_base::resolve_ref(&name);
        bundles.push(Bundle {
            category: compositor_pipeline_bundle_load_base::category_for(&name),
            multipass: embed::has_manifest(&dir),
            path: dir.to_string_lossy().into_owned(),
            name: name.clone(),
            selection: name,
            builtin: false,
        });
    }
    ListResponse { bundles }
}

pub fn activate(request: ActivateRequest, state: &mut Loop) -> ActivateResponse {
    let world = state.inner.worlds.active_id();
    let Some(two) =
        state.inner.worlds.active_mut().storage_mut().try_get_mut(&BG_TWO_MUT)
    else {
        return ActivateResponse {
            applied: false,
            error: "this world has no background slot".into(),
        };
    };
    // Empty clears the override, exactly as the settings panel's "default" entry
    // does — not a separate "reset" call, because it is not a separate state.
    two.background_shader =
        (!request.selection.is_empty()).then(|| request.selection.clone());
    // Clearing the error too: it describes the PREVIOUS selection, and leaving it
    // would report a failure against a bundle that has not been tried yet.
    two.shader_error = None;
    two.instance = None;
    compositor_support_system_persist_mark_base::base::mark_world(world, true);
    // `applied` is "the world now names this", not "it compiled" — the load
    // happens on the next frame, and the caller reads `active().error` for that.
    ActivateResponse { applied: true, error: String::new() }
}

pub fn reload(_request: ReloadRequest, state: &mut Loop) -> ReloadResponse {
    let world = state.inner.worlds.active_id();
    let Some(two) =
        state.inner.worlds.active_mut().storage_mut().try_get_mut(&BG_TWO_MUT)
    else {
        return ReloadResponse {
            applied: false,
            error: "this world has no background slot".into(),
        };
    };
    if two.background_shader.is_none() {
        // The built-in is compiled in; there is no file to have edited. Refusing
        // is better than returning success for work that cannot have happened.
        return ReloadResponse {
            applied: false,
            error: "this world is on the built-in shader; nothing to reload".into(),
        };
    }
    // The whole operation: drop the compiled instance and let the next frame read
    // the bundle again. `background_shader` is untouched, so this is a re-read of
    // the same selection rather than a change of it.
    two.shader_error = None;
    two.instance = None;
    // Not persisted — nothing about the world changed, only what is in memory.
    let _ = world;
    ReloadResponse { applied: true, error: String::new() }
}
