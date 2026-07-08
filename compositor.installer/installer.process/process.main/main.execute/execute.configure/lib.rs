//! Installer steps 2-3: prompt the Y5 Compositor configuration and build the single
//! session preset from it.

use compositor_installer_process_config_parse_base as cfg;
use compositor_installer_process_config_parse_base::prompt;
use compositor_installer_process_packages_enumerate_base as pkg;

/// Prompt the compositor configuration and build the single "Y5 Compositor" preset. The
/// capture encoder defaults to the detected GPU's (NVENC on NVIDIA, VAAPI otherwise).
pub fn gather() -> Vec<cfg::Preset> {
    println!("\n-- Y5 Compositor configuration --");
    let defaults = cfg::BaseConfig::default();
    let base = cfg::BaseConfig {
        render_node: prompt::ask("render_node", "DRM render node path", &defaults.render_node),
        desktop_name_root: prompt::ask(
            "desktop_name",
            "XDG desktop name",
            &defaults.desktop_name_root,
        ),
        log_level: prompt::ask("log_level", "Developer log levels", &defaults.log_level),
        depth: if prompt::yes_no("depth", "10-bit deep-color scanout (no = 8-bit)", defaults.depth == 10) {
            10
        } else {
            8
        },
        vrr: prompt::yes_no("vrr", "Enable adaptive sync / VRR", defaults.vrr),
        renderer: prompt::choose(
            "renderer",
            "Renderer backend. NOTE for AMD users: some have reported Vulkan not \
             working on AMD; if you're on an AMD card, choose 'gles'.",
            &["vulkan", "gles"],
            &defaults.renderer,
        ),
        renderer_fallback: prompt::yes_no(
            "renderer_fallback",
            "Fall back to GLES if Vulkan init fails",
            defaults.renderer_fallback,
        ),
    };

    let encoder = pkg::capture_encoder_for(pkg::detect_gpu());
    cfg::default_presets(&base, encoder)
}
