//! Typed GPU preference: ranking + exclusion. A self-contained value type.
//! `get()` returns the default (empty) rank — the path-based hotplug allow/deny
//! set — unchanged. The settings-backed node accessors below expose the
//! configured RENDER node and the optional SCANOUT override to the kernel
//! selection path (dev_id normalization is the consumer's job, not ours).

use std::path::{Path, PathBuf};

use compositor_developer_environment_config_mode::mode::ModeFlags;
use compositor_developer_environment_config_router::router;

#[derive(Debug, Clone, Default)]
pub struct GpuRank {
    /// Device paths preferred as primary, in order.
    pub preferred: Vec<PathBuf>,
    /// Device paths that must never be used (ignored nodes).
    pub ignored: Vec<PathBuf>,
}

pub fn get() -> GpuRank {
    GpuRank::default()
}

/// The session-primary RENDER node — the "active GPU" the compositor composites on
/// (and pins wgpu to). Resolved by [`router`] from either settings variant (simple
/// `render_node` or the advanced `gpu_router` primary). Always `Some` (config
/// validation guarantees a primary); the `Option` is kept for call-site stability.
pub fn render_node() -> Option<PathBuf> {
    Some(router::primary_render())
}

/// The session-primary explicit SCANOUT card. `Some` = an explicit override (no
/// auto-discovery unless `scanout_discovery`); `None` = auto-anchor to [`render_node`].
pub fn scanout_node() -> Option<PathBuf> {
    router::primary_scanout()
}

/// The folded per-node `mode` for `render` (the entry's tokens verbatim; an
/// absent/empty entry is `ModeFlags::empty()` = stable behaviour).
pub fn mode_for(render: &Path) -> ModeFlags {
    router::mode_for(render)
}

/// The session-primary node's folded `mode`.
pub fn primary_mode() -> ModeFlags {
    router::primary_mode()
}

/// `render_discovery` on the primary node — allow heuristic/first-card fallback
/// instead of panicking on an unavailable render node.
pub fn render_fallback() -> bool {
    router::render_fallback()
}

/// `scanout_discovery` on the primary node — allow scanout auto-discovery instead
/// of panicking on a non-KMS/unavailable scanout.
pub fn scan_fallback() -> bool {
    router::scan_fallback()
}
