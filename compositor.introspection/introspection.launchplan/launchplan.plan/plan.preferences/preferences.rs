//! Per-attribute preference store, type-erased. Mirrors the storage
//! pattern of `InferredHints`: name-keyed map of optional `Arc<dyn Any>`
//! payloads with a TypeId for safe downcast.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;
use compositor_introspection_extraction_window_base::HintAttribute;

pub use compositor_introspection_launchplan_plan_preferences_field::PreferenceField;

/// Per-attribute preference store, keyed by [`HintAttribute::name`].
/// Empty by default; only attributes the user has touched have entries.
#[derive(Default, Debug, Clone)]
pub struct Preferences {
    fields: HashMap<&'static str, PreferenceField>,
}

impl Preferences {
    pub fn new() -> Self { Self::default() }
    /// Set a typed override; `get::<A>()` then returns it over inference.
    pub fn set<A: HintAttribute>(&mut self, value: A::Value) {
        let field = self.fields.entry(A::name()).or_default();
        field.override_value = Some(Arc::new(value));
        field.override_type_id = Some(TypeId::of::<A::Value>());
    }
    /// User-set override, or `None` if unset/disabled (fall back to inference).
    pub fn get<A: HintAttribute>(&self) -> Option<A::Value> {
        let field = self.fields.get(A::name())?;
        if !field.enabled { return None; }
        let v = field.override_value.as_ref()?;
        if field.override_type_id? != TypeId::of::<A::Value>() {
            return None; // type mismatch; should not happen in normal use
        }
        v.downcast_ref::<A::Value>().cloned()
    }
    /// Whether the attribute is enabled (default true if no entry).
    pub fn is_enabled<A: HintAttribute>(&self) -> bool {
        self.fields.get(A::name()).map(|f| f.enabled).unwrap_or(true)
    }
    /// Typed form of [`Preferences::is_enabled_by_name_or`].
    pub fn is_enabled_or<A: HintAttribute>(&self, default: bool) -> bool {
        self.fields.get(A::name()).map(|f| f.enabled).unwrap_or(default)
    }
    pub fn set_enabled<A: HintAttribute>(&mut self, enabled: bool) {
        self.fields.entry(A::name()).or_default().enabled = enabled;
    }
    /// True if the user has set an explicit override (regardless of enabled).
    pub fn has_override<A: HintAttribute>(&self) -> bool {
        self.fields.get(A::name()).map(|f| f.has_override()).unwrap_or(false)
    }
    /// Remove all preference state for an attribute.
    pub fn clear<A: HintAttribute>(&mut self) { self.fields.remove(A::name()); }
    /// Remove only the override, keeping the enabled state intact.
    pub fn clear_override<A: HintAttribute>(&mut self) {
        if let Some(field) = self.fields.get_mut(A::name()) {
            field.override_value = None;
            field.override_type_id = None;
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &PreferenceField)> {
        self.fields.iter().map(|(k, v)| (*k, v))
    }
    pub fn len(&self) -> usize { self.fields.len() }
    pub fn is_empty(&self) -> bool { self.fields.is_empty() }
    // ── String-keyed accessors (for UI use) ──────────────────────
    /// Override Arc by name; `None` if no entry or disabled.
    pub fn get_raw(&self, name: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        let field = self.fields.get(name)?;
        if !field.enabled { return None; }
        field.override_value.clone()
    }
    /// Set an override by name with explicit TypeId.
    pub fn set_raw(&mut self, name: &'static str, value: Arc<dyn Any + Send + Sync>, type_id: TypeId) {
        let field = self.fields.entry(name).or_default();
        field.override_value = Some(value);
        field.override_type_id = Some(type_id);
    }
    pub fn is_enabled_by_name(&self, name: &str) -> bool {
        self.fields.get(name).map(|f| f.enabled).unwrap_or(true)
    }
    /// As [`Preferences::is_enabled_by_name`], but the caller supplies the
    /// default for an attribute with no entry. Callers that can see the
    /// inferred hints pass "a hint exists": an attribute nothing knows
    /// anything about has nothing to enable, and showing it ticked claims
    /// otherwise.
    pub fn is_enabled_by_name_or(&self, name: &str, default: bool) -> bool {
        self.fields.get(name).map(|f| f.enabled).unwrap_or(default)
    }
    /// Whether the user has set an explicit override, by name.
    pub fn has_override_by_name(&self, name: &str) -> bool {
        self.fields.get(name).map(|f| f.has_override()).unwrap_or(false)
    }
    /// Drop the override, keeping `enabled` / `capture`. The distinct gesture
    /// for "use the inferred value" — clearing the editor text cannot mean
    /// that, because it has to keep meaning "override with an empty value".
    pub fn clear_override_by_name(&mut self, name: &str) {
        if let Some(field) = self.fields.get_mut(name) {
            field.override_value = None;
            field.override_type_id = None;
        }
    }
    pub fn set_enabled_by_name(&mut self, name: &'static str, enabled: bool) {
        self.fields.entry(name).or_default().enabled = enabled;
    }
    /// Whether the attribute is capture-armed (default false if no entry).
    pub fn is_capture_by_name(&self, name: &str) -> bool {
        self.fields.get(name).map(|f| f.capture).unwrap_or(false)
    }
    pub fn set_capture_by_name(&mut self, name: &'static str, capture: bool) {
        self.fields.entry(name).or_default().capture = capture;
    }
    pub fn clear_by_name(&mut self, name: &str) { self.fields.remove(name); }
}
