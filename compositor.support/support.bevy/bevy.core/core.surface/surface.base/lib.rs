//! `BevySurface`: the GPU backing for one Bevy instance's output. Two shapes,
//! mirroring the iced bridge (`document/GPU_UNTILE_BLIT.md`):
//!
//! - **Single** (today) — ONE dmabuf, imported as both a `wgpu::Texture` Bevy
//!   renders into and a `GlesTexture` the compositor samples. Correct on a single
//!   GPU and on a split system with a non-empty render∩scanout modifier intersection.
//! - **Blit** — the untiling-blit floor for a split system with an EMPTY
//!   intersection: a render-GPU-native tiled render target plus a `LINEAR` buffer
//!   (both on the render node), with a per-frame GPU copy tiled→linear.

use compositor_support_bevy_core_alloc_base::{
    AllocatedDmabuf, allocate_dmabuf, allocate_dmabuf_negotiated, allocate_linear_on,
};
use compositor_support_bevy_core_context_base::WgpuVulkanContext;
use compositor_support_bevy_core_fault_base::SurfaceError;
use compositor_support_bevy_core_gles_base::import_dmabuf_to_gles;
use compositor_support_bevy_core_import_base::{
    TEXTURE_FORMAT, import_dmabuf_to_wgpu, import_dmabuf_to_wgpu_transfer_dst,
};
use compositor_developer_debug_instance_record::{info, warn};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::utils::{Physical, Size};

/// One render target, addressable from both wgpu and GLES.
pub struct BevySurface {
    backing: Backing,
    /// Logical size (equals texture extent today). Public for callers that place
    /// and resize the surface.
    pub size: Size<i32, Physical>,
}

/// Single- vs two-buffer backing. See the module docs and [`Backing::allocate`].
enum Backing {
    Single(SingleBacking),
    Blit(BlitBacking),
}

/// Drop order is load-bearing: `gles_texture` (EGLImage) before `wgpu_texture`
/// (Vulkan external-mem binding) before `allocated` (owns the BO).
struct SingleBacking {
    gles_texture: GlesTexture,
    wgpu_texture: wgpu::Texture,
    allocated: AllocatedDmabuf,
}

/// Two-buffer untiling-blit backing (both buffers on the render node). Bevy renders
/// into `wgpu_render` (tiled); each frame the render GPU copies it into `wgpu_linear`
/// — a LINEAR buffer sampled by the scanout GLES via `gles_texture`. Field order
/// preserves the drop discipline: imports before the allocations they reference.
struct BlitBacking {
    gles_texture: GlesTexture,
    wgpu_render: wgpu::Texture,
    wgpu_linear: wgpu::Texture,
    allocated_render: AllocatedDmabuf,
    allocated_linear: AllocatedDmabuf,
    device: wgpu::Device,
    queue: wgpu::Queue,
    size: Size<i32, Physical>,
}

impl std::fmt::Debug for BevySurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BevySurface")
            .field("size", &self.size)
            .field("format", &TEXTURE_FORMAT)
            .finish()
    }
}

impl BevySurface {
    /// Allocate a fresh backing at the given size (single- or two-buffer per
    /// [`Backing::allocate`]) and import its views.
    pub fn allocate(
        render_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Result<Self, SurfaceError> {
        info!("BevySurface::allocate {}x{}", size.w, size.h);
        let backing = Backing::allocate(render_node, wgpu_ctx, gles, size)?;
        Ok(Self { backing, size })
    }

    /// Resize: destroy-and-recreate, drop-safe. On failure `*self` is unchanged.
    pub fn resize(
        &mut self,
        render_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
        new_size: Size<i32, Physical>,
    ) -> Result<(), SurfaceError> {
        if new_size == self.size {
            return Ok(());
        }
        info!(
            "BevySurface::resize {}x{} -> {}x{}",
            self.size.w, self.size.h, new_size.w, new_size.h
        );
        let replacement = BevySurface::allocate(render_node, wgpu_ctx, gles, new_size)?;
        let _old = std::mem::replace(self, replacement);
        Ok(())
    }

    /// The wgpu texture Bevy renders INTO (single: shared buffer; blit: tiled target).
    pub fn render_texture(&self) -> &wgpu::Texture {
        self.backing.render_texture()
    }

    /// The GLES texture the compositor samples (single: shared buffer; blit: the
    /// LINEAR copy destination).
    pub fn sample_gles(&self) -> &GlesTexture {
        self.backing.sample_gles()
    }

    /// The dmabuf handed to the scanout importer (single: shared buffer; blit: the
    /// LINEAR buffer).
    pub fn scanout_dmabuf(&self) -> &smithay::backend::allocator::dmabuf::Dmabuf {
        self.backing.scanout_dmabuf()
    }

    /// Run the post-render untiling blit in blit mode; a no-op in single mode. Call
    /// once, immediately after Bevy has rendered the frame into [`render_texture`].
    pub fn post_render_blit(&self) {
        self.backing.post_render_blit();
    }

    /// Produce a `wgpu::TextureView` of the render target for use as an attachment.
    pub fn create_render_view(&self) -> wgpu::TextureView {
        self.backing
            .render_texture()
            .create_view(&wgpu::TextureViewDescriptor {
                label: Some("y5_bevy_dmabuf_render_view"),
                ..Default::default()
            })
    }
}

impl Backing {
    fn render_texture(&self) -> &wgpu::Texture {
        match self {
            Backing::Single(b) => &b.wgpu_texture,
            Backing::Blit(b) => &b.wgpu_render,
        }
    }

    fn sample_gles(&self) -> &GlesTexture {
        match self {
            Backing::Single(b) => &b.gles_texture,
            Backing::Blit(b) => &b.gles_texture,
        }
    }

    fn scanout_dmabuf(&self) -> &smithay::backend::allocator::dmabuf::Dmabuf {
        match self {
            Backing::Single(b) => &b.allocated.dmabuf,
            Backing::Blit(b) => &b.allocated_linear.dmabuf,
        }
    }

    fn post_render_blit(&self) {
        let Backing::Blit(b) = self else { return };
        let mut encoder = b
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("y5_bevy_untile_blit"),
            });
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &b.wgpu_render,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &b.wgpu_linear,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: b.size.w as u32,
                height: b.size.h as u32,
                depth_or_array_layers: 1,
            },
        );
        b.queue.submit(std::iter::once(encoder.finish()));
    }

    /// Decide single-buffer vs untiling-blit (see [`crate`] docs). Blit mode only
    /// when the `gpu_untile_blit` experiment is on, a distinct scanout card is
    /// configured, AND the render∩scanout intersection is empty. A failed blit
    /// allocation degrades to the single-buffer path.
    fn allocate(
        render_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Result<Self, SurfaceError> {
        let fourcc = smithay::backend::allocator::Fourcc::Argb8888;
        let gles_formats = smithay::backend::renderer::ImportDma::dmabuf_formats(gles);

        if let Some(scanout_node) = blit_scanout_node() {
            let empty = compositor_kernel_graphic_bridge_negotiate_base::negotiate::bridge_intersection_empty(
                gles_formats.clone(),
                wgpu_ctx.importable.clone(),
                fourcc,
            );
            if empty {
                info!(
                    "untile-blit (bevy): split (scanout={scanout_node}) with empty modifier \
                     intersection; two-buffer backing {}x{}",
                    size.w, size.h
                );
                match BlitBacking::allocate(render_node, &scanout_node, wgpu_ctx, gles, size) {
                    Ok(b) => return Ok(Backing::Blit(b)),
                    Err(e) => warn!(
                        "untile-blit (bevy) backing failed ({e:?}); degrading to single-buffer path"
                    ),
                }
            }
        }

        // Single-buffer path (today).
        let mods = compositor_kernel_graphic_bridge_negotiate_base::negotiate::bridge_modifiers(
            gles_formats,
            wgpu_ctx.importable.clone(),
            fourcc,
        );
        let allocated =
            allocate_dmabuf_negotiated(render_node, size.w as u32, size.h as u32, fourcc, &mods)?;
        let gles_texture = import_dmabuf_to_gles(gles, &allocated.dmabuf)?;
        let wgpu_texture = import_dmabuf_to_wgpu(wgpu_ctx, &allocated.dmabuf)?;
        Ok(Backing::Single(SingleBacking {
            gles_texture,
            wgpu_texture,
            allocated,
        }))
    }
}

impl BlitBacking {
    /// Allocate the two-buffer untiling-blit backing, both buffers on the RENDER
    /// node — the tiled render target Bevy draws into, and a LINEAR buffer imported
    /// as the render GPU's copy destination AND the scanout GLES sample source.
    /// Render-node placement keeps the untile copy same-device; the only cross-
    /// device step is a universal LINEAR dmabuf import. See `document/GPU_UNTILE_BLIT.md`.
    fn allocate(
        render_node: &str,
        scanout_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Result<Self, SurfaceError> {
        let (w, h) = (size.w as u32, size.h as u32);
        let fourcc = smithay::backend::allocator::Fourcc::Argb8888;

        let allocated_render = allocate_dmabuf(render_node, w, h)?;
        let wgpu_render = import_dmabuf_to_wgpu(wgpu_ctx, &allocated_render.dmabuf)?;

        // Prefer a render-node LINEAR buffer (same-device copy); fall back to the
        // scanout card if the render GPU won't allocate LINEAR.
        let allocated_linear = allocate_linear_on(render_node, w, h, fourcc).or_else(|e| {
            warn!(
                "untile-blit (bevy): LINEAR alloc on render node failed ({e:?}); \
                 falling back to scanout node {scanout_node}"
            );
            allocate_linear_on(scanout_node, w, h, fourcc)
        })?;
        let gles_texture = import_dmabuf_to_gles(gles, &allocated_linear.dmabuf)?;
        let wgpu_linear = import_dmabuf_to_wgpu_transfer_dst(wgpu_ctx, &allocated_linear.dmabuf)?;

        Ok(Self {
            gles_texture,
            wgpu_render,
            wgpu_linear,
            allocated_render,
            allocated_linear,
            device: wgpu_ctx.device.clone(),
            queue: wgpu_ctx.queue.clone(),
            size,
        })
    }
}

/// The scanout card path (decision signal), or `None` when the untiling blit does
/// not apply: experiment off, no distinct scanout card configured, or scanout ==
/// render. The empty-intersection check is done separately by the caller.
fn blit_scanout_node() -> Option<String> {
    use compositor_developer_environment_experimental_base::base as ex;
    if !ex::get().contains(ex::GpuFlags::UNTILE_BLIT) {
        return None;
    }
    let cfg = compositor_developer_environment_config_base::base::get();
    let scanout = cfg.scanout_node.as_ref()?;
    if *scanout == cfg.render_node {
        return None;
    }
    Some(scanout.clone())
}
