//! DRM render node → gbm BO → Dmabuf.

use std::os::fd::{AsRawFd, OwnedFd};
use std::path::Path;

use compositor_support_bevy_core_fault_base::AllocError;
use compositor_developer_debug_instance_record::{info, warn};
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

/// Allocate a single ARGB8888 LINEAR dmabuf at the given size from the
/// default render node.
pub fn allocate_dmabuf(
    render_node: &str,
    width: u32,
    height: u32,
) -> Result<AllocatedDmabuf, AllocError> {
    allocate_dmabuf_on(Path::new(render_node), width, height)
}

/// Variant with an explicit render node path.
pub fn allocate_dmabuf_on(
    render_node: &Path,
    width: u32,
    height: u32,
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

    info!("Opened DRM render node {} (fd={})", render_node.display(), drm_fd.as_raw_fd());

    let gbm = GbmDevice::new(drm_fd).map_err(AllocError::GbmInit)?;

    let bo = gbm
        .create_buffer_object::<()>(
            width,
            height,
            GbmFormat::Argb8888,
            BufferObjectFlags::RENDERING,
        )
        .map_err(AllocError::CreateBo)?;

    info!("Allocated gbm BO {}x{}, format=ARGB8888, modifier={:?}", width, height, bo.modifier());

    let plane_count = bo.plane_count();
    let modifier = bo.modifier();

    let mut builder = Dmabuf::builder(
        (width as i32, height as i32),
        Fourcc::Argb8888,
        modifier,
        DmabufFlags::empty(),
    );

    for plane in 0..plane_count {
        let fd = bo
            .fd_for_plane(plane as i32)
            .map_err(AllocError::ExportFd)?;
        let offset = bo.offset(plane as i32);
        let stride = bo.stride_for_plane(plane as i32);

        builder.add_plane(fd, plane as u32, offset, stride);
    }

    let dmabuf = builder.build().ok_or(AllocError::BuildDmabuf)?;

    info!("Built Dmabuf: size={:?}, format={:?}, num_planes={}, modifier={:?}", dmabuf.size(), dmabuf.format(), dmabuf.num_planes(), modifier);

    Ok(AllocatedDmabuf { dmabuf, _bo: bo, _gbm: gbm })
}

/// Map a bridge fourcc to its gbm format (the fourccs the bridge negotiates).
fn gbm_format(fourcc: Fourcc) -> Option<GbmFormat> {
    Some(match fourcc {
        Fourcc::Argb8888 => GbmFormat::Argb8888,
        Fourcc::Xrgb8888 => GbmFormat::Xrgb8888,
        Fourcc::Abgr8888 => GbmFormat::Abgr8888,
        Fourcc::Xbgr8888 => GbmFormat::Xbgr8888,
        Fourcc::Abgr2101010 => GbmFormat::Abgr2101010,
        _ => return None,
    })
}

/// Explicit-modifier bridge allocation. An EMPTY `modifiers` list (or an
/// unsupported `fourcc`) means "use the implicit path" and delegates to
/// [`allocate_dmabuf`] verbatim — byte-identical. Otherwise the BO is allocated
/// with the negotiated list so `bo.modifier()` is a REAL explicit modifier (never
/// INVALID → no AMD wgpu-import crash). Any failure falls back to the implicit path.
pub fn allocate_dmabuf_negotiated(
    render_node: &str,
    width: u32,
    height: u32,
    fourcc: Fourcc,
    modifiers: &[Modifier],
) -> Result<AllocatedDmabuf, AllocError> {
    let gbm_fmt = match (modifiers.is_empty(), gbm_format(fourcc)) {
        (false, Some(f)) => f,
        _ => return allocate_dmabuf(render_node, width, height),
    };
    match allocate_with_modifiers(Path::new(render_node), width, height, fourcc, gbm_fmt, modifiers) {
        Ok(a) => Ok(a),
        Err(e) => {
            warn!("negotiated dmabuf alloc failed ({e:?}); using implicit path");
            allocate_dmabuf(render_node, width, height)
        }
    }
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
    info!(
        "negotiated BO {}x{} fourcc={:?} modifier={:?} planes={}",
        width, height, fourcc, bo.modifier(), bo.plane_count()
    );

    let plane_count = bo.plane_count();
    let modifier = bo.modifier();
    let mut builder = Dmabuf::builder((width as i32, height as i32), fourcc, modifier, DmabufFlags::empty());
    for plane in 0..plane_count {
        let fd = bo.fd_for_plane(plane as i32).map_err(AllocError::ExportFd)?;
        let offset = bo.offset(plane as i32);
        let stride = bo.stride_for_plane(plane as i32);
        builder.add_plane(fd, plane as u32, offset, stride);
    }
    let dmabuf = builder.build().ok_or(AllocError::BuildDmabuf)?;
    publish_stats("gbm-bevy", fourcc, modifier, plane_count);
    Ok(AllocatedDmabuf { dmabuf, _bo: bo, _gbm: gbm })
}

/// Record the post-determined format for the developer "GPU formats" panel.
fn publish_stats(kind: &str, fourcc: Fourcc, modifier: Modifier, plane_count: u32) {
    use compositor_kernel_graphic_bridge_negotiate_classify::classify;
    compositor_developer_stats_registry_gpu::gpu::set_device_format(
        kind,
        &format!("{fourcc:?}"),
        modifier.into(),
        classify::label(classify::classify(modifier)),
        plane_count,
    );
}
