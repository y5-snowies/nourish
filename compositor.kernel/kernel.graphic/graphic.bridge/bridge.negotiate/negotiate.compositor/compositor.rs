//! What the ACTIVE compositor renderer can import, published once at startup.
//!
//! The off-thread producers allocate their own dmabufs on their own threads and
//! hand them to the compositor to sample. They therefore have to negotiate a
//! modifier the compositor's renderer will accept — but they cannot ask it: the
//! renderer lives on the compositor thread behind a `&mut`, and the whole point
//! of the worker is not to touch it.
//!
//! So the kernel publishes the set once, where the renderer is built.
//!
//! # Why not the GLES set
//!
//! The INLINE surfaces intersect `gles ∩ wgpu`, and that is right FOR THEM: they
//! build a `GlesTexture` per slot whatever the compositing renderer is, so GLES
//! importability is a hard requirement of that path. The worker slots build no
//! GLES view at all — the compositor imports their dmabuf natively — so the GLES
//! set is not a constraint on them, and using it would both over-constrain
//! (rejecting modifiers Vulkan takes and GLES does not) and under-constrain
//! (accepting ones the compositing renderer never claimed).
//!
//! The right set for a worker is: what the renderer that will SAMPLE the buffer
//! can import, intersected with what the wgpu device that will WRITE it can.

use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Fourcc, Modifier};
use smithay::backend::drm::{DrmNode, NodeType};
use std::sync::{OnceLock, RwLock};

/// Which GPU the compositor composites on: `render_node`, else `fallback` (the
/// scanout device) when it names nothing usable.
///
/// ONE definition, because THREE places have to agree on it and there is no
/// error if they do not. The compositor ADVERTISES a device to clients, it
/// VALIDATES the buffers they send, and it DRAWS them. Let those disagree and a
/// client allocates on one GPU, passes validation on a second and fails to
/// import on a third — which reaches the user as blank windows, not as anything
/// anyone can act on. That is exactly what happened when only the draw step was
/// moved to `render_node`.
pub fn composite_node(fallback: DrmNode) -> DrmNode {
    let path = compositor_model_environment_config_base::base::get().render_node.clone();
    DrmNode::from_path(&path)
        .ok()
        .map(|n| n.node_with_type(NodeType::Render).and_then(|r| r.ok()).unwrap_or(n))
        .unwrap_or(fallback)
}

fn slot() -> &'static RwLock<FormatSet> {
    static SLOT: OnceLock<RwLock<FormatSet>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(FormatSet::default()))
}

/// Kernel: record the active renderer's `ImportDma::dmabuf_formats()`. Called
/// once, after any renderer fallback has been resolved.
pub fn set_compositor_importable(set: FormatSet) {
    *slot().write().unwrap_or_else(|e| e.into_inner()) = set;
}

pub fn compositor_importable() -> FormatSet {
    slot().read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The modifier list a worker may allocate `fourcc` with: the compositor's
/// importable set intersected with the producer's own wgpu device's.
///
/// An empty result means the implicit path, exactly as elsewhere in the bridge —
/// which is also what happens before the compositor has published, so a producer
/// starting early degrades to today's behaviour rather than failing.
pub fn worker_modifiers(wgpu_importable: FormatSet, fourcc: Fourcc) -> Vec<Modifier> {
    compositor_kernel_graphic_bridge_negotiate_base::negotiate::bridge_modifiers(
        compositor_importable(),
        wgpu_importable,
        fourcc,
    )
}

fn scanout() -> &'static RwLock<Option<Fourcc>> {
    static SLOT: OnceLock<RwLock<Option<Fourcc>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

/// Kernel: the fourcc the swapchain ACTUALLY got, once smithay has chosen from
/// the offered ladder.
///
/// The achieved format, not the requested depth, is the honest answer to "is
/// this a 10-bit session": it is only 10-bit if deep colour was asked for AND
/// the plane accepted it AND a mode was found. One signal that already folds in
/// every condition a producer would otherwise have to re-derive.
pub fn set_scanout_fourcc(fourcc: Fourcc) {
    *scanout().write().unwrap_or_else(|e| e.into_inner()) = Some(fourcc);
}

/// Whether the compositor is scanning out more than 8 bits per channel, so a
/// producer can match rather than render 8-bit into a 10-bit pipeline.
pub fn scanout_is_deep() -> bool {
    matches!(
        *scanout().read().unwrap_or_else(|e| e.into_inner()),
        Some(Fourcc::Xrgb2101010 | Fourcc::Argb2101010 | Fourcc::Xbgr2101010 | Fourcc::Abgr2101010)
    )
}
