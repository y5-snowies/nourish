//! `CaptureRegistry`: the continuous-capture engine.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use smithay::backend::renderer::Bind;
use smithay::backend::renderer::Blit;
use smithay::backend::renderer::TextureFilter;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTarget, GlesTexture};
use smithay::utils::{Physical, Rectangle, Size};
use compositor_support_bevy_core_runtime_base::{
    WgpuVulkanContext, allocate_dmabuf_negotiated, import_dmabuf_to_gles, import_dmabuf_to_wgpu,
};

use crate::entry::{CaptureEntry, EntryId};
use crate::error::CaptureError;
use crate::handle::{CaptureHandle, HandleInner};
use crate::source::{CaptureSource, OutputId};

pub struct CaptureRegistry {
    inner: Arc<Mutex<RegistryInner>>,
    wgpu_ctx: Arc<WgpuVulkanContext>,
}

#[derive(Debug)]
pub enum BlitErr<B, R> {
    Bind(B),
    Blit(R),
}

pub(crate) struct RegistryInner {
    pub entries: HashMap<EntryId, CaptureEntry>,
    next_entry_id: u64,
    pub wgpu_ctx: Arc<WgpuVulkanContext>,
    /// Per-output current size, tracked across ticks.
    output_sizes: HashMap<OutputId, Size<i32, Physical>>,
}

impl CaptureRegistry {
    pub fn new(wgpu_ctx: Arc<WgpuVulkanContext>) -> Self {
        let inner = RegistryInner {
            entries: HashMap::new(),
            next_entry_id: 1,
            wgpu_ctx: wgpu_ctx.clone(),
            output_sizes: HashMap::new(),
        };
        Self {
            inner: Arc::new(Mutex::new(inner)),
            wgpu_ctx,
        }
    }
    // Add to impl CaptureRegistry:
    /// All capture entries reading from `output`, as
    /// `(entry_id, gles_texture, dst_size, src_override)`. `src_override` is
    /// `Some(rect)` for region captures (the sub-rect of the framebuffer to
    /// copy) and `None` for full-output captures (the caller blits the whole
    /// framebuffer). `dst_size` is always the entry/dmabuf size.
    pub fn entries_for_output(
        &self,
        output: OutputId,
    ) -> Vec<(
        EntryId,
        GlesTexture,
        Size<i32, Physical>,
        Option<Rectangle<i32, Physical>>,
    )> {
        let inner = self.inner.lock().unwrap();
        inner
            .entries
            .iter()
            .filter(|(_, e)| e.source.output() == output)
            .map(|(id, e)| (*id, e.gles_tex.clone(), e.size, e.source.src_rect()))
            .collect()
    }

    /// Like [`entries_for_output`] but hands out each entry's underlying
    /// `Dmabuf` instead of its GLES texture. The native Vulkan capture path
    /// imports these as transfer targets and blits the composed scene into them
    /// directly (the GLES tap blits via `gles_tex`; Vulkan can't, so it copies
    /// into the shared dmabuf the wgpu side also reads).
    pub fn entry_dmabufs_for_output(
        &self,
        output: OutputId,
    ) -> Vec<(
        EntryId,
        smithay::backend::allocator::dmabuf::Dmabuf,
        Size<i32, Physical>,
        Option<Rectangle<i32, Physical>>,
    )> {
        let inner = self.inner.lock().unwrap();
        inner
            .entries
            .iter()
            .filter(|(_, e)| e.source.output() == output)
            .map(|(id, e)| (*id, e.dmabuf.dmabuf.clone(), e.size, e.source.src_rect()))
            .collect()
    }

    /// The `Dmabuf` for a specific entry (cloned handle — shares the buffer).
    /// Used by the per-element capture path to bind the entry as a render
    /// target. `None` if the entry is gone.
    pub fn entry_dmabuf(
        &self,
        entry_id: EntryId,
    ) -> Option<smithay::backend::allocator::dmabuf::Dmabuf> {
        let inner = self.inner.lock().unwrap();
        inner.entries.get(&entry_id).map(|e| e.dmabuf.dmabuf.clone())
    }

    /// Update the sub-rect of a region capture entry. A move that preserves
    /// the size only changes the blit src (cheap); a size change reallocates
    /// the entry's dmabuf in place (the `EntryId`/handle stays stable). No-op
    /// if the entry is gone or is not a region capture.
    pub fn set_region(
        &self,
        render_node: &str,
        gles: &mut GlesRenderer,
        entry_id: EntryId,
        rect: Rectangle<i32, Physical>,
    ) -> Result<(), CaptureError> {
        if rect.size.w <= 0 || rect.size.h <= 0 {
            return Err(CaptureError::InvalidSize {
                w: rect.size.w,
                h: rect.size.h,
            });
        }
        let mut inner = self.inner.lock().unwrap();
        let Some(entry) = inner.entries.get_mut(&entry_id) else {
            return Ok(());
        };
        let CaptureSource::Region { output, rect: cur } = entry.source else {
            return Ok(());
        };
        if cur == rect {
            return Ok(());
        }
        let resized = cur.size != rect.size;
        entry.source = CaptureSource::Region { output, rect };
        if resized {
            inner.reallocate_entry(render_node, gles, entry_id, rect.size)?;
        }
        Ok(())
    }
    /// Request a continuous capture.
    ///
    /// If an existing entry matches `(source, size)`, returns a clone
    /// sharing it. Otherwise allocates a new entry.
    ///
    /// Requires that `tick` or `set_output_size` has been called at least
    /// once for the source's output, so the registry knows what size to
    /// allocate.
    pub fn request(
        &self,
        render_node: &str,
        gles: &mut GlesRenderer,
        source: CaptureSource,
    ) -> Result<CaptureHandle, CaptureError> {
        let mut inner = self.inner.lock().unwrap();
        let size = inner.size_for_source(source)?;

        if let Some(entry_id) = inner.find_match(source, size) {
            inner.entries.get_mut(&entry_id).unwrap().refcount += 1;
            return Ok(make_handle(&self.inner, entry_id));
        }

        let entry_id = inner.alloc_entry(render_node, gles, source, size)?;
        inner.entries.get_mut(&entry_id).unwrap().refcount += 1;
        Ok(make_handle(&self.inner, entry_id))
    }

    /// Set the current size for an output. If size changed, all entries
    /// for that output are reallocated (in place; `entry_id` stays stable).
    pub fn set_output_size(
        &self,
        render_node: &str,
        gles: &mut GlesRenderer,
        output: OutputId,
        new_size: Size<i32, Physical>,
    ) -> Result<(), CaptureError> {
        if new_size.w <= 0 || new_size.h <= 0 {
            return Err(CaptureError::InvalidSize {
                w: new_size.w,
                h: new_size.h,
            });
        }
        let mut inner = self.inner.lock().unwrap();
        let prev = inner.output_sizes.insert(output, new_size);
        if prev == Some(new_size) {
            return Ok(());
        }
        let affected: Vec<EntryId> = inner
            .entries
            .iter()
            .filter(|(_, e)| matches!(e.source, CaptureSource::OutputFramebuffer(o) if o == output))
            .map(|(id, _)| *id)
            .collect();
        for id in affected {
            inner.reallocate_entry(render_node, gles, id, new_size)?;
        }
        Ok(())
    }

    /// Tick: blit the framebuffer into every active capture entry for this
    /// output.
    ///
    /// All active entries are unconditionally blitted into each frame. If
    /// no entries exist for this output, this is a no-op.
    // pub fn tick(
    //     &self,
    //     gles: &mut GlesRenderer,
    //     output: OutputId,
    //     framebuffer: &GlesTarget<'_>,
    //     framebuffer_size: Size<i32, Physical>,
    // ) {
    //     if let Err(e) = self.set_output_size(gles, output, framebuffer_size) {
    //         warn!("set_output_size failed: {}", e);
    //         return;
    //     }

    //     let mut inner = self.inner.lock().unwrap();
    //     let to_blit: Vec<EntryId> = inner
    //         .entries
    //         .iter()
    //         .filter(|(_, e)| matches!(e.source, CaptureSource::OutputFramebuffer(o) if o == output))
    //         .map(|(id, _)| *id)
    //         .collect();

    //     for entry_id in to_blit {
    //         let mut entry_tex = inner.entries.get(&entry_id).unwrap().gles_tex.clone();
    //         let entry_size = inner.entries.get(&entry_id).unwrap().size;
    //         let src = Rectangle::<i32, Physical>::from_loc_and_size((0, 0), framebuffer_size);
    //         let dst = Rectangle::<i32, Physical>::from_loc_and_size((0, 0), entry_size);

    //         drop(inner);
    //         let result: Result<(), smithay::backend::renderer::gles::GlesError> = (|| {
    //             let mut target: GlesTarget = gles.bind(&mut entry_tex)?;
    //             gles.blit(framebuffer, &mut target, src, dst, TextureFilter::Linear)?;
    //             Ok(())
    //         })();
    //         inner = self.inner.lock().unwrap();

    //         if let Err(e) = result {
    //             warn!(entry_id = ?entry_id, error = ?e, "tick blit failed");
    //         }
    //     }
    // }

    /// Tick: blit the framebuffer into every active capture entry for
    /// this output. This is the winit-style convenience: pass in a
    /// `GlesTarget` you already have bound to a framebuffer, and the
    /// registry copies from it.
    ///
    /// For DRM-backed sources where the framebuffer isn't directly
    /// accessible, use `tick_with` and supply a closure.
    // CHECK: keep local refernce to str or pass a String reference.
    pub fn tick(
        &self,
        render_node: &str,
        gles: &mut GlesRenderer,
        output: OutputId,
        framebuffer: &GlesTarget<'_>,
        framebuffer_size: Size<i32, Physical>,
    ) {
        if let Err(e) = self.set_output_size(render_node, gles, output, framebuffer_size) {
            warn!("set_output_size failed: {}", e);
            return;
        }

        let mut inner = self.inner.lock().unwrap();
        let to_blit: Vec<EntryId> = inner
            .entries
            .iter()
            .filter(|(_, e)| e.source.output() == output)
            .map(|(id, _)| *id)
            .collect();

        for entry_id in to_blit {
            let mut entry_tex = inner.entries.get(&entry_id).unwrap().gles_tex.clone();
            let entry_size = inner.entries.get(&entry_id).unwrap().size;
            let src_override = inner.entries.get(&entry_id).unwrap().source.src_rect();
            // Region captures copy their sub-rect of the framebuffer; full
            // captures copy the whole framebuffer.
            let src = src_override
                .unwrap_or_else(|| Rectangle::<i32, Physical>::from_loc_and_size((0, 0), framebuffer_size));
            let dst = Rectangle::<i32, Physical>::from_loc_and_size((0, 0), entry_size);

            drop(inner);
            let result: Result<(), smithay::backend::renderer::gles::GlesError> = (|| {
                let mut target: GlesTarget = gles.bind(&mut entry_tex)?;
                gles.blit(framebuffer, &mut target, src, dst, TextureFilter::Linear)?;
                Ok(())
            })();
            inner = self.inner.lock().unwrap();

            if let Err(e) = result {
                warn!("tick blit failed: entry_id={entry_id:?} error={e:?}");
            }
        }
        drop(inner);
    }

    /// Lower-level tick that delegates the actual pixel-copy to a
    /// caller-provided closure. Used when the capture source isn't a
    /// directly-accessible `GlesTarget` — e.g. when the source pixels
    /// live in a `DrmCompositor::RenderFrameResult` and the blit goes
    /// through `result.blit_frontbuffer_to(...)`.
    ///
    /// The closure is invoked once per active entry. It receives the
    /// renderer, the destination `GlesTarget` (already bound to the
    /// entry's texture), and the source/destination rectangles, and
    /// must perform the blit (or any equivalent copy) returning
    /// `Result<(), GlesError>`.
    ///
    /// `tick` (the framebuffer-based variant above) is now a thin
    /// wrapper around this — they share identical entry iteration,
    /// size handling, and locking semantics.
    pub fn tick_with<F, E>(
        &self,
        output: OutputId,
        framebuffer_size: Size<i32, Physical>,
        mut blit: F,
    ) where
        F: FnMut(
            &mut GlesTexture,
            Rectangle<i32, Physical>,
            Rectangle<i32, Physical>,
        ) -> Result<(), E>,
        E: std::fmt::Debug,
    {
        // Note: caller is responsible for invoking set_output_size before
        // tick_with if the framebuffer size may have changed. set_output_size
        // still requires a GlesRenderer for reallocation.

        let mut inner = self.inner.lock().unwrap();
        let to_blit: Vec<EntryId> = inner
            .entries
            .iter()
            .filter(|(_, e)| e.source.output() == output)
            .map(|(id, _)| *id)
            .collect();

        for entry_id in to_blit {
            let mut entry_tex = inner.entries.get(&entry_id).unwrap().gles_tex.clone();
            let entry_size = inner.entries.get(&entry_id).unwrap().size;
            let src_override = inner.entries.get(&entry_id).unwrap().source.src_rect();
            let src = src_override
                .unwrap_or_else(|| Rectangle::<i32, Physical>::from_loc_and_size((0, 0), framebuffer_size));
            let dst = Rectangle::<i32, Physical>::from_loc_and_size((0, 0), entry_size);

            drop(inner);
            if let Err(e) = blit(&mut entry_tex, src, dst) {
                warn!("tick_with blit failed: entry_id={entry_id:?} error={e:?}");
            }
            inner = self.inner.lock().unwrap();
        }
        drop(inner);
    }

    pub fn wgpu_ctx(&self) -> &Arc<WgpuVulkanContext> {
        &self.wgpu_ctx
    }
}

fn make_handle(inner: &Arc<Mutex<RegistryInner>>, entry_id: EntryId) -> CaptureHandle {
    CaptureHandle {
        inner: Arc::new(HandleInner {
            registry: Arc::downgrade(inner),
            entry_id,
            detached: std::sync::atomic::AtomicBool::new(false),
        }),
    }
}

impl RegistryInner {
    fn size_for_source(&self, source: CaptureSource) -> Result<Size<i32, Physical>, CaptureError> {
        match source {
            CaptureSource::OutputFramebuffer(output) => self
                .output_sizes
                .get(&output)
                .copied()
                .ok_or(CaptureError::InvalidSize { w: 0, h: 0 }),
            // A region's dmabuf is sized to the region rect, independent of
            // the output size.
            CaptureSource::Region { rect, .. } => Ok(rect.size),
        }
    }

    fn find_match(&self, source: CaptureSource, size: Size<i32, Physical>) -> Option<EntryId> {
        self.entries
            .iter()
            .find(|(_, e)| e.source == source && e.size == size)
            .map(|(id, _)| *id)
    }

    fn alloc_entry(
        &mut self,
        render_node: &str,
        gles: &mut GlesRenderer,
        source: CaptureSource,
        size: Size<i32, Physical>,
    ) -> Result<EntryId, CaptureError> {
        if size.w <= 0 || size.h <= 0 {
            return Err(CaptureError::InvalidSize {
                w: size.w,
                h: size.h,
            });
        }

        let fourcc = smithay::backend::allocator::Fourcc::Argb8888;
        let mods = compositor_kernel_graphic_bridge_negotiate_base::negotiate::bridge_modifiers(
            smithay::backend::renderer::ImportDma::dmabuf_formats(gles),
            self.wgpu_ctx.importable.clone(),
            fourcc,
        );
        let dmabuf =
            allocate_dmabuf_negotiated(render_node, size.w as u32, size.h as u32, fourcc, &mods)?;
        let gles_tex = import_dmabuf_to_gles(gles, &dmabuf.dmabuf)?;
        let wgpu_tex = import_dmabuf_to_wgpu(&self.wgpu_ctx, &dmabuf.dmabuf)?;

        let id = EntryId(self.next_entry_id);
        self.next_entry_id += 1;
        trace!("allocated capture entry: entry_id={id:?} source={source:?} size={size:?}");

        self.entries.insert(
            id,
            CaptureEntry {
                id,
                source,
                size,
                dmabuf,
                gles_tex,
                wgpu_tex: Arc::new(wgpu_tex),
                refcount: 0,
            },
        );
        Ok(id)
    }

    fn reallocate_entry(
        &mut self,
        render_node: &str,
        gles: &mut GlesRenderer,
        entry_id: EntryId,
        new_size: Size<i32, Physical>,
    ) -> Result<(), CaptureError> {
        let Some(entry) = self.entries.get(&entry_id) else {
            return Ok(());
        };
        if entry.size == new_size {
            return Ok(());
        }
        let source = entry.source;
        let refcount = entry.refcount;

        let fourcc = smithay::backend::allocator::Fourcc::Argb8888;
        let mods = compositor_kernel_graphic_bridge_negotiate_base::negotiate::bridge_modifiers(
            smithay::backend::renderer::ImportDma::dmabuf_formats(gles),
            self.wgpu_ctx.importable.clone(),
            fourcc,
        );
        let dmabuf = allocate_dmabuf_negotiated(
            render_node,
            new_size.w as u32,
            new_size.h as u32,
            fourcc,
            &mods,
        )?;
        let gles_tex = import_dmabuf_to_gles(gles, &dmabuf.dmabuf)?;
        let wgpu_tex = import_dmabuf_to_wgpu(&self.wgpu_ctx, &dmabuf.dmabuf)?;

        self.entries.insert(
            entry_id,
            CaptureEntry {
                id: entry_id,
                source,
                size: new_size,
                dmabuf,
                gles_tex,
                wgpu_tex: Arc::new(wgpu_tex),
                refcount,
            },
        );
        trace!("reallocated capture entry: entry_id={entry_id:?} new_size={new_size:?}");
        Ok(())
    }

    pub(crate) fn decref(&mut self, entry_id: EntryId) {
        let Some(entry) = self.entries.get_mut(&entry_id) else {
            return;
        };
        if entry.refcount == 0 {
            warn!("decref on entry with refcount 0: entry_id={entry_id:?}");
            return;
        }
        entry.refcount -= 1;
        if entry.refcount == 0 {
            trace!("last reference dropped — freeing: entry_id={entry_id:?}");
            self.entries.remove(&entry_id);
        }
    }
}
