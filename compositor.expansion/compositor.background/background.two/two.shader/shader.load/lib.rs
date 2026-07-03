//! Runtime shader loader: resolve a selection, try the active renderer's format
//! fallback order, compile the first that succeeds, and surface its declared
//! properties. Returns `None` (→ caller uses the built-in parallax) when no
//! format is present or none compiles. Every failure is logged, never fatal.

#[macro_use]
extern crate compositor_developer_debug_instance_record;

use compositor_background_two_shader_locate::{Format, order, resolve_ref, source_path};
use compositor_background_two_shader_property::{Property, parse_props};
use compositor_background_two_shader_spirv::{VulkanModule, build_glsl, build_wgsl};
use smithay::backend::renderer::gles::{GlesPixelProgram, GlesRenderer};
use std::path::Path;

/// A compiled background shader for the active renderer plus its property schema.
/// Exactly one of `gles`/`vulkan` is set (whichever the active renderer needs).
pub struct LoadedShader {
    pub properties: Vec<Property>,
    pub gles: Option<GlesPixelProgram>,
    pub vulkan: Option<VulkanModule>,
}

/// Load the bundle named (or absolute-pathed) by `value` for the active renderer.
/// Returns the compiled shader (or `None` → built-in) plus the compile error
/// when a source for this renderer existed but failed (for the settings status).
pub fn load(
    renderer: &mut GlesRenderer,
    prefers_dmabuf: bool,
    value: &str,
) -> (Option<LoadedShader>, Option<String>) {
    let bundle = resolve_ref(value);
    let mut last_error = None;
    for &fmt in order(prefers_dmabuf) {
        let Some(path) = source_path(&bundle, fmt) else { continue };
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                error!("background.shader: read {path:?}: {e}");
                last_error = Some(format!("read {path:?}: {e}"));
                continue;
            }
        };
        let properties = parse_props(&src);
        let id = hash_path(&path);
        let result = match fmt {
            Format::GlesFrag => compile_gles(renderer, &src).map(|p| LoadedShader {
                properties: properties.clone(),
                gles: Some(p),
                vulkan: None,
            }),
            Format::VulkanWgsl | Format::Wgsl => build_wgsl(&src, id).map(|m| LoadedShader {
                properties: properties.clone(),
                gles: None,
                vulkan: Some(m),
            }),
            Format::Glsl => build_glsl(&src, id).map(|m| LoadedShader {
                properties: properties.clone(),
                gles: None,
                vulkan: Some(m),
            }),
        };
        match result {
            Ok(loaded) => {
                info!("background.shader: loaded {path:?} ({} props)", loaded.properties.len());
                return (Some(loaded), None);
            }
            Err(e) => {
                error!("background.shader: compile {path:?}: {e}");
                last_error = Some(e);
            }
        }
    }
    (None, last_error)
}

fn compile_gles(r: &mut GlesRenderer, src: &str) -> Result<GlesPixelProgram, String> {
    compositor_background_two_draw_program::compile_source(r, src).map_err(|e| format!("{e:?}"))
}

/// The WGSL source for a bundle selection, if it has a `vulkan/` or `wgsl/`
/// format AND that source compiles (the settings preview feeds it straight to
/// wgpu, where an invalid module is a fatal error, not a fallback). `None` for
/// GLSL/GLES-only or broken bundles, so the preview uses the built-in shader.
pub fn preview_wgsl(value: &str) -> Option<String> {
    let bundle = resolve_ref(value);
    // WGSL bundles: use directly (validated so the wgpu preview never crashes).
    for fmt in [Format::VulkanWgsl, Format::Wgsl] {
        if let Some(path) = source_path(&bundle, fmt) {
            if let Ok(src) = std::fs::read_to_string(&path) {
                if build_wgsl(&src, 0).is_ok() {
                    return Some(src);
                }
            }
        }
    }
    // GLSL bundles: cross-compile to WGSL (+ a fullscreen vertex) for the preview.
    if let Some(path) = source_path(&bundle, Format::Glsl) {
        if let Ok(src) = std::fs::read_to_string(&path) {
            if let Ok(wgsl) = compositor_background_two_shader_spirv::glsl_to_preview_wgsl(&src) {
                return Some(wgsl);
            }
        }
    }
    None
}

/// Parse the `@prop` properties for a bundle selection (any format file carries
/// the same `// @prop` annotations). Empty if absent. For the settings controls.
pub fn properties_for(value: &str) -> Vec<Property> {
    let bundle = resolve_ref(value);
    for fmt in [Format::Wgsl, Format::Glsl, Format::VulkanWgsl, Format::GlesFrag] {
        if let Some(path) = source_path(&bundle, fmt) {
            if let Ok(src) = std::fs::read_to_string(&path) {
                return parse_props(&src);
            }
        }
    }
    Vec::new()
}

/// A stable-per-path pipeline-cache id, kept clear of the built-in shader ids.
fn hash_path(p: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.hash(&mut h);
    h.finish() | 0xF000_0000_0000_0000
}
