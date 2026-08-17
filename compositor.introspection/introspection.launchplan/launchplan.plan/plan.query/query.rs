use std::collections::HashMap;
use std::io;
use std::process::{Command, Stdio};
use compositor_introspection_extraction_window_base::attributes::{DetectedHandler, EnvOverlay, ExecArgs, ExecProgram, WorkingDirectory};
use compositor_introspection_extraction_window_base::{AttributeCategory, HandlerId, HintAttribute};
use compositor_introspection_inference_hint_base::ApplicationData;
use compositor_introspection_launchplan_plan_preferences::Preferences;

pub fn initial_handler(application_data: &ApplicationData) -> Option<HandlerId> {
    application_data.best_value::<DetectedHandler>()
}

/// Effective value: preferences over inferred best; disabled => `None`.
pub fn current<A: HintAttribute>(
    global_preferences: &Preferences,
    handler_preferences: &HashMap<HandlerId, Preferences>,
    active_handler: Option<HandlerId>,
    application_data: &ApplicationData,
) -> Option<A::Value> {
    match A::category() {
        AttributeCategory::Identity | AttributeCategory::Launch => {
            let inferred = application_data.best_value::<A>();
            // An attribute the user has never touched has no entry, and its
            // enabled state defaults from whether there is anything to
            // enable. Defaulting to `true` unconditionally meant a stray
            // override on an attribute with no hint — an empty string left
            // behind by clearing the editor — still resolved to a value.
            //
            // Once the user touches the attribute an entry exists, so their
            // explicit choice always wins over this default.
            if !global_preferences.is_enabled_or::<A>(inferred.is_some()) { return None; }
            global_preferences.get::<A>().or(inferred)
        }
        AttributeCategory::HandlerScoped(handler_id) => {
            if active_handler != Some(handler_id) { return None; }
            let inferred = application_data.best_value::<A>();
            let prefs = handler_preferences.get(&handler_id);
            // Same hint-aware default as the arm above, so a handler-scoped
            // attribute resolves by the same rule as a global one.
            let enabled = prefs
                .map(|p| p.is_enabled_or::<A>(inferred.is_some()))
                .unwrap_or(inferred.is_some());
            if !enabled { return None; }
            prefs.and_then(|p| p.get::<A>()).or(inferred)
        }
    }
}

pub fn prefs_for<'a>(
    global_preferences: &'a Preferences,
    handler_preferences: &'a HashMap<HandlerId, Preferences>,
    active_handler: Option<HandlerId>,
    category: &AttributeCategory,
) -> Option<&'a Preferences> {
    match category {
        AttributeCategory::Identity | AttributeCategory::Launch => Some(global_preferences),
        AttributeCategory::HandlerScoped(handler_id) => {
            if active_handler != Some(*handler_id) { return None; }
            handler_preferences.get(handler_id)
        }
    }
}

pub fn prefs_for_mut<'a>(
    global_preferences: &'a mut Preferences,
    handler_preferences: &'a mut HashMap<HandlerId, Preferences>,
    category: &AttributeCategory,
) -> &'a mut Preferences {
    match category {
        AttributeCategory::Identity | AttributeCategory::Launch => global_preferences,
        AttributeCategory::HandlerScoped(handler_id) => {
            handler_preferences.entry(*handler_id).or_insert_with(Preferences::new)
        }
    }
}

/// Generic exec from program + args; `NotFound` if no program resolvable.
pub fn generic_command(
    program: Option<<ExecProgram as HintAttribute>::Value>,
    args: Option<<ExecArgs as HintAttribute>::Value>,
) -> Result<Command, io::Error> {
    let program = program
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "launch plan has no program"))?;
    let mut cmd = Command::new(program);
    cmd.args(args.unwrap_or_default());
    Ok(cmd)
}

// `run_command` (spawn the plan) moved to the introspection.execution subsystem;
// `generic_command` above is still used to synthesize the bare Command.
