//! Each attribute is a zero-sized marker type implementing `HintAttribute`,
//! in `AttributeCategory::Identity` (name, icon, sandbox, desktop entry).

use compositor_introspection_extraction_window_hints_attribute::attribute::HintAttribute;
use compositor_introspection_extraction_window_hints_descriptor::descriptor::{AttributeDescriptor, AttributeKind};
use compositor_introspection_extraction_window_hints_id::category::AttributeCategory;
use compositor_introspection_extraction_window_hints_id::handler_id::HandlerId;
use compositor_introspection_extraction_window_hints_values::values::SandboxIdentity;
use std::path::PathBuf;

/// Human-readable name for the app/window.
#[derive(Debug)]
pub struct DisplayName;
impl HintAttribute for DisplayName {
    type Value = String;
    fn name() -> &'static str { "display_name" }
    fn category() -> AttributeCategory { AttributeCategory::Identity }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "Display name", Self::category(), AttributeKind::Text).invalidated_by(&["app_id"]) }
}

/// Absolute path to a `.desktop` file describing this app.
#[derive(Debug)]
pub struct DesktopEntryPath;
impl HintAttribute for DesktopEntryPath {
    type Value = PathBuf;
    fn name() -> &'static str { "desktop_entry_path" }
    fn category() -> AttributeCategory { AttributeCategory::Identity }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "Desktop entry", Self::category(), AttributeKind::Path).invalidated_by(&["app_id"]) }
}

/// Which handler the registry detected for this window.
#[derive(Debug)]
pub struct DetectedHandler;
impl HintAttribute for DetectedHandler {
    type Value = HandlerId;
    fn name() -> &'static str { "detected_handler" }
    fn category() -> AttributeCategory { AttributeCategory::Identity }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "Detected handler", Self::category(), AttributeKind::Custom("handler_id")) }
}

/// Resolved icon file path on disk.
#[derive(Debug)]
pub struct IconPath;
impl HintAttribute for IconPath {
    type Value = PathBuf;
    fn name() -> &'static str { "icon_path" }
    fn category() -> AttributeCategory { AttributeCategory::Identity }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "Icon file", Self::category(), AttributeKind::Path).invalidated_by(&["app_id", "desktop_entry_path"]) }
}

/// Icon name from the desktop entry (pre-resolution).
#[derive(Debug)]
pub struct IconName;
impl HintAttribute for IconName {
    type Value = String;
    fn name() -> &'static str { "icon_name" }
    fn category() -> AttributeCategory { AttributeCategory::Identity }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "Icon name", Self::category(), AttributeKind::Text).invalidated_by(&["app_id", "desktop_entry_path"]) }
}

/// Icon name the client declared over `xdg_toplevel_icon_v1` (pre-resolution).
///
/// Sits BESIDE [`IconName`] rather than replacing it: the desktop entry's icon
/// identifies the application, this one identifies the toplevel, and an app may
/// have both (or disagree with itself). Which one becomes the [`IconPath`] is
/// decided in `window.hints.extract.entry` — the desktop entry wins, and this
/// is resolved only when the entry gave nothing that exists on disk.
#[derive(Debug)]
pub struct XdgIconName;
impl HintAttribute for XdgIconName {
    type Value = String;
    fn name() -> &'static str { "xdg_icon_name" }
    fn category() -> AttributeCategory { AttributeCategory::Identity }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "Toplevel icon name", Self::category(), AttributeKind::Text) }
}

/// Sandbox identity (Flatpak/Snap/etc).
#[derive(Debug)]
pub struct Sandbox;
impl HintAttribute for Sandbox {
    type Value = SandboxIdentity;
    fn name() -> &'static str { "sandbox" }
    fn category() -> AttributeCategory { AttributeCategory::Identity }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "Sandbox", Self::category(), AttributeKind::Custom("sandbox")) }
}

/// Whether the app supports `org.freedesktop.Application` D-Bus activation.
#[derive(Debug)]
pub struct DBusActivatable;
impl HintAttribute for DBusActivatable {
    type Value = bool;
    fn name() -> &'static str { "dbus_activatable" }
    fn category() -> AttributeCategory { AttributeCategory::Identity }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "D-Bus activatable", Self::category(), AttributeKind::Bool).invalidated_by(&["app_id", "desktop_entry_path"]) }
}

/// D-Bus service name (typically derived from the desktop file's stem).
#[derive(Debug)]
pub struct DBusServiceName;
impl HintAttribute for DBusServiceName {
    type Value = String;
    fn name() -> &'static str { "dbus_service_name" }
    fn category() -> AttributeCategory { AttributeCategory::Identity }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "D-Bus service name", Self::category(), AttributeKind::Text) }
}
