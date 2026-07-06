//! `IcedSurface`: one DMABUF, imported as both a `wgpu::Texture` and a
//! `GlesTexture`.
//!
//! This is the unit the engine renders into and the compositor samples from.
//! Each Iced instance owns one. Allocation, import, and resize are managed
//! here so callers never have to think about drop ordering across the three
//! views.

use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::utils::{Physical, Size};

use crate::dmabuf_alloc::{AllocatedDmabuf, allocate_dmabuf_negotiated};
use crate::error::SurfaceError;
use crate::gles_import::import_dmabuf_to_gles;
use crate::wgpu_context::WgpuVulkanContext;
use crate::wgpu_import::{TEXTURE_FORMAT, import_dmabuf_to_wgpu};

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

/// The three views of one dmabuf, grouped so field-drop order is guaranteed:
/// `gles_texture` (EGLImage) → `wgpu_texture` (Vulkan external-mem binding) →
/// `allocated` (owns the BO). See the type-level note on `IcedSurface`.
struct Backing {
    gles_texture: GlesTexture,
    wgpu_texture: wgpu::Texture,
    allocated: AllocatedDmabuf,
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
        // Step 1: Clear to color.
        let view = backing.wgpu_texture.create_view(&wgpu::TextureViewDescriptor {
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
                texture: &backing.wgpu_texture,
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

    /// Sampleable GLES view, or `None` while released.
    pub fn gles_texture(&self) -> Option<&GlesTexture> {
        self.backing.as_ref().map(|b| &b.gles_texture)
    }

    /// The underlying dmabuf, or `None` while released.
    pub fn dmabuf(&self) -> Option<&smithay::backend::allocator::dmabuf::Dmabuf> {
        self.backing.as_ref().map(|b| &b.allocated.dmabuf)
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
            b.wgpu_texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("y5_iced_dmabuf_render_view"),
                ..Default::default()
            })
        })
    }
}

impl Backing {
    fn allocate(
        render_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Result<Self, SurfaceError> {
        trace!("IcedSurface backing allocate {}x{}", size.w, size.h);

        // Negotiate an explicit modifier across gles ∩ wgpu (empty ⇒ implicit path).
        let fourcc = smithay::backend::allocator::Fourcc::Argb8888;
        let mods = compositor_kernel_graphic_bridge_negotiate_base::negotiate::bridge_modifiers(
            smithay::backend::renderer::ImportDma::dmabuf_formats(gles),
            wgpu_ctx.importable.clone(),
            fourcc,
        );
        let allocated =
            allocate_dmabuf_negotiated(render_node, size.w as u32, size.h as u32, fourcc, &mods)?;
        let gles_texture = import_dmabuf_to_gles(gles, &allocated.dmabuf)?;
        let wgpu_texture = import_dmabuf_to_wgpu(wgpu_ctx, &allocated.dmabuf)?;

        Ok(Self {
            gles_texture,
            wgpu_texture,
            allocated,
        })
    }
}
