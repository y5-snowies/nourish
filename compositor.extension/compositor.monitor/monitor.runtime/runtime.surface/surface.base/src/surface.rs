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
    /// Sampleable view used by smithay's GlesRenderer to composite the UI.
    pub gles_texture: GlesTexture,
    /// Render-attachment view used by iced_wgpu to draw the UI.
    pub wgpu_texture: wgpu::Texture,
    /// The underlying allocation. Keeps gbm + BO alive.
    pub allocated: AllocatedDmabuf,
    /// The logical size of the surface. Equals the texture extent today;
    /// kept as its own field so resize-with-oversize-allocation can diverge
    /// it from texture dims later without changing this field's meaning.
    pub size: Size<i32, Physical>,
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
        // Step 1: Clear to color.
        let view = self.create_render_view();
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
                texture: &self.wgpu_texture,
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
    /// wgpu texture and a GLES texture.
    pub fn allocate(
        render_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Result<Self, SurfaceError> {
        info!("IcedSurface::allocate {}x{}", size.w, size.h);

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
            size,
        })
    }

    /// Resize. This is a destroy-and-recreate, in drop-safe order.
    ///
    /// The old views drop first (gles, then wgpu, then allocation), then a
    /// fresh surface is allocated. If allocation fails, `*self` is replaced
    /// with the error path's leftovers — caller should treat the surface as
    /// invalid and destroy the instance.
    ///
    /// This is synchronous; the caller is responsible for batching/debouncing
    /// (see `IcedRegistry`'s pending-resize queue in crate 3).
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
            "IcedSurface::resize {}x{} -> {}x{}",
            self.size.w, self.size.h, new_size.w, new_size.h
        );

        // Allocate a replacement first. If this fails, *self is unchanged
        // and the caller sees a clean error.
        let replacement = IcedSurface::allocate(render_node, wgpu_ctx, gles, new_size)?;

        // Now swap. The old fields drop in declaration order (gles, wgpu,
        // allocated) when `_old` goes out of scope at the end of the block.
        let _old = std::mem::replace(self, replacement);
        Ok(())
    }

    /// Convenience: produce a `wgpu::TextureView` for use as a render attachment.
    pub fn create_render_view(&self) -> wgpu::TextureView {
        self.wgpu_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("y5_iced_dmabuf_render_view"),
            ..Default::default()
        })
    }
}
