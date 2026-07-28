//! `VulkanFramebuffer` (a bind target) and `VulkanFrame` (an in-progress frame).
//!
//! The frame model bridges Smithay's incremental `Frame` API (render → clear →
//! draw* → finish) onto the piece-crates' single-pass `record_composition`
//! helper: each `clear`/`draw_solid`/`render_texture_*` call appends a `DrawOp`,
//! and `finish()` replays them inside one `record_composition` pass, then
//! submits. Foundation simplifications (marked below): the clear is approximated
//! as a full-target clear, and submission is synchronous (`device_wait_idle`).

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use compositor_kernel_vulkan_pipeline_composite_base::composite::PushQuad;
use smithay::backend::renderer::sync::SyncPoint;
use smithay::backend::renderer::{Color32F, ContextId, Frame, Texture};
use smithay::backend::allocator::Fourcc;
use smithay::utils::{Buffer as BufferCoord, Physical, Point, Rectangle, Size, Transform};
use std::marker::PhantomData;

use crate::error::VulkanError;
use crate::renderer::VulkanRenderer;
use crate::texture::VulkanTexture;

/// Render-target objects parked for deferred destruction: the GPU may still be
/// compositing into the image when the framebuffer drops (the native IN_FENCE
/// path returns from submit before completion), so destruction waits for a
/// point where the frame is provably done (`VulkanRenderer::drain_retired`).
pub(crate) struct RetiredTarget {
    pub(crate) image: vk::Image,
    pub(crate) memory: vk::DeviceMemory,
    pub(crate) view: vk::ImageView,
}

/// A render target the renderer can draw into. Owns the color-attachment image
/// imported from the bound dmabuf; retired to the renderer when the
/// framebuffer drops, destroyed once the frame that used it has completed.
pub struct VulkanFramebuffer<'buffer> {
    pub(crate) device: ash::Device,
    pub(crate) retire: std::sync::Arc<std::sync::Mutex<Vec<RetiredTarget>>>,
    pub(crate) image: vk::Image,
    pub(crate) memory: vk::DeviceMemory,
    pub(crate) view: vk::ImageView,
    pub(crate) format: vk::Format,
    pub(crate) fourcc: Option<Fourcc>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// This framebuffer owns its imported objects and retires them on drop.
    /// False for a target served by the renderer's import cache, which keeps
    /// them alive across frames and owns their destruction.
    pub(crate) owned: bool,
    /// How the composite must acquire this target's existing contents.
    pub(crate) acquire: TargetAcquire,
    pub(crate) _marker: PhantomData<&'buffer mut ()>,
}

/// How a bound target's existing contents must be taken over — the difference
/// between a partial redraw that keeps the undamaged remainder and one that
/// composites over garbage.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TargetAcquire {
    /// Nothing of ours has ever been composited into this dmabuf, so there is
    /// nothing to preserve: discard, and clear the whole target this frame.
    Fresh,
    /// The SAME `VkImage` a previous composite left in `GENERAL`, released to the
    /// display engine. The driver tracks its layout, so it can be acquired back
    /// from there with its contents intact.
    Cached,
}

impl TargetAcquire {
    /// The `oldLayout` for the acquire barrier, or `None` when there is nothing
    /// worth preserving (the caller then takes the full-clear path).
    pub(crate) fn old_layout(self) -> Option<vk::ImageLayout> {
        match self {
            TargetAcquire::Fresh => None,
            TargetAcquire::Cached => Some(vk::ImageLayout::GENERAL),
        }
    }
}

impl std::fmt::Debug for VulkanFramebuffer<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VulkanFramebuffer")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl Texture for VulkanFramebuffer<'_> {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    fn size(&self) -> Size<i32, BufferCoord> {
        Size::from((self.width as i32, self.height as i32))
    }
    fn format(&self) -> Option<Fourcc> {
        self.fourcc
    }
}

impl Drop for VulkanFramebuffer<'_> {
    fn drop(&mut self) {
        // Cached target: the renderer keeps these objects for the next frame's
        // bind, so dropping the framebuffer must not retire them.
        if !self.owned {
            return;
        }
        let target = RetiredTarget {
            image: self.image,
            memory: self.memory,
            view: self.view,
        };
        if let Ok(mut retired) = self.retire.lock() {
            retired.push(target);
        } else {
            // Poisoned retire list (a panic elsewhere): destroy eagerly rather
            // than leak — the process is going down anyway.
            unsafe {
                if target.view != vk::ImageView::null() {
                    self.device.destroy_image_view(target.view, None);
                }
                if target.image != vk::Image::null() {
                    self.device.destroy_image(target.image, None);
                }
                if target.memory != vk::DeviceMemory::null() {
                    self.device.free_memory(target.memory, None);
                }
            }
        }
    }
}

/// One resolved fullscreen-shader variant queued for this frame: the SPIR-V
/// module + entry points (cached as a `FullscreenPass` keyed by `id`) and the
/// owned push-constant bytes. The producing scene element owns all shader
/// specifics; the renderer just runs it.
pub(crate) struct ShaderVariant {
    pub id: u64,
    pub spv: Vec<u8>,
    /// Separate vertex-stage module (set when the fragment was compiled alone,
    /// e.g. a `glsl/` bundle paired with a fullscreen vertex).
    pub vert_spv: Option<Vec<u8>>,
    pub vert_entry: String,
    pub frag_entry: String,
    pub push: Vec<u8>,
}

/// A single queued draw: a solid fill or a textured quad. Order is preserved
/// (back-to-front z-order is the call order from the scene).
pub(crate) enum DrawOp {
    Solid {
        quad: PushQuad,
        /// The op's damage rects in OUTPUT space, one scissor per draw. Empty ⇒
        /// the op contributes nothing this frame. Ignored when the target had
        /// nothing to preserve and the whole frame is redrawn.
        scissors: Vec<vk::Rect2D>,
    },
    Textured {
        view: vk::ImageView,
        quad: PushQuad,
        scissors: Vec<vk::Rect2D>,
        /// Per-surface HDR composite flag `[transfer, is_hdr, 0, 0]` (M5).
        surf: [f32; 4],
        /// Source texture dimensions — the size of the mipped copy the AA
        /// trilinear/aniso modes render this surface into.
        tex_w: u32,
        tex_h: u32,
        /// Per-element metadata (space, …), tagged by the scene wrapper. AA is
        /// applied only to `World` elements; screen/background are not.
        meta: compositor_orchestration_draw_dispatch_frame::ElementMeta,
    },
    /// A fullscreen native shader pass (e.g. the parallax background): the SDR
    /// variant plus an optional HDR-output variant; `submit_frame` builds/caches
    /// a `FullscreenPass` per variant and picks by the active output mode.
    ShaderPass {
        sdr: ShaderVariant,
        hdr: Option<ShaderVariant>,
        scissors: Vec<vk::Rect2D>,
    },
}

/// Clamp one element-local damage rect into `dst` and lift it to output space —
/// the scissor for that draw. `None` when the rect falls outside `dst` entirely.
/// Mirrors the GLES frame's per-instance damage constraint.
pub(crate) fn scissor_for(
    dst: Rectangle<i32, Physical>,
    damage: Rectangle<i32, Physical>,
) -> Option<vk::Rect2D> {
    let local = Rectangle::from_loc_and_size((0, 0), dst.size).intersection(damage)?;
    if local.size.w <= 0 || local.size.h <= 0 {
        return None;
    }
    Some(vk::Rect2D {
        offset: vk::Offset2D {
            x: local.loc.x + dst.loc.x,
            y: local.loc.y + dst.loc.y,
        },
        extent: vk::Extent2D {
            width: local.size.w as u32,
            height: local.size.h as u32,
        },
    })
}

/// The scissor list for one element draw (element-local `damage` against `dst`).
pub(crate) fn scissors_for(
    dst: Rectangle<i32, Physical>,
    damage: &[Rectangle<i32, Physical>],
) -> Vec<vk::Rect2D> {
    damage.iter().filter_map(|d| scissor_for(dst, *d)).collect()
}

pub struct VulkanFrame<'frame, 'buffer> {
    pub(crate) renderer: &'frame mut VulkanRenderer,
    pub(crate) framebuffer: &'frame mut VulkanFramebuffer<'buffer>,
    pub(crate) output_size: Size<i32, Physical>,
    pub(crate) transform: Transform,
    pub(crate) clear: [f32; 4],
    /// The rects smithay asked to be cleared, in OUTPUT space (unlike
    /// per-element damage, these arrive unshifted). Unused on a full redraw,
    /// which clears the whole target.
    pub(crate) clear_rects: Vec<vk::Rect2D>,
    pub(crate) ops: Vec<DrawOp>,
    /// Metadata for the element currently being drawn (its space, etc.). Set per
    /// element by the scene wrapper via `SceneDispatch::set_element_meta`; read in
    /// `render_texture_from_to`. Defaults to `Screen` so anything the wrapper
    /// doesn't tag (never, in practice) stays on the plain path.
    pub(crate) current_meta: compositor_orchestration_draw_dispatch_frame::ElementMeta,
}

impl VulkanFrame<'_, '_> {
    fn extent(&self) -> (u32, u32) {
        (self.framebuffer.width, self.framebuffer.height)
    }
}

impl Frame for VulkanFrame<'_, '_> {
    type Error = VulkanError;
    type TextureId = VulkanTexture;

    fn context_id(&self) -> ContextId<VulkanTexture> {
        self.renderer.context_id_value()
    }

    fn clear(&mut self, color: Color32F, at: &[Rectangle<i32, Physical>]) -> Result<(), VulkanError> {
        // `at` is already output-space; the composite clears exactly these rects
        // and preserves the rest, falling back to a full-target clear only when
        // the bound target had no contents worth keeping.
        self.clear = [color.r(), color.g(), color.b(), color.a()];
        let out = self.extent();
        let full = Rectangle::from_loc_and_size((0, 0), (out.0 as i32, out.1 as i32));
        self.clear_rects = at
            .iter()
            .filter_map(|r| crate::frame::scissor_for(full, *r))
            .collect();
        Ok(())
    }

    fn draw_solid(
        &mut self,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        color: Color32F,
    ) -> Result<(), VulkanError> {
        let out = self.extent();
        let quad = compositor_kernel_vulkan_element_solid_base::solid::quad(
            out,
            (dst.loc.x, dst.loc.y, dst.size.w, dst.size.h),
            [color.r(), color.g(), color.b(), color.a()],
        );
        self.ops.push(DrawOp::Solid {
            quad,
            scissors: crate::frame::scissors_for(dst, damage),
        });
        Ok(())
    }

    fn render_texture_from_to(
        &mut self,
        texture: &VulkanTexture,
        src: Rectangle<f64, BufferCoord>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _src_transform: Transform,
        alpha: f32,
    ) -> Result<(), VulkanError> {
        let out = self.extent();
        let tw = texture.width().max(1) as f32;
        let th = texture.height().max(1) as f32;
        let src_uv = (
            (src.loc.x as f32) / tw,
            (src.loc.y as f32) / th,
            (src.size.w as f32) / tw,
            (src.size.h as f32) / th,
        );
        let quad = compositor_kernel_vulkan_element_texture_base::texture::quad(
            out,
            (dst.loc.x, dst.loc.y, dst.size.w, dst.size.h),
            src_uv,
            alpha,
        );
        // Pin the texture until the frame that samples it provably completes —
        // the native IN_FENCE path returns from submit while the GPU still
        // reads it (see `VulkanRenderer::pinned_textures`).
        self.renderer.pinned_textures.push(texture.clone());
        // AA targets ONLY world content (windows + iced-world), tagged per
        // element by the scene wrapper (`set_element_meta`). Screen-space iced
        // (settings/picker) and the bevy background are never eligible.
        self.ops.push(DrawOp::Textured {
            view: texture.view(),
            quad,
            scissors: crate::frame::scissors_for(dst, damage),
            surf: texture.surf(),
            tex_w: texture.width().max(1),
            tex_h: texture.height().max(1),
            meta: self.current_meta,
        });
        Ok(())
    }

    fn transformation(&self) -> Transform {
        self.transform
    }

    fn output_size(&self) -> Size<i32, Physical> {
        self.output_size
    }

    fn wait(&mut self, _sync: &SyncPoint) -> Result<(), VulkanError> {
        // Foundation: submission is synchronous (finish() waits the device), so
        // an explicit cross-frame wait is a no-op. Real acquire-fence waits
        // bridge through vulkan.sync once async submission lands.
        Ok(())
    }

    fn finish(self) -> Result<SyncPoint, VulkanError> {
        let extent = self.extent();
        let acquire = self.framebuffer.acquire;
        self.renderer.submit_frame(
            self.framebuffer.image,
            self.framebuffer.view,
            self.framebuffer.format,
            extent,
            self.clear,
            self.clear_rects,
            acquire,
            self.ops,
        )
    }
}
