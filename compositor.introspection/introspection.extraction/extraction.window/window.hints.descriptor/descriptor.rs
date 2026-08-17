//! Used by higher crates to enumerate attributes for a given handler and
//! decide how to render an editor for each.

use compositor_introspection_extraction_window_hints_id::category::AttributeCategory;

/// Static metadata for one attribute. Renders independently of any
/// particular value.
#[derive(Debug, Clone)]
pub struct AttributeDescriptor {
    /// Stable key, equal to the `HintAttribute::name` string.
    pub key: &'static str,
    /// Human-readable label for the UI.
    pub label: &'static str,
    /// Scope (identity / launch / handler-specific).
    pub category: AttributeCategory,
    /// What kind of editor to render.
    pub kind: AttributeKind,
    /// Attribute keys whose change makes this attribute's INFERRED value
    /// suspect — because it was derived from them, or because it contradicts
    /// them.
    ///
    /// Declared here rather than in a central table so the fact lives beside
    /// the attribute that owns it: a new derived attribute cannot be added
    /// without the edge, and a central list cannot silently fall behind.
    ///
    /// Optional. An empty list means "nothing invalidates this", which is the
    /// correct answer for anything extracted straight from `/proc` or the
    /// Wayland surface. Declaring an edge only buys the report; whether the
    /// value can actually be RE-DERIVED is a separate, also-optional
    /// capability — see the invalidation pass.
    pub invalidated_by: &'static [&'static str],
}

/// What kind of value an attribute carries, from the UI's perspective.
///
/// The UI uses this to decide which widget to render. Custom kinds carry
/// a string tag so the UI can dispatch on it (e.g., `"chrome_profile"`).
#[derive(Debug, Clone)]
pub enum AttributeKind {
    /// Free-form text input.
    Text,
    /// Filesystem path. UI may show a file picker.
    Path,
    /// Boolean toggle.
    Bool,
    /// Ordered list of strings. UI shows add/remove/reorder controls.
    StringList,
    /// One of a fixed set of strings. UI shows a dropdown.
    EnumOf(Vec<&'static str>),
    /// UI-specific rendering. The string tag identifies which renderer.
    Custom(&'static str),
}

impl AttributeDescriptor {
    pub fn new(
        key: &'static str,
        label: &'static str,
        category: AttributeCategory,
        kind: AttributeKind,
    ) -> Self {
        Self { key, label, category, kind, invalidated_by: &[] }
    }

    /// Declare what makes this attribute's inferred value suspect. Builder so
    /// the common case — nothing does — stays a plain `new`.
    pub fn invalidated_by(mut self, sources: &'static [&'static str]) -> Self {
        self.invalidated_by = sources;
        self
    }
}
