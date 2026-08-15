//! Descriptor aggregation helpers.
//!
//! The UI uses these to enumerate every editor section to draw for a
//! given application. Categorized into:
//! - **Identity**: applies regardless of which handler is active.
//! - **Launch**: the generic exec primitives.
//! - **Handler-scoped**: only relevant when a specific handler is active.

use compositor_introspection_extraction_window_base::{
    attributes::{
        AppId, DBusActivatable, DBusServiceName, DesktopEntryPath, DetectedHandler, DisplayName,
        EnvOverlay, ExecArgs, ExecProgram, IconName, IconPath, Sandbox, Title, WorkingDirectory,
        XdgIconName,
    },
    AppHandler, AttributeDescriptor, HandlerRegistry, HintAttribute,
};

/// Descriptors for the identity-scoped attributes built into the
/// `compositor_introspection_extraction_window_base` crate, in display order.
pub fn identity_descriptors() -> Vec<AttributeDescriptor> {
    vec![
        DisplayName::descriptor(),
        Title::descriptor(),
        AppId::descriptor(),
        IconPath::descriptor(),
        IconName::descriptor(),
        XdgIconName::descriptor(),
        DesktopEntryPath::descriptor(),
        DetectedHandler::descriptor(),
        Sandbox::descriptor(),
        DBusActivatable::descriptor(),
        DBusServiceName::descriptor(),
    ]
}

/// Descriptors for the launch-scoped attributes built into the
/// `compositor_introspection_extraction_window_base` crate, in display order.
pub fn launch_descriptors() -> Vec<AttributeDescriptor> {
    vec![
        ExecProgram::descriptor(),
        ExecArgs::descriptor(),
        WorkingDirectory::descriptor(),
        EnvOverlay::descriptor(),
    ]
}

/// All descriptors that apply when the given handler is active, in
/// display order: identity, then launch, then handler-scoped.
///
/// If the handler isn't registered, returns identity + launch only.
pub fn all_descriptors_for(
    registry: &HandlerRegistry,
    handler: Option<compositor_introspection_extraction_window_base::HandlerId>,
) -> Vec<AttributeDescriptor> {
    let mut out = identity_descriptors();
    out.extend(launch_descriptors());
    if let Some(handler_id) = handler {
        if let Some(h) = registry.get(handler_id) {
            out.extend(h.attribute_descriptors());
        }
    }
    out
}
