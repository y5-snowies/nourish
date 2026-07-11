//! DRM render node → gbm BO → Dmabuf.
//!
//! This is the bottom of the import stack. `allocate_dmabuf` returns an
//! `AllocatedDmabuf` that owns the gbm device + buffer object. Dropping it
//! releases the underlying GPU memory; keep it alive for as long as either
//! WGPU or GLES holds an imported view.
//!
//! The returned `Dmabuf` is cheap to clone (it's just plane fds + metadata),
//! and the `AllocatedDmabuf` wrapper holds the lifetime-critical pieces.

use std::os::fd::{AsRawFd, OwnedFd};
use std::path::Path;

use crate::error::AllocError;
use gbm::{BufferObjectFlags, Device as GbmDevice, Format as GbmFormat};
use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufFlags};
use smithay::backend::allocator::{Buffer, Fourcc, Modifier};

/// The `scanout_bridge` mode token (primary node): allocate bridge buffers with
/// `SCANOUT` usage so the driver picks a scanout-plane-compatible (tiled)
/// modifier. Off = today's RENDERING-only behavior.
fn scanout_bridge() -> bool {
    use compositor_developer_environment_config_mode::mode::ModeFlags;
    compositor_developer_environment_config_router::router::primary_mode()
        .contains(ModeFlags::SCANOUT_BRIDGE)
}

/// BO usage flags for a bridge allocation: `SCANOUT` is added under the
/// `gpu_scanout_bridge` experiment so gbm targets a plane-scannable layout.
fn bo_flags() -> BufferObjectFlags {
    if scanout_bridge() {
        BufferObjectFlags::RENDERING | BufferObjectFlags::SCANOUT
    } else {
        BufferObjectFlags::RENDERING
    }
}

/// Opaque holder for an allocated buffer. Keeps gbm alive while the dmabuf
/// is in use. Drop order inside this struct: `dmabuf` first (releases fds
/// and any imports), then `_bo`, then `_gbm`. Rust drops struct fields in
/// declaration order, so the ordering below is load-bearing — don't reorder.
pub struct AllocatedDmabuf {
    pub dmabuf: Dmabuf,
    // The buffer object holds the GPU allocation; dropping it before the
    // dmabuf would close handles the dmabuf still references.
    _bo: gbm::BufferObject<()>,
    // The gbm device must outlive any BO created from it.
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

/// Path to the DRM render node we use for allocations.
///
/// Exposed as a constant so callers can verify or override. `/dev/dri/renderD129`
/// is the second render node on systems with multiple GPUs (NVIDIA + iGPU);
/// adjust for single-GPU systems if you find this wrong.
// pub const DEFAULT_RENDER_NODE: &str = GPU_DEVICE;

/// Allocate a single ARGB8888 LINEAR dmabuf at the given size.
///
/// LINEAR is the safest modifier for cross-API sharing (Mesa/NVIDIA/GLES/Vulkan
/// all handle it). Returns an `AllocatedDmabuf` that owns the gbm device and
/// buffer object; the inner `Dmabuf` can be cloned cheaply and imported by
/// wgpu or GLES.
pub fn allocate_dmabuf(
    render_node: &str,
    width: u32,
    height: u32,
) -> Result<AllocatedDmabuf, AllocError> {
    allocate_dmabuf_on(Path::new(render_node), width, height)
}

/// Variant of [`allocate_dmabuf`] with an explicit render node path.
pub fn allocate_dmabuf_on(
    render_node: &Path,
    width: u32,
    height: u32,
) -> Result<AllocatedDmabuf, AllocError> {
    if width == 0 || height == 0 {
        return Err(AllocError::InvalidDimensions { width, height });
    }

    // 1. Open the render node.
    let drm_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(render_node)
        .map_err(AllocError::OpenDrm)?;
    let drm_fd: OwnedFd = drm_file.into();

    info!(
        "Opened DRM render node {} (fd={})",
        render_node.display(),
        drm_fd.as_raw_fd()
    );

    // 2. Wrap in a gbm device.
    let gbm = GbmDevice::new(drm_fd).map_err(AllocError::GbmInit)?;

    // 3. Allocate the buffer object. RENDERING usage = usable as render target.
    let bo = gbm
        .create_buffer_object::<()>(
            width,
            height,
            GbmFormat::Argb8888,
            bo_flags(),
        )
        .map_err(AllocError::CreateBo)?;

    info!(
        "Allocated gbm BO {}x{}, format=ARGB8888, modifier={:?}, stride={}, tight_pitch={}, offset={}, planes={}",
        width,
        height,
        bo.modifier(),
        bo.stride_for_plane(0),
        width * 4,
        bo.offset(0),
        bo.plane_count(),
    );

    // 4. Export plane(s) as a Smithay Dmabuf.
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

    info!(
        "Built Dmabuf: size={:?}, format={:?}, num_planes={}, modifier={:?}",
        dmabuf.size(),
        dmabuf.format(),
        dmabuf.num_planes(),
        modifier,
    );

    Ok(AllocatedDmabuf {
        dmabuf,
        _bo: bo,
        _gbm: gbm,
    })
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
    // TEMPORARY (`gpu_scanout_bridge`): the negotiated modifiers don't yet include
    // the scanout plane's set, so an explicit list would pin a non-scannable (e.g.
    // LINEAR) modifier. Fall through to the implicit path, which now carries
    // `SCANOUT` usage → the driver picks a plane-compatible tiled modifier.
    if scanout_bridge() {
        return allocate_dmabuf(render_node, width, height);
    }
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

/// Allocate a single-plane `LINEAR` dmabuf on a SPECIFIC card, forcing a
/// pitch-linear layout via the `GBM_BO_USE_LINEAR` **usage flag** (not the
/// explicit-modifier path).
///
/// This is the untiling blit's OUTPUT buffer (see `document/GPU_UNTILE_BLIT.md`):
/// LINEAR is the universal interop layout the scanout side can always import, and
/// the render GPU copies its native tiled frame into it.
///
/// Why the usage flag and not `create_buffer_object_with_modifiers2([LINEAR])`:
/// NVIDIA's gbm rejects the explicit-LINEAR-modifier allocation with `EINVAL`,
/// whereas `GBM_BO_USE_LINEAR` is honored — and allocating this buffer on the
/// RENDER node is what keeps the blit copy same-device (no foreign-pitch shear).
/// The resulting `bo.modifier()` is `DRM_FORMAT_MOD_LINEAR`, which every importer
/// handles.
pub fn allocate_linear_on(
    node: &str,
    width: u32,
    height: u32,
    fourcc: Fourcc,
) -> Result<AllocatedDmabuf, AllocError> {
    let gbm_fmt = gbm_format(fourcc).ok_or(AllocError::UnsupportedFourcc(fourcc))?;
    if width == 0 || height == 0 {
        return Err(AllocError::InvalidDimensions { width, height });
    }
    // Try SCANOUT|LINEAR first: SCANOUT makes the driver align the row pitch (i915
    // pads to a 256-byte-aligned stride). This matters because the render GPU's copy
    // engine (NVIDIA) writes linear images at a 256-aligned pitch; if the scanout
    // card handed back a TIGHT pitch (`width*4`, not 256-aligned), the two disagree
    // and every row drifts — the partial-stretch shear. An aligned stride makes both
    // sides agree. Fall back to plain LINEAR if SCANOUT is refused (e.g. tiny BOs).
    let flags = BufferObjectFlags::RENDERING | BufferObjectFlags::LINEAR;
    alloc_linear_flags(node, width, height, fourcc, gbm_fmt, flags | BufferObjectFlags::SCANOUT)
        .or_else(|e| {
            warn!("linear+scanout alloc failed ({e:?}); retrying plain linear (tight pitch)");
            alloc_linear_flags(node, width, height, fourcc, gbm_fmt, flags)
        })
}

/// Allocate one LINEAR bo with the given gbm usage flags and export it as a dmabuf.
fn alloc_linear_flags(
    node: &str,
    width: u32,
    height: u32,
    fourcc: Fourcc,
    gbm_fmt: GbmFormat,
    flags: BufferObjectFlags,
) -> Result<AllocatedDmabuf, AllocError> {
    let drm_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(Path::new(node))
        .map_err(AllocError::OpenDrm)?;
    let drm_fd: OwnedFd = drm_file.into();
    let gbm = GbmDevice::new(drm_fd).map_err(AllocError::GbmInit)?;

    let bo = gbm
        .create_buffer_object::<()>(width, height, gbm_fmt, flags)
        .map_err(AllocError::CreateBo)?;
    let modifier = bo.modifier();
    let stride = bo.stride_for_plane(0);
    info!(
        "linear BO {}x{} fourcc={:?} modifier={:?} stride={} tight_pitch={} 256_aligned={} scanout={} offset={} planes={}",
        width,
        height,
        fourcc,
        modifier,
        stride,
        width * 4,
        stride % 256 == 0,
        flags.contains(BufferObjectFlags::SCANOUT),
        bo.offset(0),
        bo.plane_count()
    );

    let plane_count = bo.plane_count();
    let mut builder = Dmabuf::builder((width as i32, height as i32), fourcc, modifier, DmabufFlags::empty());
    for plane in 0..plane_count {
        let fd = bo.fd_for_plane(plane as i32).map_err(AllocError::ExportFd)?;
        let offset = bo.offset(plane as i32);
        let stride = bo.stride_for_plane(plane as i32);
        builder.add_plane(fd, plane as u32, offset, stride);
    }
    let dmabuf = builder.build().ok_or(AllocError::BuildDmabuf)?;
    publish_stats("gbm-iced-linear", fourcc, modifier, plane_count);
    Ok(AllocatedDmabuf { dmabuf, _bo: bo, _gbm: gbm })
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
    trace!(
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
    publish_stats("gbm-iced", fourcc, modifier, plane_count);
    Ok(AllocatedDmabuf {
        dmabuf,
        _bo: bo,
        _gbm: gbm,
    })
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
