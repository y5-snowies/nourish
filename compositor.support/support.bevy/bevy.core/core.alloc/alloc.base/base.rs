use std::os::fd::{AsRawFd, OwnedFd};
use std::path::Path;
use compositor_support_bevy_core_fault_base::AllocError;
use compositor_model_debug_instance_record::{fatal, info, warn};
use gbm::{BufferObjectFlags, Device as GbmDevice, Format as GbmFormat};
use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufFlags};
use smithay::backend::allocator::{Buffer, Fourcc, Modifier};

/// Opaque holder for an allocated buffer. Keeps gbm alive while the dmabuf
/// is in use. Drop order inside this struct: `dmabuf` first (releases fds
/// and any imports), then `_bo`, then `_gbm`.
pub struct AllocatedDmabuf {
    pub dmabuf: Dmabuf,
    _bo: gbm::BufferObject<()>,
    _gbm: GbmDevice<OwnedFd>,
}

impl std::fmt::Debug for AllocatedDmabuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AllocatedDmabuf")
            .field("size", &self.dmabuf.size())
            .field("format", &self.dmabuf.format())
            .field("num_planes", &self.dmabuf.num_planes())
            .finish()
    }
}



/// The fourccs a producer's buffer may be allocated with, from the format layer.
///
/// `gbm::Format` IS `DrmFourcc`, so this was always an allow-list wearing an
/// identity function. The list itself lives in `format.universe` — one place, not
/// two byte-identical copies that a fix to one would never reach.
fn gbm_format(fourcc: Fourcc) -> Option<GbmFormat> {
    compositor_kernel_graphic_format_catalog_base::catalog::allocatable(fourcc).then_some(fourcc)
}

/// Explicit-modifier bridge allocation — the ONLY way to allocate here. The BO is
/// allocated with the negotiated list, so `bo.modifier()` is a REAL explicit modifier
/// (never INVALID → no AMD wgpu-import crash).
///
/// An empty `modifiers` list or an unsupported `fourcc` is FATAL. An allocation failure
/// is an ERROR. It used to fall through to the implicit path, which is how an
/// unimportable modifier reached the compositor: the driver chose freely, `vkCreateImage`
/// failed, and the draw path went ahead with a view that had no format features —
/// corrupting windows unrelated to the caller. The implicit entry points are private for
/// the same reason; negotiate, or fail visibly.
///
/// # Why the empty list ends the process
///
/// It is not a property of this buffer, it is a property of the session: the same two
/// format sets are intersected for every surface, so a set that is empty once is empty
/// every time. Returning an error let the caller carry on, and each caller carried on
/// the only way it could — without backing. Under winit that produced 143 identical
/// failures, an iced worker with no surface behind any handle, and a session with a
/// pannable world and no UI at all, which reads as a rendering bug and is not one.
/// Refusing to run beats running blind, and one message beats 143.
pub fn allocate_dmabuf_negotiated(
    render_node: &str,
    width: u32,
    height: u32,
    fourcc: Fourcc,
    modifiers: &[Modifier],
) -> Result<AllocatedDmabuf, AllocError> {
    if modifiers.is_empty() {
        fatal!(
            "no negotiated modifier for {fourcc:?} on {render_node}: the format negotiation \
             produced an EMPTY set, so nothing can be allocated here and every other surface \
             would fail identically. Look for the `bridge:` line above for which side offered \
             nothing — an empty compositor-importable set means the renderer had not published \
             yet, an empty intersection means the two ends share no modifier."
        );
    }
    let Some(gbm_fmt) = gbm_format(fourcc) else {
        fatal!(
            "{fourcc:?} is not a fourcc this bridge can allocate on {render_node}, yet \
             {} modifier(s) were negotiated for it — negotiation and `gbm_format` disagree \
             about the format set, which is a code defect and not a device limitation.",
            modifiers.len()
        );
    };
    allocate_with_modifiers(Path::new(render_node), width, height, fourcc, gbm_fmt, modifiers)
        .inspect_err(|e| warn!("negotiated dmabuf alloc failed for {fourcc:?}: {e:?}"))
}

fn allocate_with_modifiers(
    render_node: &Path,
    width: u32,
    height: u32,
    fourcc: Fourcc,
    gbm_fmt: GbmFormat,
    modifiers: &[Modifier],
) -> Result<AllocatedDmabuf, AllocError> {
    if width == 0 || height == 0 {
        return Err(AllocError::InvalidDimensions { width, height });
    }
    let drm_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(render_node)
        .map_err(AllocError::OpenDrm)?;
    let drm_fd: OwnedFd = drm_file.into();
    let gbm = GbmDevice::new(drm_fd).map_err(AllocError::GbmInit)?;

    let gbm_mods = modifiers.iter().map(|m| gbm::Modifier::from(Into::<u64>::into(*m)));
    let bo = gbm
        .create_buffer_object_with_modifiers2::<()>(width, height, gbm_fmt, gbm_mods, BufferObjectFlags::RENDERING)
        .map_err(AllocError::CreateBo)?;
    compositor_kernel_graphic_format_audit_base::audit::allocation(
        "bevy/iced surface", &render_node.display().to_string(), fourcc, modifiers.len(), bo.modifier(),
    );

    let plane_count = bo.plane_count();
    let modifier = bo.modifier();
    let mut builder = Dmabuf::builder((width as i32, height as i32), fourcc, modifier, DmabufFlags::empty());
    for plane in 0..plane_count {
        let fd = bo.fd_for_plane(plane as i32).map_err(AllocError::ExportFd)?;
        let offset = bo.offset(plane as i32);
        let stride = bo.stride_for_plane(plane as i32);
        builder.add_plane(fd, offset, stride);
    }
    let dmabuf = builder.build().ok_or(AllocError::BuildDmabuf)?;
    publish_stats("gbm-bevy", fourcc, modifier, plane_count);
    Ok(AllocatedDmabuf { dmabuf, _bo: bo, _gbm: gbm })
}

/// Record the post-determined format for the developer "GPU formats" panel.
fn publish_stats(kind: &str, fourcc: Fourcc, modifier: Modifier, plane_count: u32) {
    use compositor_kernel_graphic_format_rule_base::rule;
    compositor_model_stats_registry_gpu::gpu::set_device_format(
        kind,
        &format!("{fourcc:?}"),
        modifier.into(),
        rule::label(rule::classify(modifier)),
        plane_count,
    );
}
