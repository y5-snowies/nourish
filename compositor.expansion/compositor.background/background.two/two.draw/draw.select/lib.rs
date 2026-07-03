//! Background-shader selection + the loaded-shader Vulkan pass builder, split
//! out of `draw.parallax` so the render element stays within the size policy.

use compositor_background_two_shader_spirv::VulkanModule;
use compositor_orchestration_draw_dispatch_frame::{NativeShaderPass, ParallaxUniforms, ShaderVariant};
use smithay::backend::renderer::gles::{GlesPixelProgram, GlesRenderer};
use std::borrow::Cow;
use std::sync::Arc;

/// Resolve the active background shader for `selection` (a bundle name/path):
/// a runtime-loaded GLES program or Vulkan module for the active renderer,
/// falling back to the built-in `spacev3` (GLES) / native parallax (Vulkan).
/// Also returns the effective `@prop` params: the shader's declared defaults,
/// with `params_override` (the per-world edited values) overlaid slot-for-slot.
pub fn build(
    renderer: &mut GlesRenderer,
    selection: Option<&str>,
    params_override: &[(String, f32)],
) -> (Option<GlesPixelProgram>, Option<Arc<VulkanModule>>, [f32; 16], Option<String>) {
    let prefers_dmabuf =
        compositor_developer_stats_registry_base::base::compositor_prefers_dmabuf();
    let (loaded, error) = match selection {
        Some(s) => compositor_background_two_shader_load::load(renderer, prefers_dmabuf, s),
        None => (None, None),
    };
    // Effective params: the shader's declared defaults, then this world's overrides
    // matched by prop NAME (slot = the prop's index in declaration order).
    let props = loaded
        .as_ref()
        .map(|l| l.properties.clone())
        .unwrap_or_else(compositor_background_two_shader_builtin::builtin_props);
    let mut params = compositor_background_two_shader_property::default_params(&props);
    for (name, val) in params_override {
        if let Some(slot) = props.iter().position(|p| &p.name == name) {
            if slot < 16 {
                params[slot] = *val;
            }
        }
    }
    let (loaded_gles, vulkan) = match loaded {
        Some(l) => (l.gles, l.vulkan.map(Arc::new)),
        None => (None, None),
    };
    // GLES: loaded program or built-in. Vulkan: no GLES program (never sampled).
    let program = if prefers_dmabuf {
        None
    } else {
        loaded_gles
            .or_else(|| Some(compositor_background_two_draw_program::compile_program(renderer)))
    };
    (program, vulkan, params, error)
}

/// The dispatch-seam pass for a runtime-loaded Vulkan shader: SDR only (no HDR
/// variant — the renderer reuses `sdr`), with the standard engine + params push.
pub fn loaded_pass<'a>(
    m: &'a VulkanModule,
    u: &ParallaxUniforms,
    params: &[f32; 16],
) -> NativeShaderPass<'a> {
    NativeShaderPass {
        sdr: ShaderVariant {
            id: m.id,
            spv: Cow::Borrowed(&m.spv),
            vert_spv: m.vert_spv.as_deref().map(Cow::Borrowed),
            vert_entry: Cow::Borrowed(&m.vert_entry),
            frag_entry: Cow::Borrowed(&m.frag_entry),
            push: Cow::Owned(
                compositor_background_two_draw_vulkan::vulkan::engine_push(u, params).to_vec(),
            ),
        },
        hdr: None,
    }
}
