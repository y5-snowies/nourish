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

use crate::error::VulkanError;
use crate::frame::{DrawOp, ShaderVariant, VulkanFrame};
use crate::renderer::VulkanRenderer;

/// Move a seam shader variant into an owned `DrawOp` variant. Only `push` is
/// copied — it must outlive this dispatch call and is ~112 bytes. The SPIR-V
/// modules and entry names are `Arc`s shared with the producing element, so
/// this is a refcount bump: they are read at most once, on the first
/// `ensure_shader_pass` cache miss, but this runs on every draw of every frame.
fn own_variant(v: SeamVariant<'_>) -> ShaderVariant {
    ShaderVariant {
        id: v.id,
        spv: v.spv,
        vert_spv: v.vert_spv,
        vert_entry: v.vert_entry,
        frag_entry: v.frag_entry,
        push: v.push.into_owned(),
    }
}

impl SceneDispatch for VulkanRenderer {
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
        pass: NativeShaderPass<'_>,
    ) -> Result<(), VulkanError> {
        // Native fullscreen shader: queue a generic shader pass carrying the
        // producer's SPIR-V + push bytes (the GLES program/uniforms are unused
        // here). Replayed by a `FullscreenPass` during `submit_frame`.
        frame.ops.push(DrawOp::ShaderPass {
            sdr: own_variant(pass.sdr),
            hdr: pass.hdr.map(own_variant),
            scissors: crate::frame::scissors_for(dst, damage),
        });
        Ok(())
    }
}
