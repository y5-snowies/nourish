//! The strict per-frame routing policy for one output's (render → scanout) handoff.
//!
//! Pure decision the Stage-4 frame executor consults: given the output's
//! [`CopyRoute`] and the render node's folded `mode`, pick the [`FramePath`] — or
//! return an actionable message to abort with. STRICT: no path is taken unless a
//! token names it (no implicit copy, no blit floor). See `document/GPU_TOPOLOGY.md`
//! and `document/GPU_UNTILE_BLIT.md`.

use compositor_developer_environment_config_mode::mode::ModeFlags;
use compositor_kernel_gpu_topology_route_base::route::CopyRoute;

/// The path chosen for one output's per-frame handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePath {
    /// render card == scanout card: composite straight into the scanout buffer.
    Direct,
    /// Cross-device, `zero_copy` enabled: import the render dmabuf into the scanout
    /// side. The executor attempts the shared-modifier import; on failure it consults
    /// [`on_import_failure`].
    ZeroCopyImport,
    /// Cross-device, `untile_blit` enabled: GPU-blit the render frame into a
    /// scanout-importable buffer (the mandatory-but-explicit floor).
    UntileBlit,
}

/// Decide the frame path for `route` under the render node's `mode`. `Err` carries an
/// actionable message the executor must abort with — STRICT: a cross-device output
/// with neither `zero_copy` nor `untile_blit` has no enabled path.
pub fn decide(route: &CopyRoute, mode: ModeFlags) -> Result<FramePath, String> {
    match route {
        CopyRoute::None => Ok(FramePath::Direct),
        CopyRoute::DmabufCopy { render, scanout } => choose(mode).map_err(|_| {
            format!(
                "cross-device output (render {render:?} → scanout {scanout:?}) has no enabled \
                 path: set `zero_copy` and/or `untile_blit` in gpu_router[render].mode. (strict: \
                 no implicit copy, no blit floor). See document/GPU_UNTILE_BLIT.md."
            )
        }),
    }
}

/// The cross-device branch, decoupled from `DrmNode` so it is unit-testable.
fn choose(mode: ModeFlags) -> Result<FramePath, ()> {
    if mode.contains(ModeFlags::ZERO_COPY) {
        Ok(FramePath::ZeroCopyImport)
    } else if mode.contains(ModeFlags::UNTILE_BLIT) {
        Ok(FramePath::UntileBlit)
    } else {
        Err(())
    }
}

/// After a `zero_copy` import (or its DRM atomic test-commit) is rejected: fall back
/// to the blit ONLY when `blit_fallback` AND `untile_blit` are both set; otherwise
/// this is fatal (strict — the fast path failed and no fallback was requested).
pub fn on_import_failure(mode: ModeFlags) -> Result<FramePath, String> {
    if mode.contains(ModeFlags::BLIT_FALLBACK) && mode.contains(ModeFlags::UNTILE_BLIT) {
        Ok(FramePath::UntileBlit)
    } else {
        Err("zero_copy import/test-commit failed and no `blit_fallback` (+ `untile_blit`) is set \
             to fall back to — fatal (strict). See document/GPU_TOPOLOGY.md."
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compositor_developer_environment_config_mode::mode::{self, ModeToken};

    fn m(tokens: &[ModeToken]) -> ModeFlags {
        mode::fold(tokens)
    }

    #[test]
    fn direct_route_ignores_mode() {
        assert_eq!(decide(&CopyRoute::None, ModeFlags::empty()).unwrap(), FramePath::Direct);
        assert_eq!(decide(&CopyRoute::None, m(&[ModeToken::UntileBlit])).unwrap(), FramePath::Direct);
    }

    #[test]
    fn cross_device_needs_an_explicit_path() {
        // empty → no enabled path → Err (strict).
        assert!(choose(ModeFlags::empty()).is_err());
        // untile_blit only → blit.
        assert_eq!(choose(m(&[ModeToken::UntileBlit])).unwrap(), FramePath::UntileBlit);
        // zero_copy present → import (takes precedence over blit for the first path).
        assert_eq!(choose(m(&[ModeToken::ZeroCopy])).unwrap(), FramePath::ZeroCopyImport);
        assert_eq!(
            choose(m(&[ModeToken::ZeroCopy, ModeToken::UntileBlit])).unwrap(),
            FramePath::ZeroCopyImport
        );
    }

    #[test]
    fn import_failure_needs_blit_fallback_and_untile() {
        assert!(on_import_failure(ModeFlags::empty()).is_err());
        // blit_fallback alone is not enough — needs untile_blit too.
        assert!(on_import_failure(m(&[ModeToken::BlitFallback])).is_err());
        assert!(on_import_failure(m(&[ModeToken::UntileBlit])).is_err());
        assert_eq!(
            on_import_failure(m(&[ModeToken::BlitFallback, ModeToken::UntileBlit])).unwrap(),
            FramePath::UntileBlit
        );
    }
}
