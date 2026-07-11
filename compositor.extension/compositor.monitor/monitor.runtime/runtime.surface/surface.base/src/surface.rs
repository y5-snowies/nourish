//! `IcedSurface`: one DMABUF, imported as both a `wgpu::Texture` and a
//! `GlesTexture`.
//!
//! This is the unit the engine renders into and the compositor samples from.
//! Each Iced instance owns one. Allocation, import, and resize are managed
//! here so callers never have to think about drop ordering across the three
//! views.

use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::utils::{Physical, Size};

use crate::dmabuf_alloc::{
    AllocatedDmabuf, allocate_dmabuf, allocate_dmabuf_negotiated, allocate_linear_on,
};
use crate::error::SurfaceError;
use crate::gles_import::import_dmabuf_to_gles;
use crate::wgpu_context::WgpuVulkanContext;
use crate::wgpu_import::{
    TEXTURE_FORMAT, import_dmabuf_to_wgpu, import_dmabuf_to_wgpu_transfer_dst,
};

/// One render target, addressable from both wgpu and GLES.
///
/// ## Drop ordering
/// Fields are declared in the order they must drop. Rust drops struct fields
/// top-down, and the order matters because:
///   1. `gles_texture` holds an EGLImage that references the dmabuf.
///   2. `wgpu_texture` holds a Vulkan external-memory binding that references
///      the dmabuf fd.
///   3. `allocated` owns the underlying GPU memory; releasing it while either
///      import is alive can hit driver assertions.
///
/// Do not reorder these without re-doing the lifetime analysis.
pub struct IcedSurface {
    /// GPU backing (dmabuf + both imports). `None` when the surface has been
    /// **released** to reclaim memory while it isn't visible — its `IcedRuntime`
    /// keeps running; `ensure` re-allocates on demand before the next render.
    /// The three imports are kept together so they drop in the required order
    /// (gles → wgpu → allocation) on both release and resize.
    backing: Option<Backing>,
    /// The logical size of the surface, retained across release so a released
    /// surface can be re-allocated at the same size without the caller re-stating it.
    pub size: Size<i32, Physical>,
}

/// The GPU backing for one bridge surface. Two shapes:
///
/// - [`Backing::Single`] — today's zero-copy path: ONE dmabuf, imported as both a
///   wgpu render target and a GLES sample source. Correct on a single GPU and on a
///   split system whose render∩scanout modifier intersection is non-empty (UMA /
///   same-vendor).
/// - [`Backing::Blit`] — the untiling-blit floor for a split system with an EMPTY
///   intersection: TWO dmabufs (render-GPU tiled target + scanout-card LINEAR),
///   with a per-frame GPU copy between them. See `document/GPU_UNTILE_BLIT.md`.
///
/// The decision is made once per allocation in [`Backing::allocate`].
enum Backing {
    Single(SingleBacking),
    Blit(BlitBacking),
}

/// The three views of one dmabuf, grouped so field-drop order is guaranteed:
/// `gles_texture` (EGLImage) → `wgpu_texture` (Vulkan external-mem binding) →
/// `allocated` (owns the BO). See the type-level note on `IcedSurface`.
struct SingleBacking {
    gles_texture: GlesTexture,
    wgpu_texture: wgpu::Texture,
    allocated: AllocatedDmabuf,
}

/// Two-buffer backing for the untiling blit. Iced renders into `wgpu_render` (the
/// render GPU's native tiled buffer); after each frame the render GPU copies it
/// into `wgpu_linear` — a `LINEAR` buffer physically allocated on the SCANOUT card
/// and imported here as a copy destination — and the scanout side samples that
/// same LINEAR buffer via `gles_texture`. See `document/GPU_UNTILE_BLIT.md`.
///
/// ## Drop ordering (fields drop top-down)
///   1. `gles_texture` — EGLImage referencing `allocated_linear`.
///   2. `wgpu_render` — Vulkan binding referencing `allocated_render`.
///   3. `wgpu_linear` — Vulkan binding referencing `allocated_linear`.
///   4. `allocated_render` / `allocated_linear` — own the BOs; must outlive every
///      import above.
/// `device`/`queue` are cheap clones kept so the blit can be submitted from
/// `render()` (which has no `WgpuVulkanContext` in hand); they drop last, harmless.
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

impl std::fmt::Debug for IcedSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IcedSurface")
            .field("size", &self.size)
            .field("format", &TEXTURE_FORMAT)
            .finish()
    }
}

impl IcedSurface {
    /// DEBUG ONLY: clear the wgpu texture to a solid color via a tiny render pass.
    /// Useful for proving the wgpu->GLES round-trip works independent of iced.
    pub fn debug_clear(
        &self,
        wgpu_ctx: &crate::wgpu_context::WgpuVulkanContext,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) {
        // DEBUG ONLY: no-op when the backing has been released.
        let Some(backing) = self.backing.as_ref() else { return };
        let render_texture = backing.render_texture();
        // Step 1: Clear to color.
        let view = render_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("y5_iced_dmabuf_render_view"),
            ..Default::default()
        });
        let mut encoder = wgpu_ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("debug_clear"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                occlusion_query_set: None, // <-=- added 2 fields, removed one
                multiview_mask: None,
                label: Some("debug_clear_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
            });
        }
        // Step 2: Copy the cleared texture to a CPU-readable staging buffer.
        const ALIGN: u32 = 256;
        let bytes_per_pixel = 4u32;
        let unaligned_bpr = bytes_per_pixel * self.size.w as u32;
        let bytes_per_row = unaligned_bpr.div_ceil(ALIGN) * ALIGN;
        let staging_size = (bytes_per_row * self.size.h as u32) as u64;

        let staging = wgpu_ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("debug_clear_staging"),
            size: staging_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: render_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(self.size.h as u32),
                },
            },
            wgpu::Extent3d {
                width: self.size.w as u32,
                height: self.size.h as u32,
                depth_or_array_layers: 1,
            },
        );

        // wgpu_ctx.queue.submit(std::iter::once(encoder.finish()));

        let submission_index = wgpu_ctx.queue.submit(std::iter::once(encoder.finish()));

        info!("wait 1");
        let _ = wgpu_ctx.device.poll(wgpu::PollType::Wait {
            timeout: None,
            submission_index: Some(submission_index),
        });

        info!("wait 1 OK");

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = wgpu_ctx.device.poll(wgpu::PollType::Wait {
            timeout: None,
            submission_index: None,
        });
        let _ = rx.recv();

        let data = slice.get_mapped_range();
        let data = data.unwrap();
        let bytes = data.to_vec();

        // Sample pixel (0, 0):
        let pixel_0 = &bytes[0..4];

        // Sample pixel (w/2, h/2) — accounting for row padding:
        let mid_y = self.size.h as usize / 2;
        let mid_x = self.size.w as usize / 2;
        let mid_offset = mid_y * bytes_per_row as usize + mid_x * 4;
        let pixel_mid = &bytes[mid_offset..mid_offset + 4];

        info!(
            "debug_clear readback: pixel(0,0)={:?}, pixel(mid)={:?}, bytes_per_row={}, expected BGRA (assuming you passed r,g,b,a): B={} G={} R={} A={}",
            pixel_0,
            pixel_mid,
            bytes_per_row,
            (b * 255.0) as u8,
            (g * 255.0) as u8,
            (r * 255.0) as u8,
            (a * 255.0) as u8,
        );

        drop(data);
        staging.unmap();
    }
    /// Allocate a fresh dmabuf at the given size and import it as both a
    /// wgpu texture and a GLES texture. Starts out resident.
    pub fn allocate(
        render_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Result<Self, SurfaceError> {
        let backing = Backing::allocate(render_node, wgpu_ctx, gles, size)?;
        Ok(Self {
            backing: Some(backing),
            size,
        })
    }

    /// Whether the GPU backing is currently allocated. `false` after `release`
    /// and before the next `ensure`.
    pub fn is_resident(&self) -> bool {
        self.backing.is_some()
    }

    /// Free the GPU backing (dmabuf + both imports) while keeping `size`. The
    /// imports drop in the required order (gles → wgpu → allocation). No-op if
    /// already released. Re-`ensure` before rendering or sampling again.
    pub fn release(&mut self) {
        if self.backing.is_some() {
            trace!("IcedSurface::release {}x{}", self.size.w, self.size.h);
        }
        self.backing = None;
    }

    /// Re-allocate the backing at the current `size` if it was released. No-op
    /// if already resident.
    pub fn ensure(
        &mut self,
        render_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
    ) -> Result<(), SurfaceError> {
        if self.backing.is_some() {
            return Ok(());
        }
        self.backing = Some(Backing::allocate(render_node, wgpu_ctx, gles, self.size)?);
        Ok(())
    }

    /// Sampleable GLES view, or `None` while released. In blit mode this is the
    /// LINEAR scanout-card buffer (the copy destination), not the tiled render
    /// target — i.e. exactly the buffer the scanout side can import.
    pub fn gles_texture(&self) -> Option<&GlesTexture> {
        self.backing.as_ref().map(|b| b.gles_texture())
    }

    /// The dmabuf handed to the scanout importer, or `None` while released. In
    /// blit mode this is the LINEAR scanout-card buffer, not the tiled render
    /// target.
    pub fn dmabuf(&self) -> Option<&smithay::backend::allocator::dmabuf::Dmabuf> {
        self.backing.as_ref().map(|b| b.scanout_dmabuf())
    }

    /// Run the post-render untiling blit if this surface is in blit mode; a no-op
    /// in single-buffer mode and while released. Call once, immediately after the
    /// engine has rendered a frame into [`create_render_view`](Self::create_render_view).
    pub fn post_render_blit(&self) {
        if let Some(b) = self.backing.as_ref() {
            b.post_render_blit();
        }
    }

    /// Resize. Destroy-and-recreate in drop-safe order when resident; when
    /// released, only the retained `size` changes (the backing is re-allocated
    /// at the new size on the next `ensure`).
    ///
    /// On a resident resize, a replacement is allocated first so a failure
    /// leaves `*self` unchanged and the caller sees a clean error.
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

        trace!(
            "IcedSurface::resize {}x{} -> {}x{}",
            self.size.w, self.size.h, new_size.w, new_size.h
        );

        if self.backing.is_some() {
            // Allocate the replacement first (clean error on failure), then let
            // the old backing drop (gles → wgpu → allocation) as it is replaced.
            let replacement = Backing::allocate(render_node, wgpu_ctx, gles, new_size)?;
            self.backing = Some(replacement);
        }
        self.size = new_size;
        Ok(())
    }

    /// Convenience: produce a `wgpu::TextureView` for use as a render
    /// attachment, or `None` while released.
    pub fn create_render_view(&self) -> Option<wgpu::TextureView> {
        self.backing.as_ref().map(|b| {
            b.render_texture().create_view(&wgpu::TextureViewDescriptor {
                label: Some("y5_iced_dmabuf_render_view"),
                ..Default::default()
            })
        })
    }
}

impl Backing {
    /// The GLES texture the scanout side samples (single: the shared buffer; blit:
    /// the LINEAR scanout-card buffer).
    fn gles_texture(&self) -> &GlesTexture {
        match self {
            Backing::Single(b) => &b.gles_texture,
            Backing::Blit(b) => &b.gles_texture,
        }
    }

    /// The dmabuf handed to the scanout importer (single: the shared buffer; blit:
    /// the LINEAR scanout-card buffer).
    fn scanout_dmabuf(&self) -> &smithay::backend::allocator::dmabuf::Dmabuf {
        match self {
            Backing::Single(b) => &b.allocated.dmabuf,
            Backing::Blit(b) => &b.allocated_linear.dmabuf,
        }
    }

    /// The wgpu texture the engine renders INTO (single: the shared buffer; blit:
    /// the render-GPU native tiled target).
    fn render_texture(&self) -> &wgpu::Texture {
        match self {
            Backing::Single(b) => &b.wgpu_texture,
            Backing::Blit(b) => &b.wgpu_render,
        }
    }

    /// After the engine renders, copy the tiled render target into the LINEAR
    /// scanout buffer. No-op in single-buffer mode. Ordered on the same queue
    /// after the engine's submit, so the copy sees the finished frame.
    fn post_render_blit(&self) {
        let Backing::Blit(b) = self else { return };
        let mut encoder = b
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("y5_iced_untile_blit"),
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

    /// Decide single-buffer vs untiling-blit and allocate accordingly.
    ///
    /// STRICT: when a distinct scanout card is configured (split system) AND the
    /// render∩scanout modifier intersection is empty (no single buffer can satisfy
    /// both sides), the untiling blit is the ONLY correct path — so it is taken when
    /// the `untile_blit` mode token is set, and its absence (or a failed blit
    /// allocation) is a FATAL abort with an actionable message, never a silent degrade.
    /// In every other case — single GPU, UMA/same-vendor split with a shared modifier,
    /// or a non-empty intersection — the byte-identical single-buffer path is used.
    fn allocate(
        render_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Result<Self, SurfaceError> {
        trace!("IcedSurface backing allocate {}x{}", size.w, size.h);

        let fourcc = smithay::backend::allocator::Fourcc::Argb8888;
        let gles_formats = smithay::backend::renderer::ImportDma::dmabuf_formats(gles);

        // Split signal: a distinct scanout card is configured. Empty render∩scanout
        // intersection ⇒ no single buffer satisfies both sides, so the ONLY way to
        // serve this surface is the untiling blit — which must be explicitly enabled.
        if let Some(scanout_node) = distinct_scanout_node() {
            let empty = compositor_kernel_graphic_bridge_negotiate_base::negotiate::bridge_intersection_empty(
                gles_formats.clone(),
                wgpu_ctx.importable.clone(),
                fourcc,
            );
            if empty {
                if !untile_blit_enabled() {
                    abort!(
                        "FATAL: empty render∩scanout modifier intersection on a split system \
                         (scanout {scanout_node}): no single buffer satisfies both the render GPU \
                         and the scanout card, and the `untile_blit` mode token is not set. Add \
                         `untile_blit` to gpu_router[render].mode. See document/GPU_UNTILE_BLIT.md."
                    );
                }
                info!(
                    "untile-blit: split (scanout={scanout_node}) with empty modifier intersection; \
                     allocating two-buffer (render-node tiled + LINEAR copy target) backing {}x{}",
                    size.w, size.h
                );
                return match BlitBacking::allocate(render_node, &scanout_node, wgpu_ctx, gles, size) {
                    Ok(b) => Ok(Backing::Blit(b)),
                    // STRICT: no silent degrade to single-buffer — there is nothing softer
                    // than the blit, so a failed allocation is fatal (was: degrade + BAD_MATCH).
                    Err(e) => abort!(
                        "FATAL: untile-blit backing allocation failed for scanout {scanout_node} \
                         ({e:?}). The blit floor is mandatory here and has no fallback. \
                         See document/GPU_UNTILE_BLIT.md."
                    ),
                };
            }
        }

        // Single-buffer path (today): negotiate an explicit modifier across
        // gles ∩ wgpu (empty ⇒ implicit, byte-identical allocation).
        let mode = compositor_developer_environment_config_router::router::mode_for(
            std::path::Path::new(render_node),
        );
        let mods = compositor_kernel_graphic_bridge_negotiate_base::negotiate::bridge_modifiers(
            gles_formats,
            wgpu_ctx.importable.clone(),
            fourcc,
            mode,
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
    /// node:
    ///   - a render-GPU-native **tiled** buffer (implicit modifier) — the render
    ///     target iced draws into, and
    ///   - a **LINEAR** buffer, imported into the render GPU's wgpu as the copy
    ///     destination AND into the scanout GLES as the sample source.
    ///
    /// Allocating the LINEAR buffer on the render node (not the scanout card) is
    /// deliberate: it keeps the untile copy `tiled → linear` **same-device**, where
    /// the driver models both images' pitch self-consistently, instead of a raw
    /// cross-device `vkCmdCopyImage` into a foreign buffer whose row pitch Vulkan
    /// must model from the other card's gbm stride — the fragile step that showed up
    /// as partial-stretch shear. The only cross-device operation left is importing a
    /// plain LINEAR dmabuf into the scanout GLES, which is the universal interop path
    /// every driver handles. See `document/GPU_UNTILE_BLIT.md`.
    fn allocate(
        render_node: &str,
        scanout_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Result<Self, SurfaceError> {
        let (w, h) = (size.w as u32, size.h as u32);
        let fourcc = smithay::backend::allocator::Fourcc::Argb8888;

        // 1. Render-GPU tiled target (native modifier — the reliable wgpu import).
        let allocated_render = allocate_dmabuf(render_node, w, h)?;
        let wgpu_render = import_dmabuf_to_wgpu(wgpu_ctx, &allocated_render.dmabuf)?;

        // 2. LINEAR copy destination, PREFERABLY on the render node (keeps the copy
        //    same-device — no foreign-pitch shear). If the render GPU won't allocate
        //    LINEAR (some drivers refuse), fall back to the scanout card: that copy is
        //    cross-device (and can shear on picky drivers) but it renders rather than
        //    dropping to the single-buffer path that BAD_MATCHes.
        let allocated_linear = allocate_linear_on(render_node, w, h, fourcc).or_else(|e| {
            warn!(
                "untile-blit: LINEAR alloc on render node failed ({e:?}); \
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

/// The scanout card path to blit toward, or `None` when the untiling blit does not
/// apply: the `gpu_untile_blit` experiment is off, no distinct scanout card is
/// configured, or the scanout card is the render card (non-split). The empty-
/// intersection check is done separately by the caller.
/// The distinct scanout card (`primary_scanout` != `primary_render`) — the split
/// signal, independent of `untile_blit`. `None` on a single-card system.
fn distinct_scanout_node() -> Option<String> {
    use compositor_developer_environment_config_router::router;
    let scanout = router::primary_scanout()?.to_string_lossy().into_owned();
    (scanout != router::primary_render_string()).then_some(scanout)
}

/// Whether the primary render node carries the `untile_blit` mode token.
fn untile_blit_enabled() -> bool {
    use compositor_developer_environment_config_mode::mode::ModeFlags;
    compositor_developer_environment_config_router::router::primary_mode()
        .contains(ModeFlags::UNTILE_BLIT)
}
