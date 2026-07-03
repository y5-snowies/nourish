//! Runtime WGSL/GLSL → SPIR-V compilation via naga (the Vulkan path).
//!
//! Mirrors `draw.vulkan/build.rs` (validate, then `write_vec(.., None)` so every
//! entry point lands in one blob, with the WGSL→API Y-flip disabled — our
//! geometry is authored in Vulkan clip space). WGSL yields one module holding
//! both `vs_main`+`fs_main`; a `glsl/` fragment yields a fragment-only module
//! that we pair with a prebuilt fullscreen-vertex module.

use naga::back::spv;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use naga::{Module, ShaderStage};

/// A compiled Vulkan shader ready for the renderer's fullscreen pass. When
/// `vert_spv` is `Some`, the vertex stage lives in that separate module;
/// otherwise `spv` holds both stages.
pub struct VulkanModule {
    pub id: u64,
    pub spv: Vec<u8>,
    pub vert_spv: Option<Vec<u8>>,
    pub vert_entry: String,
    pub frag_entry: String,
}

/// A fullscreen-triangle vertex stage (clip-space), used as the separate vertex
/// module for `glsl/` fragments. Same `vs_main` as the built-in `parallax.wgsl`.
const FULLSCREEN_VS: &str = "\
struct VsOut { @builtin(position) pos: vec4<f32> };
@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    let uv = vec2<f32>(f32((vid << 1u) & 2u), f32(vid & 2u));
    var o: VsOut;
    o.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return o;
}";

/// Compile a self-contained WGSL bundle (vertex + fragment in one module).
pub fn build_wgsl(src: &str, id: u64) -> Result<VulkanModule, String> {
    let module = naga::front::wgsl::parse_str(src).map_err(|e| format!("wgsl parse: {e:?}"))?;
    let vert_entry = entry(&module, ShaderStage::Vertex).ok_or("wgsl: no vertex entry")?;
    let frag_entry = entry(&module, ShaderStage::Fragment).ok_or("wgsl: no fragment entry")?;
    Ok(VulkanModule { id, spv: to_spirv(&module)?, vert_spv: None, vert_entry, frag_entry })
}

/// Compile a desktop-`#version 450 core` GLSL fragment + the fullscreen vertex.
pub fn build_glsl(src: &str, id: u64) -> Result<VulkanModule, String> {
    let mut front = naga::front::glsl::Frontend::default();
    let opts = naga::front::glsl::Options::from(ShaderStage::Fragment);
    let frag = front.parse(&opts, src).map_err(|e| format!("glsl parse: {e:?}"))?;
    let frag_entry = entry(&frag, ShaderStage::Fragment).ok_or("glsl: no fragment entry")?;
    let vs = naga::front::wgsl::parse_str(FULLSCREEN_VS).map_err(|e| format!("vs parse: {e:?}"))?;
    let vert_entry = entry(&vs, ShaderStage::Vertex).ok_or("vs: no vertex entry")?;
    Ok(VulkanModule {
        id,
        spv: to_spirv(&frag)?,
        vert_spv: Some(to_spirv(&vs)?),
        vert_entry,
        frag_entry,
    })
}

/// Convert a desktop-GLSL fragment bundle to self-contained preview WGSL:
/// glsl-in → wgsl-out, rename the fragment entry (`main`) to `fs_main`, and
/// prepend the fullscreen `vs_main`. For the settings wgpu preview of GLSL
/// bundles (the push block emits as `var<immediate>`, which the preview rewrites
/// to a uniform). Returns `Err` if naga can't parse/emit the shader.
pub fn glsl_to_preview_wgsl(src: &str) -> Result<String, String> {
    let mut front = naga::front::glsl::Frontend::default();
    let opts = naga::front::glsl::Options::from(ShaderStage::Fragment);
    let module = front.parse(&opts, src).map_err(|e| format!("glsl parse: {e:?}"))?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .map_err(|e| format!("validate: {e:?}"))?;
    let wgsl = naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
        .map_err(|e| format!("wgsl: {e:?}"))?;
    Ok(format!("{FULLSCREEN_VS}\n{}", wgsl.replace("fn main(", "fn fs_main(")))
}

fn entry(m: &Module, stage: ShaderStage) -> Option<String> {
    m.entry_points.iter().find(|e| e.stage == stage).map(|e| e.name.clone())
}

fn to_spirv(module: &Module) -> Result<Vec<u8>, String> {
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(module)
        .map_err(|e| format!("validate: {e:?}"))?;
    let mut opts = spv::Options::default();
    opts.flags.remove(spv::WriterFlags::ADJUST_COORDINATE_SPACE);
    let words = spv::write_vec(module, &info, &opts, None).map_err(|e| format!("spv: {e:?}"))?;
    Ok(words.iter().flat_map(|w| w.to_le_bytes()).collect())
}
