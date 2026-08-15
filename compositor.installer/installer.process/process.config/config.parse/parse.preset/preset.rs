use compositor_installer_process_config_parse_model::Env;

/// One installable session variant: its identity, file names, target binary, and
/// the exact environment it runs with.
#[derive(Clone, Debug)]
pub struct Preset {
    /// Stable id: "default" (the one installed session) | "custom".
    pub id: String,
    /// Human label for menus / logs.
    pub label: String,
    /// XDG desktop name (XDG_CURRENT_DESKTOP / DesktopNames=). Must equal `env.desktop_name`.
    pub desktop_name: String,
    /// Display-manager session entry Name=.
    pub session_name: String,
    /// /usr/bin wrapper script basename, e.g. "y5.compositor.dev.desktop".
    pub wrapper: String,
    /// systemd user unit basename, e.g. "y5.dev.service".
    pub service: String,
    /// /usr/share/wayland-sessions basename, e.g. "y5-compositor-dev.desktop".
    pub wayland_session: String,
    /// Installed binary the wrapper execs: "y5.compositor" or "y5.compositor.dev".
    pub binary: String,
    /// The full compositor environment for this preset.
    pub env: Env,
}

pub const SYSTEM_BINARY: &str = "y5.compositor";

pub const DEV_BINARY: &str = "y5.compositor.dev";

/// Wrap a fully-specified custom Env into the `Y5CompositorCustom` preset. The
/// `desktop_name` of the env is forced to the custom identity so the wrapper's
/// XDG_CURRENT_DESKTOP and the portal config line up.
pub fn custom_preset(root: &str, mut env: Env) -> Preset {
    let desktop_name = format!("{root}Custom");
    env.desktop_name = desktop_name.clone();
    Preset {
        id: "custom".into(),
        label: "Custom".into(),
        desktop_name,
        session_name: "Y5Custom".into(),
        wrapper: "y5.compositor.custom.desktop".into(),
        service: "y5.custom.service".into(),
        wayland_session: "y5-compositor-custom.desktop".into(),
        binary: DEV_BINARY.into(),
        env,
    }
}
