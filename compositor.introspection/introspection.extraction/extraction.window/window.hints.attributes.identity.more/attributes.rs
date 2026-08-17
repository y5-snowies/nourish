//! Identity attributes carried by the live surface (not derived from the
//! process tree): the window title and the Wayland app id / X11 WM_CLASS.
//! Both are `AttributeCategory::Identity` so they edit and persist exactly
//! like the attributes in the sibling `attributes.identity` crate; they
//! exist primarily so a placeholder can capture a new window by matching
//! its title and/or app id.

use compositor_introspection_extraction_window_hints_attribute::attribute::HintAttribute;
use compositor_introspection_extraction_window_hints_descriptor::descriptor::{AttributeDescriptor, AttributeKind};
use compositor_introspection_extraction_window_hints_id::category::AttributeCategory;

/// Live window title (xdg_toplevel title / X11 `WM_NAME`).
#[derive(Debug)]
pub struct Title;
impl HintAttribute for Title {
    type Value = String;
    fn name() -> &'static str { "title" }
    fn category() -> AttributeCategory { AttributeCategory::Identity }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "Window title", Self::category(), AttributeKind::Text) }
}

/// `NoDisplay=true` on the resolved desktop entry: the entry exists so the
/// desktop knows how to start the program, but no menu ever offers it — DBus
/// service backends (xdg-desktop-portal), MIME handlers, session helpers.
///
/// A window from such an entry is not one the user launched and not one they
/// can launch again, which is what makes it worth recording separately from
/// `DBusActivatable` (many ordinary apps are D-Bus activatable too).
#[derive(Debug)]
pub struct NoDisplay;
impl HintAttribute for NoDisplay {
    type Value = bool;
    fn name() -> &'static str { "no_display" }
    fn category() -> AttributeCategory { AttributeCategory::Identity }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "No display", Self::category(), AttributeKind::Bool) }
}

/// Wayland `app_id` (xdg-shell) or X11 `WM_CLASS` surfaced via xwayland.
#[derive(Debug)]
pub struct AppId;
impl HintAttribute for AppId {
    type Value = String;
    fn name() -> &'static str { "app_id" }
    fn category() -> AttributeCategory { AttributeCategory::Identity }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "App ID", Self::category(), AttributeKind::Text) }
}
