//! Typed GPU preference: ranking + exclusion. A self-contained value type.
//! `get()` returns the default (empty) rank — the path-based hotplug allow/deny
//! set — unchanged. The settings-backed node accessors below expose the
//! configured RENDER node and the optional SCANOUT override to the kernel
//! selection path (dev_id normalization is the consumer's job, not ours).

use std::path::PathBuf;

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

/// The configured RENDER node (`settings.json` `render_node`) — the "active GPU"
/// the compositor composites on (and pins wgpu to). The scanout path anchors to
/// this. `None` only if unset/blank (misconfiguration; selection then falls back
/// to smithay's heuristic, preserving pre-settings behavior).
pub fn render_node() -> Option<PathBuf> {
    let s = compositor_developer_environment_config_base::base::get().render_node.trim().to_string();
    (!s.is_empty()).then(|| PathBuf::from(s))
}

/// The optional explicit SCANOUT card (`settings.json` `scanout_node`). `Some`
/// = an explicit override (no auto-discovery, probe-or-panic); `None` = auto-
/// discover the KMS card anchored to [`render_node`].
pub fn scanout_node() -> Option<PathBuf> {
    compositor_developer_environment_config_base::base::get()
        .scanout_node
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}
