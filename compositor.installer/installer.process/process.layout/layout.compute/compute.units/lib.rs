//! Action-plan builders for the optional components (MX gesture daemon, polkit agent,
//! developer tool window, Xwayland) plus the two always-installed maintenance commands.

use std::path::PathBuf;

use compositor_installer_process_config_parse_base::Preset;
use compositor_installer_process_layout_compute_policy as policy;
use compositor_installer_process_layout_compute_uninstall as uninstall;
use compositor_installer_process_layout_compute_update as update;
use compositor_installer_process_layout_compute_stage::{
    Action, Source, Stage, home, place, user_systemd_dir,
};

/// Install + enable the MX gesture daemon (binary, udev rule, config, user service).
pub fn mx_actions(stage: &Stage) -> Vec<Action> {
    vec![
        place(home().join(".local/bin/mx-gesture-daemon"), Source::Copy(stage.binary("mx-gesture-daemon")), 0o755, false),
        place(PathBuf::from("/etc/udev/rules.d/42-logitech-hidpp.rules"), Source::Copy(stage.template("mx/42-logitech-hidpp.rules")), 0o644, true),
        place(home().join(".config/mx-gesture-daemon/config.toml"), Source::Copy(stage.template("mx/config.example.toml")), 0o644, false),
        place(user_systemd_dir().join("mx-gesture-daemon.service"), Source::Copy(stage.template("mx/mx-gesture-daemon.service")), 0o644, false),
        Action::UdevReload,
        Action::SystemctlUser(vec!["daemon-reload".into()]),
        Action::SystemctlUser(vec!["enable".into(), "mx-gesture-daemon.service".into()]),
    ]
}

/// Install + enable the polkit agent (binary + a new user systemd service).
pub fn polkit_actions(stage: &Stage) -> Vec<Action> {
    vec![
        place(PathBuf::from("/usr/local/bin/y5-polkit-agent"), Source::Copy(stage.binary("y5-polkit-agent")), 0o755, true),
        place(user_systemd_dir().join("y5-polkit-agent.service"), Source::Text(policy::polkit_service()), 0o644, false),
        Action::SystemctlUser(vec!["daemon-reload".into()]),
        Action::SystemctlUser(vec!["enable".into(), "y5-polkit-agent.service".into()]),
    ]
}

/// The two maintenance commands: `y5.compositor.update` (re-run the published bootstrap)
/// and `y5.compositor.uninstall` (remove what was placed). Placed unconditionally like
/// the binaries, so every install refreshes them.
///
/// Both are generated from the FIRST preset: its binary is the marker `update` gates on,
/// and its names are what `uninstall` removes.
pub fn maintenance_actions(preset: &Preset) -> Vec<Action> {
    vec![
        place(
            PathBuf::from("/usr/bin/y5.compositor.update"),
            Source::Text(update::update_script(preset)),
            0o755,
            true,
        ),
        place(
            PathBuf::from("/usr/bin/y5.compositor.uninstall"),
            Source::Text(uninstall::uninstall_script(preset)),
            0o755,
            true,
        ),
    ]
}

/// Install the developer tool window as `/usr/bin/y5.compositor.monitor` (the staged
/// Tauri binary is `compositor-developer-tool`), plus an app-launcher desktop entry so
/// it shows up in the menu.
pub fn devtool_actions(stage: &Stage) -> Vec<Action> {
    vec![
        place(PathBuf::from("/usr/bin/y5.compositor.monitor"), Source::Copy(stage.binary("compositor-developer-tool")), 0o755, true),
        place(PathBuf::from("/usr/share/applications/y5.compositor.monitor.desktop"), Source::Text(policy::devtool_desktop_entry()), 0o644, true),
    ]
}

/// Install + enable the patched xwayland-satellite (X11-app compatibility): the binary
/// at the path its service expects, plus the user systemd service. Pairs with the
/// default-on `xwayland` package group (the `xorg-x11-server-Xwayland` it drives).
pub fn xwayland_actions(stage: &Stage) -> Vec<Action> {
    vec![
        place(PathBuf::from("/usr/bin/xwayland-satellite"), Source::Copy(stage.binary("xwayland-satellite")), 0o755, true),
        place(user_systemd_dir().join("xwayland.service"), Source::Copy(stage.template("xwayland/xwayland.service")), 0o644, false),
        Action::SystemctlUser(vec!["daemon-reload".into()]),
        Action::SystemctlUser(vec!["enable".into(), "xwayland.service".into()]),
    ]
}
