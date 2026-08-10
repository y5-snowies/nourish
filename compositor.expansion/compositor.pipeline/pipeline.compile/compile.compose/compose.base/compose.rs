//! Combine a pass's WGSL with the shared `#import` modules it uses (and any
//! shader-defs) into one naga module, then compile to a Vulkan module.
//!
//! Each shared module declares its own import path with `#define_import_path
//! <name>` at its top; a pass then `#import <name>::<item>`s from it. Passes are
//! typically fragment-only — `module_to_vulkan` pairs them with the fullscreen
//! vertex, exactly like a `glsl/` bundle.

use compositor_pipeline_compile_spirv_base::{module_to_vulkan, VulkanModule};
use naga_oil::compose::{
    ComposableModuleDescriptor, Composer, NagaModuleDescriptor, ShaderDefValue, ShaderLanguage,
};
use std::collections::{BTreeMap, HashMap};

/// A shared importable module: its WGSL source and the path naga_oil names in
/// diagnostics (the bundle-relative file path).
pub struct Module<'a> {
    pub source: &'a str,
    pub file_path: &'a str,
}

/// Compose `source` (bundle-relative `file_path`) with the `modules` it may
/// `#import`, applying `defines` as shader-defs, and compile to a `VulkanModule`
/// tagged with cache id `id`. Any naga_oil / naga error is returned as a string
/// so the loader can fall back.
pub fn compose_wgsl(
    source: &str,
    file_path: &str,
    modules: &[Module<'_>],
    defines: &BTreeMap<String, String>,
    id: u64,
) -> Result<VulkanModule, String> {
    // Non-validating: naga_oil's internal validator lacks the PUSH_CONSTANT
    // capability our `var<immediate>` engine block needs — the real validation
    // (with `Capabilities::all()`) happens in `module_to_vulkan`'s SPIR-V step.
    let mut composer = Composer::non_validating();
    for m in modules {
        composer
            .add_composable_module(ComposableModuleDescriptor {
                source: m.source,
                file_path: m.file_path,
                language: ShaderLanguage::Wgsl,
                ..Default::default()
            })
            .map_err(|e| format!("compose module {}: {e}", m.file_path))?;
    }
    let module = composer
        .make_naga_module(NagaModuleDescriptor {
            source,
            file_path,
            shader_defs: shader_defs(defines),
            ..Default::default()
        })
        .map_err(|e| format!("compose {file_path}: {e}"))?;
    module_to_vulkan(module, id)
}

/// Map manifest string defines to typed naga_oil shader-defs: `true`/`false` →
/// Bool, else an integer → Int. Non-integer, non-bool values are skipped
/// (naga_oil shader-defs are bool/int/uint only).
pub fn shader_defs(defines: &BTreeMap<String, String>) -> HashMap<String, ShaderDefValue> {
    let mut out = HashMap::new();
    for (k, v) in defines {
        let val = match v.as_str() {
            "true" => ShaderDefValue::Bool(true),
            "false" => ShaderDefValue::Bool(false),
            s => match s.parse::<i32>() {
                Ok(i) => ShaderDefValue::Int(i),
                Err(_) => continue,
            },
        };
        out.insert(k.clone(), val);
    }
    out
}
