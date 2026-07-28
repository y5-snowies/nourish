//! Interactive construction of a fully custom `Env`.

use compositor_installer_process_config_parse_model::{BaseConfig, Env};

/// Interactively build a Custom env, printing every field's available options.
/// `base` supplies the defaults shown (Enter keeps them).
pub fn prompt_custom_env(base: &BaseConfig) -> Env {
    use compositor_installer_process_config_parse_prompt::*;
    println!("\n== Custom preset — set each value (Enter keeps the [default]) ==");
    let renderer = choose(
        "renderer",
        "Renderer backend",
        &["vulkan", "gles"],
        "vulkan",
    );
    let renderer_fallback = yes_no(
        "renderer_fallback",
        "Fall back to GLES if Vulkan init fails",
        base.renderer_fallback,
    );
    let hdr = yes_no("hdr", "Enable HDR output (Vulkan only)", false);
    let depth = if yes_no("depth", "10-bit deep-color scanout (no = 8-bit)", base.depth == 10) {
        10
    } else {
        8
    };
    let vrr = yes_no("vrr", "Enable adaptive sync / VRR", base.vrr);
    let render_node = ask("render_node", "DRM render node path", &base.render_node);
    let log_level = ask("log_level", "Developer log levels (comma-separated)", &base.log_level);
    let vk_diag = choose(
        "vk_diag",
        "Vulkan diagnostics overlay",
        &["", "vk", "blit"],
        "",
    );
    Env {
        renderer,
        renderer_fallback,
        // The KMS IN_FENCE path (`config.base::RENDERER_SYNC_DEFAULT`); not
        // prompted, and normalized again by `compute.plan::settings_json`.
        renderer_sync: "infence".to_string(),
        hdr,
        depth,
        vrr,
        render_node,
        // desktop_name set by custom_preset().
        desktop_name: String::new(),
        log_level,
        vk_diag,
        capture_encoder: "nvenc".to_string(),
        window_client_size_fallback: false,
        window_subsurface_shrinks: false,
    }
}
