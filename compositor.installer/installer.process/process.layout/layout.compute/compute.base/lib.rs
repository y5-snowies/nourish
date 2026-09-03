//! File layout: generate the per-preset wrapper scripts, systemd units,
//! wayland-session entries and xdg-portal configs, plus the PAM lock, MX daemon
//! and polkit-agent artifacts; model them as an ordered action plan; and apply it
//! (or print it, for `--dry-run`).
//!
//! Façade: the implementation lives in the sibling `compute.*` crates; this crate
//! re-exports the original public surface unchanged. Pure std.

/// Text generators for every installed config file, modeled on the live
/// reference captures kept under .reference/references/ (live-installation-example-files/
/// and installer-artifact/).
pub mod templates {
    pub use compositor_installer_process_layout_compute_policy::*;
    pub use compositor_installer_process_layout_compute_session::*;
    pub use compositor_installer_process_layout_compute_uninstall::*;
    pub use compositor_installer_process_layout_compute_update::*;
}

pub use compositor_installer_process_layout_compute_apply::apply;
pub use compositor_installer_process_layout_compute_plan::{
    binary_actions, pam_actions, preset_actions, settings_action, settings_json,
};
pub use compositor_installer_process_layout_compute_stage::{
    Action, Source, Stage, home, is_root, settings_exists, settings_path,
};
pub use compositor_installer_process_layout_compute_units::{
    devtool_actions, maintenance_actions, mx_actions, polkit_actions,
};
pub use compositor_installer_process_layout_compute_retire::retire::xwayland_retire_actions;
