//! `VulkanRenderer`'s implementation of the scene-dispatch seam.
//!
//! Blank for now: the GLES-resource elements (iced UI, bevy 3D, parallax pixel
//! shader) carry `GlesTexture`/`GlesPixelProgram` that the Vulkan path cannot
//! consume directly. Per the integration plan, these no-op on Vulkan until each
//! element grows a renderer-native (dmabuf-import / vulkan composite) path. This
//! is the sanctioned "blank draw for non-GLES renderers" hook.

use smithay::backend::renderer::gles::{GlesPixelProgram, GlesTexture, Uniform};
use smithay::utils::{Buffer as BufferCoord, Physical, Rectangle, Size};
use compositor_orchestration_draw_dispatch_frame::{ElementMeta, NativeShaderPass, SceneDispatch};
use compositor_orchestration_draw_dispatch_frame::ShaderVariant as SeamVariant;
use compositor_pipeline_abi_seam_base::base::{PipelineOutput, ShaderPipeline as SeamPipeline, TargetFormat};
use compositor_pipeline_execute_graph_base::graph::{
    GraphFormat, GraphOutput, GraphPass, GraphPipeline,
};

use crate::error::VulkanError;
use crate::frame::{DrawOp, ShaderVariant, VulkanFrame};
use crate::renderer::VulkanRenderer;

/// Move a seam shader variant into an owned `DrawOp` variant. Only `push` is
/// copied — it must outlive this dispatch call and is ~112 bytes. The SPIR-V
/// modules and entry names are `Arc`s shared with the producing element, so
/// this is a refcount bump: they are read at most once, on the first
/// `ensure_shader_pass` cache miss, but this runs on every draw of every frame.
fn own_variant(v: SeamVariant) -> ShaderVariant {
    ShaderVariant {
        id: v.id,
        spv: v.spv,
        vert_spv: v.vert_spv,
        vert_entry: v.vert_entry,
        frag_entry: v.frag_entry,
        push: v.push.to_vec(),
    }
}

impl SceneDispatch for VulkanRenderer {
    fn set_after_band(&mut self, band: Option<smithay::backend::allocator::dmabuf::Dmabuf>) {
        self.after_band = band;
    }

    fn set_band_machinery(&mut self, pass: Option<NativeShaderPass>) {
        // The SAME conversion `draw_pixel_program` performs on the element path,
        // so the injected op is byte-for-byte what the band's draw would have
        // pushed. A pass without a pipeline handle (single-pass shader) carries no
        // after-content machinery and publishes nothing.
        self.band_machinery = pass
            .as_ref()
            .and_then(|p| p.pipeline.as_ref())
            .and_then(|p| compositor_pipeline_abi_seam_base::base::as_pipeline(Some(p)))
            .map(|gp| {
                std::sync::Arc::new(compositor_pipeline_execute_graph_base::graph::from_seam(
                    gp.clone(),
                ))
            });
    }

    // Vulkan consumes iced/bevy/parallax output via dmabuf import (PreImported),
    // not the GLES-welded seam below.
    fn prefers_dmabuf() -> bool {
        true
    }

    fn set_element_meta(frame: &mut VulkanFrame<'_, '_>, meta: ElementMeta) {
        // Stamp the current element's metadata; `render_texture_from_to` reads it
        // so AA is applied only to world content (windows + iced-world).
        frame.current_meta = meta;
    }

    fn draw_prerendered_texture(
        _frame: &mut VulkanFrame<'_, '_>,
        _texture: &GlesTexture,
        _src: Rectangle<f64, BufferCoord>,
        _dst: Rectangle<i32, Physical>,
        _damage: &[Rectangle<i32, Physical>],
        _alpha: f32,
    ) -> Result<(), VulkanError> {
        // Blank until a dmabuf-imported vulkan texture path lands for these elements.
        Ok(())
    }

    fn draw_pixel_program(
        frame: &mut VulkanFrame<'_, '_>,
        _program: Option<&GlesPixelProgram>,
        _src: Rectangle<f64, BufferCoord>,
        dst: Rectangle<i32, Physical>,
        _size: Size<i32, BufferCoord>,
        damage: &[Rectangle<i32, Physical>],
        _alpha: f32,
        _uniforms: &[Uniform<'_>],
        pass: NativeShaderPass,
    ) -> Result<(), VulkanError> {
        // A multipass bundle carries a `pipeline`: queue the whole graph, which
        // `submit_frame` runs via the graph executor. Otherwise queue the single
        // native fullscreen shader pass (SPIR-V + push), replayed by a
        // `FullscreenPass`. The GLES program/uniforms are unused on Vulkan.
        // The seam hands the pipeline across as an opaque handle, because
        // `dispatch.uniforms` describes how a scene element reaches a renderer and
        // has no business knowing what a multipass bundle is. THIS is the renderer
        // that understands one, so this is where it is named again.
        //
        // A downcast that fails means a second producer put something else in the
        // handle — a wiring mistake, not a runtime condition — so it is reported
        // once and the frame falls back to the single pass rather than drawing
        // nothing with no explanation.
        let pipeline = pass
            .pipeline
            .as_ref()
            .map(|p| compositor_pipeline_abi_seam_base::base::as_pipeline(Some(p)));
        match pipeline {
            Some(Some(gp)) => {
                frame.ops.push(DrawOp::Pipeline(std::sync::Arc::new(
                    compositor_pipeline_execute_graph_base::graph::from_seam(gp.clone()),
                )));
            }
            Some(None) => {
                static ONCE: std::sync::Once = std::sync::Once::new();
                ONCE.call_once(|| {
                    error!(
                        "draw seam: NativeShaderPass carried a pipeline handle this renderer                          does not recognise; falling back to the single pass"
                    )
                });
                frame.ops.push(DrawOp::ShaderPass {
                    sdr: own_variant(pass.sdr),
                    hdr: pass.hdr.map(own_variant),
                    scissors: crate::frame::scissors_for(dst, damage),
                });
            }
            None => {
                frame.ops.push(DrawOp::ShaderPass {
                    sdr: own_variant(pass.sdr),
                    hdr: pass.hdr.map(own_variant),
                    scissors: crate::frame::scissors_for(dst, damage),
                });
            }
        }
        Ok(())
    }
}
