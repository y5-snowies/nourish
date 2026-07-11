#[derive(Clone, Debug)]
pub struct Environment {
    pub GPU: String,
    pub DesktopName: String,
}

/// Thin adapter over the central environment config
/// (`compositor_developer_environment_config_base`). Kept so existing consumers
/// that hold an `Environment { GPU, DesktopName }` (e.g. `State::new`) need no
/// signature change. The values come from the single `COMPOSITOR_ENVIRONMENT`
/// JSON, which `init()` has already parsed by the time this is called.
pub fn Get() -> Environment {
    let env = compositor_developer_environment_config_base::base::get();
    let DesktopName = String::from(env.desktop_name.trim());
    let GPU = compositor_developer_environment_config_router::router::primary_render_string();

    return Environment { GPU, DesktopName };
}
