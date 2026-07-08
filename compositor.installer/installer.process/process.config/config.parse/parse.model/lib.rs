//! Config model: the compositor environment (`Env`) and the prompted base
//! configuration (`BaseConfig`). Pure std.

/// Carrier for the installer's prompted values + the GPU-derived capture encoder. NOT
/// the full settings schema and NOT serialized directly — the complete settings.json is
/// built by layering the fields the installer sets onto `config.base::default_settings()`
/// (see `compute.plan::settings_json`), so the seed always has every required field.
#[derive(Clone, Debug)]
pub struct Env {
    pub renderer: String,
    pub renderer_fallback: bool,
    pub renderer_sync: String,
    pub hdr: bool,
    pub depth: u8,
    pub vrr: bool,
    pub render_node: String,
    pub desktop_name: String,
    pub log_level: String,
    pub vk_diag: String,
    pub capture_encoder: String,
    pub window_client_size_fallback: bool,
    pub window_subsurface_shrinks: bool,
}

/// Values prompted once for the default Y5 Desktop. Most propagate unchanged into
/// every other preset (the user enters the render node, log level, etc. once).
#[derive(Clone, Debug)]
pub struct BaseConfig {
    pub render_node: String,
    /// Root XDG desktop name; variants append a suffix (Dev, DevGles, …).
    pub desktop_name_root: String,
    pub log_level: String,
    /// Default scanout depth for the Y5 Desktop (10 per spec — same as Dev).
    pub depth: u8,
    pub vrr: bool,
    /// Renderer backend: "vulkan" (default) or "gles". AMD cards may need gles.
    pub renderer: String,
    pub renderer_fallback: bool,
}

impl Default for BaseConfig {
    fn default() -> Self {
        BaseConfig {
            render_node: "/dev/dri/renderD128".to_string(),
            desktop_name_root: "Y5Compositor".to_string(),
            log_level: "info,warn,error".to_string(),
            depth: 10,
            vrr: true,
            renderer: "vulkan".to_string(),
            renderer_fallback: false,
        }
    }
}
