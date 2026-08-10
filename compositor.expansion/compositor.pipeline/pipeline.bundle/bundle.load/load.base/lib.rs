//! Runtime shader loader: resolve a selection, try the active renderer's format
//! fallback order, compile the first that succeeds, and surface its declared
//! properties. Returns `None` (→ caller uses the built-in parallax) when no
//! format is present or none compiles. Every failure is logged, never fatal.

#[macro_use]
extern crate compositor_model_debug_instance_record;

use compositor_pipeline_bundle_embed_base::embed::read_file;
use compositor_pipeline_bundle_locate_base::{Format, order, resolve_ref, source_path};
use compositor_pipeline_bundle_manifest_base::manifest::{PIPELINE_FILE, parse as parse_manifest};
use compositor_pipeline_bundle_property_base::{
    Property, apply_optimized, has_optimized, merge_props, parse_props,
};
use compositor_pipeline_compile_spirv_base::{VulkanModule, build_glsl, build_wgsl};
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
///
/// `optimized` asks for the shader's cheap variant. It only bites when the source
/// actually declares `@optimized` knobs; for anything else the reference source
/// compiles and the flag is dropped, so the id keeps matching the reference and no
/// duplicate pipeline is built for identical SPIR-V.
pub fn load(
    renderer: &mut GlesRenderer,
    prefers_dmabuf: bool,
    value: &str,
    optimized: bool,
) -> (Option<LoadedShader>, Option<String>) {
    // A `builtin:` world compiles from an embedded WGSL source — no disk access.
    // It runs only on the Vulkan (dmabuf) path; on GLES there is no built-in
    // source, so fall through to the stock parallax.
    if let Some(src) = compositor_pipeline_bundle_builtin_base::source(value) {
        if !prefers_dmabuf {
            return (None, None);
        }
        let properties = parse_props(src);
        let opt = optimized && has_optimized(src);
        let src = if opt { apply_optimized(src) } else { src.to_string() };
        return match build_wgsl(&src, variant(builtin_id(value), opt)) {
            Ok(m) => (Some(LoadedShader { properties, gles: None, vulkan: Some(m) }), None),
            Err(e) => {
                error!("background.shader: builtin {value}: {e}");
                (None, Some(e))
            }
        };
    }
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
        // Same deal as the built-in path: a bundle that declares `@optimized`
        // knobs gets its cheap variant here, which is how a user shader lights up
        // the same toggle without any per-bundle plumbing.
        let opt = optimized && has_optimized(&src);
        let src = if opt { apply_optimized(&src) } else { src };
        let id = variant(hash_path(&path), opt);
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
    compositor_pipeline_compile_program_base::compile_source(r, src).map_err(|e| format!("{e:?}"))
}

/// The WGSL source for a bundle selection, if it has a `vulkan/` or `wgsl/`
/// format AND that source compiles (the settings preview feeds it straight to
/// wgpu, where an invalid module is a fatal error, not a fallback). `None` for
/// GLSL/GLES-only or broken bundles, so the preview uses the built-in shader.
/// `optimized` applies the same `@optimized` rewrite the runtime path uses, so the
/// panel previews the variant actually on screen rather than the reference.
pub fn preview_wgsl(value: &str, optimized: bool) -> Option<String> {
    let opt = |s: &str| if optimized { apply_optimized(s) } else { s.to_string() };
    if let Some(src) = compositor_pipeline_bundle_builtin_base::source(value) {
        return Some(opt(src));
    }
    let bundle = resolve_ref(value);
    // WGSL bundles: use directly (validated so the wgpu preview never crashes).
    for fmt in [Format::VulkanWgsl, Format::Wgsl] {
        if let Some(path) = source_path(&bundle, fmt) {
            if let Ok(src) = std::fs::read_to_string(&path) {
                if build_wgsl(&src, 0).is_ok() {
                    return Some(opt(&src));
                }
            }
        }
    }
    // GLSL bundles: cross-compile to WGSL (+ a fullscreen vertex) for the preview.
    if let Some(path) = source_path(&bundle, Format::Glsl) {
        if let Ok(src) = std::fs::read_to_string(&path) {
            if let Ok(wgsl) = compositor_pipeline_compile_spirv_base::glsl_to_preview_wgsl(&src) {
                return Some(wgsl);
            }
        }
    }
    None
}

/// Whether this selection ships an optimized variant — i.e. whether its source
/// declares any `@optimized` knob. Drives whether the settings toggle is live or
/// greyed out; a toggle that silently did nothing would be a lie.
pub fn supports_optimized(value: &str) -> bool {
    if let Some(src) = compositor_pipeline_bundle_builtin_base::source(value) {
        return has_optimized(src);
    }
    let bundle = resolve_ref(value);
    for fmt in [Format::VulkanWgsl, Format::Wgsl, Format::Glsl, Format::GlesFrag] {
        if let Some(path) = source_path(&bundle, fmt) {
            if let Ok(src) = std::fs::read_to_string(&path) {
                return has_optimized(&src);
            }
        }
    }
    false
}

/// Parse the `@prop` properties for a bundle selection (any format file carries
/// the same `// @prop` annotations). Empty if absent. For the settings controls.
///
/// A multipass bundle declares its props across `passes/`, which the single-pass
/// scan below never looks at — hence "This shader exposes no variables".
pub fn properties_for(value: &str) -> Vec<Property> {
    if let Some(p) = compositor_pipeline_bundle_builtin_base::props(value) {
        return p;
    }
    let bundle = resolve_ref(value);
    if let Some(props) = multipass_properties(&bundle) {
        return props;
    }
    for fmt in [Format::Wgsl, Format::Glsl, Format::VulkanWgsl, Format::GlesFrag] {
        if let Some(path) = source_path(&bundle, fmt) {
            if let Ok(src) = std::fs::read_to_string(&path) {
                return parse_props(&src);
            }
        }
    }
    Vec::new()
}

/// Which picker heading a selection sits under.
///
/// A built-in names its own; a multipass bundle may name one in its manifest;
/// everything else is "User", so a dropped-in folder is findable without its
/// author declaring anything. Reads the manifest — memoize on the bundle LIST.
///
/// A heading from the shader FOLDER carries `USER_MARK`, its own declared one
/// included: a dropped-in bundle may name `Multipass` too, and the picker groups
/// by this string, so without the mark it would join the curated set rather than
/// sit beside it. Shipped bundles — compiled-in or built-in — are never marked.
pub fn category_for(value: &str) -> String {
    use compositor_pipeline_bundle_builtin_base as builtin;
    if let Some(b) = builtin::builtins().iter().find(|b| b.id == value) {
        return b.category.to_string();
    }
    let declared = read_file(&resolve_ref(value), PIPELINE_FILE)
        .ok()
        .and_then(|src| parse_manifest(&src).ok())
        .and_then(|m| m.category)
        .unwrap_or_else(|| builtin::USER_CATEGORY.to_string());
    match compositor_pipeline_bundle_embed_base::embed::bundle_of(value).is_some() {
        true => declared,
        false => format!("{}{declared}", builtin::USER_MARK),
    }
}

/// The union of every pass's `@prop`s for a `pipeline.json` bundle, in manifest
/// pass order, first definition wins — the SAME union `shader.pipeline` computes
/// while compiling, via the same `merge_props`.
///
/// Same ORDER, not merely the same set: the union index is the param slot the
/// panel edits. `None` — not empty — when this is not a multipass bundle, so the
/// caller can fall through to the single-pass scan.
fn multipass_properties(bundle: &Path) -> Option<Vec<Property>> {
    let manifest = parse_manifest(&read_file(bundle, PIPELINE_FILE).ok()?).ok()?;
    let mut props = Vec::new();
    for pass in &manifest.passes {
        if let Ok(src) = read_file(bundle, &pass.shader) {
            merge_props(&mut props, parse_props(&src));
        }
    }
    Some(props)
}

/// A stable-per-path pipeline-cache id, kept clear of the built-in shader ids.
fn hash_path(p: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.hash(&mut h);
    h.finish() | 0xF000_0000_0000_0000
}

/// The optimized twin's pipeline-cache id.
///
/// It MUST differ from the reference's. The renderer's cache is keyed on
/// `(id, format)` alone and a hit ignores the SPIR-V bytes entirely, so sharing
/// an id would keep drawing the reference pipeline until the compositor restarts.
/// XOR of a bit inside the payload keeps the top nibble — and therefore the
/// built-in / bundle id-space split — intact.
fn variant(id: u64, optimized: bool) -> u64 {
    if optimized { id ^ 0x0800_0000_0000_0000 } else { id }
}

/// A stable pipeline-cache id for a built-in world, in a range distinct from the
/// user-bundle ids (`0xF…`) and the stock parallax's small fixed ids.
fn builtin_id(id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    (h.finish() & 0x0FFF_FFFF_FFFF_FFFF) | 0xE000_0000_0000_0000
}
