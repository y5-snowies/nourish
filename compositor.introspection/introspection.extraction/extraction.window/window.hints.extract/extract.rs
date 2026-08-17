use compositor_introspection_extraction_window_hints_attributes_identity::attributes::{DisplayName, Sandbox};
use compositor_introspection_extraction_window_hints_attributes_launch::attributes::{
    ExecArgs, ExecProgram, WorkingDirectory,
};
use compositor_introspection_extraction_window_hints_container_extract::extract::push_container_hints;
use compositor_introspection_extraction_window_hints_extract_entry::extract::{
    push_desktop_hints, push_env_hints, push_surface_identity_hints, push_toplevel_icon_hints,
};
use compositor_introspection_extraction_window_hints_inferred::inferred::InferredHints;
use compositor_introspection_extraction_window_hints_source::source::{Confidence, SourceMethod};
use compositor_introspection_extraction_window_hints_values::values::ToplevelIcon;
use compositor_introspection_extraction_window_hints_values::sandbox::parse_sandbox;
use compositor_introspection_extraction_window_meta_types::types::MetaNode;
use std::path::PathBuf;

/// Populate base hints from the window's process tree: identity, sandbox,
/// exec primitives, desktop entry, icon, env overlay. Handler-specific hints
/// are added separately by the registry after detection picks a handler.
///
/// `icon` is the live toplevel's `xdg_toplevel_icon_v1` state, and it is the one
/// input here that is NOT derivable from `node`: a surface can only be read on
/// the compositor thread, so callers that have a window pass it and the
/// background sampler passes `None`.
pub fn extract_base_hints(node: &MetaNode, icon: Option<&ToplevelIcon>) -> InferredHints {
    let mut hints = InferredHints::new();
    let meta = &node.meta;

    // ---- Executable -----------------------------------------------------
    if let Some(exe) = &meta.exe {
        hints.push::<ExecProgram>(
            exe.clone(),
            SourceMethod::ProcExe,
            "/proc/<pid>/exe symlink",
            Confidence::High,
        );
    }
    if let Some(cmdline) = &meta.cmdline {
        if let Some(first) = cmdline.first() {
            hints.push::<ExecProgram>(
                PathBuf::from(first),
                SourceMethod::ProcCmdline,
                "argv[0]",
                Confidence::Low,
            );
        }
        if cmdline.len() > 1 {
            hints.push::<ExecArgs>(
                cmdline[1..].to_vec(),
                SourceMethod::ProcCmdline,
                "argv[1..]",
                Confidence::Medium,
            );
        }
    }

    // ---- Working directory ---------------------------------------------
    if let Some(cwd) = &meta.cwd {
        hints.push::<WorkingDirectory>(
            cwd.clone(),
            SourceMethod::ProcExe,
            "/proc/<pid>/cwd at pin time",
            Confidence::Low,
        );
    }

    // ---- Sandbox identity ----------------------------------------------
    if let Some(cg) = &meta.cgroup {
        hints.push::<Sandbox>(
            parse_sandbox(cg),
            SourceMethod::ProcCgroup,
            "parsed from /proc/<pid>/cgroup",
            Confidence::High,
        );
    }

    // ---- Surface identity + environment + desktop entry resolution ------
    push_surface_identity_hints(meta, &mut hints);
    push_env_hints(meta, &mut hints);
    push_desktop_hints(meta, &mut hints);
    // AFTER the desktop entry, on purpose: the toplevel's own icon name only
    // becomes the IconPath when the entry resolved none.
    if let Some(icon) = icon {
        push_toplevel_icon_hints(icon, &mut hints);
    }
    // No-op for a host process; lets a relaunch target the container rather
    // than an exe path that only exists inside it.
    push_container_hints(meta, &mut hints);

    // ---- Fallback display name -----------------------------------------
    if !hints.has::<DisplayName>() {
        if let Some(comm) = &meta.comm {
            hints.push::<DisplayName>(
                comm.clone(),
                SourceMethod::ProcExe,
                "/proc/<pid>/comm",
                Confidence::Low,
            );
        } else if let Some(app_id) = &meta.app_id {
            hints.push::<DisplayName>(
                app_id.clone(),
                SourceMethod::WaylandSurface,
                "Wayland app_id",
                Confidence::Low,
            );
        }
    }

    hints
}
