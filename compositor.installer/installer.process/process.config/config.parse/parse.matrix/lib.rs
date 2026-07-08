//! The single installed session preset (Y5 Compositor).

use compositor_installer_process_config_parse_model::{BaseConfig, Env};
use compositor_installer_process_config_parse_preset::{Preset, SYSTEM_BINARY};

/// Build the one and only session preset — "Y5 Compositor" — from the prompted base
/// config. A single system session with no experimental / sync variants and no Custom
/// preset; the renderer backend (vulkan or gles) is the prompted `base.renderer`.
/// `capture_encoder` is chosen for the detected GPU by the caller (NVENC on NVIDIA,
/// VAAPI otherwise).
pub fn default_presets(base: &BaseConfig, capture_encoder: &str) -> Vec<Preset> {
    let env = Env {
        renderer: base.renderer.clone(),
        renderer_fallback: base.renderer_fallback,
        renderer_sync: String::new(),
        hdr: false,
        depth: base.depth,
        vrr: base.vrr,
        render_node: base.render_node.clone(),
        desktop_name: base.desktop_name_root.clone(),
        log_level: base.log_level.clone(),
        vk_diag: String::new(),
        capture_encoder: capture_encoder.to_string(),
        window_client_size_fallback: false,
        window_subsurface_shrinks: false,
    };
    vec![Preset {
        id: "default".into(),
        label: "Y5 Compositor".into(),
        desktop_name: base.desktop_name_root.clone(),
        session_name: "Y5 Compositor".into(),
        wrapper: "y5.compositor.desktop".into(),
        service: "y5.service".into(),
        wayland_session: "y5-compositor.desktop".into(),
        binary: SYSTEM_BINARY.into(),
        env,
    }]
}
