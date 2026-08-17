//! Push [`ContainerId`] / [`ContainerName`] hints for a containerised process.

use compositor_introspection_extraction_window_hints_attributes_container::attributes::{
    ContainerId, ContainerName,
};
use compositor_introspection_extraction_window_hints_container_detect::detect;
use compositor_introspection_extraction_window_hints_container_query::query;
use compositor_introspection_extraction_window_hints_inferred::inferred::InferredHints;
use compositor_introspection_extraction_window_hints_source::source::{Confidence, SourceMethod};
use compositor_introspection_extraction_window_meta_types::types::Meta;

/// Detect the container this process runs in and record what we know about it.
///
/// No-op — and no cost beyond one cgroup scan — for the overwhelmingly common
/// case of a process that isn't containerised.
///
/// The name may legitimately be absent on the first extraction: resolving it
/// needs the container runtime, and [`query::name_for`] refuses to block the
/// caller for that. The next sampler pass fills it in.
pub fn push_container_hints(meta: &Meta, hints: &mut InferredHints) {
    let container_var = meta
        .selected_env
        .as_ref()
        .and_then(|env| env.get("container"))
        .map(String::as_str);

    let Some(found) = detect::detect(meta.cgroup.as_deref(), container_var) else {
        return;
    };

    let Some(id) = found.id else {
        // The env marker said "container" but the cgroup carried no id — we
        // know it is contained, but not WHICH, so there is nothing to launch
        // into and nothing worth recording.
        return;
    };

    hints.push::<ContainerId>(
        id.clone(),
        SourceMethod::ProcCgroup,
        "container id in /proc/<pid>/cgroup",
        Confidence::High,
    );

    if found.runtime != detect::Runtime::Podman {
        return; // only podman name resolution is implemented
    }
    if let Some(name) = query::name_for(&id) {
        hints.push::<ContainerName>(
            name,
            SourceMethod::ProcCgroup,
            "resolved from the container id via podman",
            Confidence::High,
        );
    }
}
