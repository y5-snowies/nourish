//! Container identity, in `AttributeCategory::Identity`. Present only when the
//! window's process was found to live inside an OCI container.
//!
//! Both the id and the name are carried, deliberately: the ID is what `/proc`
//! actually yields (it is in the cgroup path, so it costs nothing and is always
//! available), while the NAME is the stable handle — a container ID changes
//! every time the container is recreated, so a placeholder persisted across a
//! `podman rm && podman run` would hold a dead ID but a live name. Launch
//! prefers the name and falls back to the id.

use compositor_introspection_extraction_window_hints_attribute::attribute::HintAttribute;
use compositor_introspection_extraction_window_hints_descriptor::descriptor::{AttributeDescriptor, AttributeKind};
use compositor_introspection_extraction_window_hints_id::category::AttributeCategory;

/// Full container ID, as parsed out of `/proc/<pid>/cgroup`.
#[derive(Debug)]
pub struct ContainerId;
impl HintAttribute for ContainerId {
    type Value = String;
    fn name() -> &'static str { "container_id" }
    fn category() -> AttributeCategory { AttributeCategory::Identity }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "Container ID", Self::category(), AttributeKind::Text) }
}

/// Human-assigned container name, resolved from the ID via the runtime.
#[derive(Debug)]
pub struct ContainerName;
impl HintAttribute for ContainerName {
    type Value = String;
    fn name() -> &'static str { "container_name" }
    fn category() -> AttributeCategory { AttributeCategory::Identity }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "Container name", Self::category(), AttributeKind::Text).invalidated_by(&["container_id"]) }
}
