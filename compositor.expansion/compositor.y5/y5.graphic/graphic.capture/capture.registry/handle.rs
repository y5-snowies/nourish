//! `CaptureHandle`: live continuous-capture reference.

use std::sync::{Arc, Mutex, Weak};

use smithay::backend::renderer::gles::GlesRenderer;

use crate::entry::{EntryId, SnapshotData};
use crate::error::CaptureError;
use crate::registry::RegistryInner;
use crate::snapshot::SnapshotHandle;

/// Reference to a continuous capture managed by the registry.
///
/// Cloneable for shared live-feed consumption. Drop decrements the entry's
/// refcount; when it reaches zero, the registry frees the entry.
#[derive(Clone)]
pub struct CaptureHandle {
    pub(crate) inner: Arc<HandleInner>,
}

pub(crate) struct HandleInner {
    pub registry: Weak<Mutex<RegistryInner>>,
    pub entry_id: EntryId,
    /// If true, this handle's underlying entry has already been extracted
    /// from the registry via `take`. Drop must not try to decref.
    pub detached: std::sync::atomic::AtomicBool,
}

impl CaptureHandle {
    /// Returns the entry id this handle references. Stable across the
    /// handle's lifetime — resize replaces the entry's *contents*, not its id.
    pub fn entry_id(&self) -> EntryId {
        self.inner.entry_id
    }

    /// Returns the current wgpu texture for this entry. Cheap Arc clone.
    /// On resize, the texture identity changes (different Arc); compare
    /// across frames to detect.
    pub fn wgpu_texture(&self) -> Option<Arc<wgpu::Texture>> {
        let reg = self.inner.registry.upgrade()?;
        let reg = reg.lock().ok()?;
        reg.entries
            .get(&self.inner.entry_id)
            .map(|e| e.wgpu_tex.clone())
    }

    /// Returns the dmabuf for this entry.
    pub fn dmabuf(&self) -> Option<smithay::backend::allocator::dmabuf::Dmabuf> {
        let reg = self.inner.registry.upgrade()?;
        let reg = reg.lock().ok()?;
        reg.entries
            .get(&self.inner.entry_id)
            .map(|e| e.dmabuf.dmabuf.clone())
    }

    /// Returns the current size of the entry.
    pub fn size(&self) -> Option<smithay::utils::Size<i32, smithay::utils::Physical>> {
        let reg = self.inner.registry.upgrade()?;
        let reg = reg.lock().ok()?;
        reg.entries.get(&self.inner.entry_id).map(|e| e.size)
    }

    /// Take a snapshot, consuming this handle.
    ///
    /// **Sole-owner path (zero-copy)**: if this handle is the only
    /// reference to its entry (no other clones exist), the entry is removed
    /// from the registry and its dmabuf transferred directly into the
    /// returned `SnapshotHandle`. The registry stops blitting (no entry).
    ///
    /// **Shared path (copy)**: if other clones exist, allocates a fresh
    /// dmabuf, blits the current entry's contents into it, and returns
    /// that as a `SnapshotHandle`. The other clones keep their live stream
    /// — they're unaffected.
    pub fn take(
        self,
        render_node: &str,
        gles: &mut GlesRenderer,
    ) -> Result<SnapshotHandle, CaptureError> {
        let registry = self
            .inner
            .registry
            .upgrade()
            .ok_or(CaptureError::RegistryDropped)?;

        // First, determine sole-owner status under lock.
        let (is_sole_owner, source, size) = {
            let mut reg = registry.lock().unwrap();
            let entry = reg
                .entries
                .get(&self.inner.entry_id)
                .ok_or(CaptureError::RegistryDropped)?;
            (entry.refcount == 1, entry.source, entry.size)
        };

        if is_sole_owner {
            // Zero-copy: pull the entry out of the registry.
            let mut reg = registry.lock().unwrap();
            let entry = reg
                .entries
                .remove(&self.inner.entry_id)
                .ok_or(CaptureError::RegistryDropped)?;
            // Prevent our Drop from trying to decref a removed entry.
            self.inner
                .detached
                .store(true, std::sync::atomic::Ordering::Relaxed);
            trace!(
                "zero-copy take: sole owner, entry detached from registry: entry_id={:?}",
                self.inner.entry_id
            );
            Ok(SnapshotHandle::from_entry(SnapshotData::from(entry)))
        } else {
            // Shared: allocate fresh, blit current → fresh.
            let snapshot = make_copy_snapshot(
                render_node,
                &registry,
                gles,
                self.inner.entry_id,
                source,
                size,
            )?;
            // Drop our handle reference; the entry continues to exist for
            // the other clones.
            // (Drop happens automatically when `self` goes out of scope at
            // return. The Drop impl on HandleInner runs once the Arc count
            // hits zero — which it will when this handle's clones are also
            // gone. Since we're the consumed self and no clone exists from
            // this consumption, our HandleInner is being dropped right now,
            // which will decref properly.)
            trace!(
                "copy take: shared entry, snapshot allocated independently: entry_id={:?}",
                self.inner.entry_id
            );
            Ok(snapshot)
        }
    }

    /// Take a snapshot WITHOUT consuming this handle. Always allocates a
    /// fresh dmabuf and copies, even if sole owner. The live stream keeps
    /// running for this handle.
    pub fn snapshot(
        &self,
        render_node: &str,
        gles: &mut GlesRenderer,
    ) -> Result<SnapshotHandle, CaptureError> {
        let registry = self
            .inner
            .registry
            .upgrade()
            .ok_or(CaptureError::RegistryDropped)?;

        let (source, size) = {
            let reg = registry.lock().unwrap();
            let entry = reg
                .entries
                .get(&self.inner.entry_id)
                .ok_or(CaptureError::RegistryDropped)?;
            (entry.source, entry.size)
        };

        make_copy_snapshot(
            render_node,
            &registry,
            gles,
            self.inner.entry_id,
            source,
            size,
        )
    }
}

impl Drop for HandleInner {
    fn drop(&mut self) {
        if self.detached.load(std::sync::atomic::Ordering::Relaxed) {
            // The entry was extracted by `take`. Nothing to decref.
            return;
        }
        if let Some(reg) = self.registry.upgrade() {
            if let Ok(mut reg) = reg.lock() {
                reg.decref(self.entry_id);
            }
        }
    }
}

impl std::fmt::Debug for CaptureHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureHandle")
            .field("entry_id", &self.inner.entry_id)
            .finish()
    }
}

/// Allocate a fresh dmabuf and copy a given entry's contents into it.
///
/// The copy runs on the SAME wgpu (Vulkan) device the capture wrote the entry
/// with, via `copy_texture_to_texture`. A GLES copy here (the old path) wrote
/// the dmabuf through GL, which the Vulkan sampler in bevy could not see — so
/// the morph/picker snapshot sampled black. Keeping it on wgpu makes the frozen
/// frame visible to bevy.
///
/// Returns a `SnapshotHandle` owning the fresh dmabuf.
fn make_copy_snapshot(
    render_node: &str,
    registry: &Arc<Mutex<RegistryInner>>,
    gles: &mut GlesRenderer,
    src_entry_id: EntryId,
    _source: crate::source::CaptureSource,
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
) -> Result<SnapshotHandle, CaptureError> {
    // Pull source wgpu texture + the shared wgpu context out under lock.
    let (src_wgpu, wgpu_ctx) = {
        let reg = registry.lock().unwrap();
        let entry = reg
            .entries
            .get(&src_entry_id)
            .ok_or(CaptureError::RegistryDropped)?;
        (entry.wgpu_tex.clone(), reg.wgpu_ctx.clone())
    };

    // Allocate the fresh dmabuf + imports. The GLES import is kept only so the
    // SnapshotData stays well-formed; bevy samples the wgpu texture.
    let fourcc = smithay::backend::allocator::Fourcc::Argb8888;
    let mods = compositor_kernel_graphic_bridge_negotiate_base::negotiate::bridge_modifiers(
        smithay::backend::renderer::ImportDma::dmabuf_formats(gles),
        wgpu_ctx.importable.clone(),
        fourcc,
    );
    let dmabuf = compositor_support_bevy_core_runtime_base::allocate_dmabuf_negotiated(
        render_node,
        size.w as u32,
        size.h as u32,
        fourcc,
        &mods,
    )?;
    let gles_tex = compositor_support_bevy_core_runtime_base::import_dmabuf_to_gles(gles, &dmabuf.dmabuf)?;
    let wgpu_tex = compositor_support_bevy_core_runtime_base::import_dmabuf_to_wgpu(&wgpu_ctx, &dmabuf.dmabuf)?;

    // Vulkan copy: src entry (COPY_SRC) → fresh (COPY_DST), same device.
    let mut encoder = wgpu_ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("y5-snapshot-copy") });
    encoder.copy_texture_to_texture(
        src_wgpu.as_image_copy(),
        wgpu_tex.as_image_copy(),
        wgpu::Extent3d {
            width: size.w as u32,
            height: size.h as u32,
            depth_or_array_layers: 1,
        },
    );
    wgpu_ctx.queue.submit(Some(encoder.finish()));
    let _ = wgpu_ctx.device.poll(wgpu::PollType::wait_indefinitely());

    Ok(SnapshotHandle::from_entry(SnapshotData {
        size,
        dmabuf,
        gles_tex,
        wgpu_tex: Arc::new(wgpu_tex),
    }))
}
