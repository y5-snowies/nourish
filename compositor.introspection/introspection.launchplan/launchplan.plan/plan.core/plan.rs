//! [`LaunchPlan`] — the persistent unit of compositor placeholder state.
use crate::synthesizer::SynthesizerRegistry;
use compositor_introspection_extraction_window_base::attributes::{EnvOverlay, ExecArgs, ExecProgram, WorkingDirectory};
use compositor_introspection_extraction_window_base::{AttributeCategory, AttributeDescriptor, HandlerId, HintAttribute};
use compositor_introspection_inference_hint_base::ApplicationData;
use compositor_introspection_launchplan_plan_preferences::Preferences;
use compositor_introspection_launchplan_plan_query::query;
use std::{any::{Any, TypeId}, collections::HashMap, io, process::Command, sync::Arc};

#[derive(Debug, Clone)]
pub struct LaunchPlan {
    pub application_data: ApplicationData,
    pub global_preferences: Preferences,
    pub handler_preferences: HashMap<HandlerId, Preferences>,
    pub active_handler: Option<HandlerId>,
}

impl LaunchPlan {
    pub fn new(application_data: ApplicationData) -> Self {
        let active_handler = query::initial_handler(&application_data);
        Self { application_data, global_preferences: Preferences::new(), handler_preferences: HashMap::new(), active_handler }
    }
    pub fn current<A: HintAttribute>(&self) -> Option<A::Value> {
        query::current::<A>(&self.global_preferences, &self.handler_preferences, self.active_handler, &self.application_data)
    }
    pub fn handler_prefs_mut(&mut self, handler_id: HandlerId) -> &mut Preferences { self.handler_preferences.entry(handler_id).or_insert_with(Preferences::new) }
    pub fn active_handler_prefs(&self) -> Option<&Preferences> { self.active_handler.and_then(|id| self.handler_preferences.get(&id)) }
    pub fn set_active_handler(&mut self, handler: Option<HandlerId>) {
        if let Some(id) = handler { self.handler_preferences.entry(id).or_default(); }
        self.active_handler = handler;
    }
    // Launch execution moved to the introspection.execution subsystem; the plan
    // now only exposes the data (program/args/env/cwd) it builds the command from.
    // ── Dynamic (string-keyed) accessors, for descriptor-driven UIs ──
    pub fn current_raw(&self, descriptor: &AttributeDescriptor) -> Option<Arc<dyn Any + Send + Sync>> {
        let prefs = self.prefs_for(&descriptor.category)?;
        if !prefs.is_enabled_by_name(descriptor.key) { return None; }
        prefs.get_raw(descriptor.key).or_else(|| self.application_data.hints.best_raw(descriptor.key))
    }
    pub fn set_pref_raw(&mut self, descriptor: &AttributeDescriptor, value: Arc<dyn Any + Send + Sync>, type_id: TypeId) {
        self.prefs_for_mut(&descriptor.category).set_raw(descriptor.key, value, type_id);
    }
    pub fn set_enabled_raw(&mut self, descriptor: &AttributeDescriptor, enabled: bool) {
        self.prefs_for_mut(&descriptor.category).set_enabled_by_name(descriptor.key, enabled);
    }
    pub fn is_pref_enabled(&self, descriptor: &AttributeDescriptor) -> bool {
        match self.prefs_for(&descriptor.category) {
            Some(p) => p.is_enabled_by_name(descriptor.key),
            None => false,
        }
    }
    pub fn clear_pref_raw(&mut self, descriptor: &AttributeDescriptor) { self.prefs_for_mut(&descriptor.category).clear_by_name(descriptor.key); }
    pub fn best_raw(&self, descriptor: &AttributeDescriptor) -> Option<Arc<dyn Any + Send + Sync>> { self.application_data.hints.best_raw(descriptor.key) }
    fn prefs_for(&self, category: &AttributeCategory) -> Option<&Preferences> { query::prefs_for(&self.global_preferences, &self.handler_preferences, self.active_handler, category) }
    fn prefs_for_mut(&mut self, category: &AttributeCategory) -> &mut Preferences { query::prefs_for_mut(&mut self.global_preferences, &mut self.handler_preferences, category) }
}

pub mod synthesizer {
    //! [`LaunchSynthesizer`]: per-handler command synthesis.
    use crate::plan::LaunchPlan;
    use compositor_introspection_extraction_window_base::HandlerId;
    use std::{collections::HashMap, process::Command, sync::Arc};

    /// Build a runnable `Command` from a `LaunchPlan`. Return `None` to
    /// decline; the plan falls back to generic exec.
    pub trait LaunchSynthesizer: Send + Sync {
        fn handler_id(&self) -> HandlerId;
        fn synthesize(&self, plan: &LaunchPlan) -> Option<Command>;
    }

    /// Registry of synthesizers, keyed by [`HandlerId`].
    #[derive(Default)]
    pub struct SynthesizerRegistry {
        synthesizers: HashMap<HandlerId, Arc<dyn LaunchSynthesizer>>,
    }

    impl SynthesizerRegistry {
        pub fn new() -> Self { Self::default() }
        pub fn register<S: LaunchSynthesizer + 'static>(&mut self, synthesizer: S) -> &mut Self {
            let id = synthesizer.handler_id();
            self.synthesizers.insert(id, Arc::new(synthesizer));
            self
        }
        pub fn get(&self, id: HandlerId) -> Option<&dyn LaunchSynthesizer> { self.synthesizers.get(&id).map(|a| a.as_ref()) }
        pub fn ids(&self) -> impl Iterator<Item = HandlerId> + '_ { self.synthesizers.keys().copied() }
        pub fn len(&self) -> usize { self.synthesizers.len() }
        pub fn is_empty(&self) -> bool { self.synthesizers.is_empty() }
    }
}
