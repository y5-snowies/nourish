//! `VulkanRenderer`'s implementation of the scene-dispatch seam.
//!
//! Blank for now: the GLES-resource elements (iced UI, bevy 3D, parallax pixel
//! shader) carry `GlesTexture`/`GlesPixelProgram` that the Vulkan path cannot
//! consume directly. Per the integration plan, these no-op on Vulkan until each
//! element grows a renderer-native (dmabuf-import / vulkan composite) path. This
//! is the sanctioned "blank draw for non-GLES renderers" hook.

use smithay::backend::renderer::gles::{GlesPixelProgram, GlesTexture, Uniform};
use smithay::utils::{Buffer as BufferCoord, Physical, Rectangle, Size};
use compositor_orchestration_draw_dispatch_frame::{NativeShaderPass, SceneDispatch};
use compositor_orchestration_draw_dispatch_frame::ShaderVariant as SeamVariant;

use crate::error::VulkanError;
use crate::frame::{DrawOp, ShaderVariant, VulkanFrame};
use crate::renderer::VulkanRenderer;

/// Copy a seam shader variant into an owned `DrawOp` variant (the push bytes
/// must outlive this dispatch call, so they're copied into the queued op).
fn own_variant(v: SeamVariant<'_>) -> ShaderVariant {
    ShaderVariant {
        id: v.id,
        spv: v.spv.into_owned(),
        vert_spv: v.vert_spv.map(|s| s.into_owned()),
        vert_entry: v.vert_entry.into_owned(),
        frag_entry: v.frag_entry.into_owned(),
        push: v.push.into_owned(),
    }
}

impl SceneDispatch for VulkanRenderer {
    // Vulkan consumes iced/bevy/parallax output via dmabuf import (PreImported),
    // not the GLES-welded seam below.
    fn prefers_dmabuf() -> bool {
        true
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
        _dst: Rectangle<i32, Physical>,
        _size: Size<i32, BufferCoord>,
        _damage: &[Rectangle<i32, Physical>],
        _alpha: f32,
        _uniforms: &[Uniform<'_>],
        pass: NativeShaderPass<'_>,
    ) -> Result<(), VulkanError> {
        // Native fullscreen shader: queue a generic shader pass carrying the
        // producer's SPIR-V + push bytes (the GLES program/uniforms are unused
        // here). Replayed by a `FullscreenPass` during `submit_frame`.
        frame.ops.push(DrawOp::ShaderPass {
            sdr: own_variant(pass.sdr),
            hdr: pass.hdr.map(own_variant),
        });
        Ok(())
    }
}
