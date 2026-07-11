//! Normalize the two settings.json variants into one [`ResolvedRouter`].
//!
//! The **simple** variant (`render_node` (+ `scanout_node`)) desugars to a single
//! entry with an EMPTY behaviour mode — byte-identical to stable HEAD. The
//! **advanced** variant (`gpu_router`) is used directly. `Environment::validate()`
//! (in config.base) already guaranteed exactly-one variant, non-empty keys, and an
//! unambiguous `session_primary`, so [`desugar`] is total.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use compositor_developer_environment_config_base::base as cfg;
use compositor_developer_environment_config_mode::mode::{self, ModeFlags};

/// One declared render→scanout pair with its folded mode.
pub struct ResolvedEntry {
    pub render: PathBuf,
    pub scanout: Option<PathBuf>,
    pub mode: ModeFlags,
}

/// The single resolved model. `primary_*` feed today's single-pair consumers
/// (wgpu pin, capture, env export, orchestration GPU, scanout selection); `entries`
/// is the forward seam for multi-scanout/multi-render assembly.
pub struct ResolvedRouter {
    pub primary_render: PathBuf,
    pub primary_scanout: Option<PathBuf>,
    pub entries: Vec<ResolvedEntry>,
}

static ROUTER: OnceLock<ResolvedRouter> = OnceLock::new();

/// The resolved router, desugared once from the parsed settings.
pub fn router() -> &'static ResolvedRouter {
    ROUTER.get_or_init(desugar)
}

/// The session-primary render node (the one compositing node: wgpu pin, capture,
/// `COMPOSITOR_RENDER_NODE`, orchestration `Environment.GPU`).
pub fn primary_render() -> PathBuf {
    router().primary_render.clone()
}

/// The session-primary render node as a string (for the many `String` readers).
pub fn primary_render_string() -> String {
    router().primary_render.to_string_lossy().into_owned()
}

/// The session-primary scanout override (today's `scanout_node` semantics).
pub fn primary_scanout() -> Option<PathBuf> {
    router().primary_scanout.clone()
}

/// The folded mode for a render node — the entry's tokens verbatim, NO default set
/// OR'd in. An absent/empty entry is `ModeFlags::empty()` (= stable behaviour via
/// preserved polarity).
pub fn mode_for(render: &Path) -> ModeFlags {
    router()
        .entries
        .iter()
        .find(|e| e.render == render)
        .map(|e| e.mode)
        .unwrap_or_else(ModeFlags::empty)
}

/// The session-primary node's mode.
pub fn primary_mode() -> ModeFlags {
    mode_for(&router().primary_render)
}

/// `render_discovery` on the primary node — allow heuristic/first-card fallback.
pub fn render_fallback() -> bool {
    primary_mode().contains(ModeFlags::RENDER_DISCOVERY)
}

/// `scanout_discovery` on the primary node — allow scanout auto-discovery.
pub fn scan_fallback() -> bool {
    primary_mode().contains(ModeFlags::SCANOUT_DISCOVERY)
}

/// `local_render` on the primary node — the "option B" gate: composite on the
/// render node itself (its own GBM) and cross to the scanout card per the route,
/// instead of the default (composite directly on the scanout card's GBM). Off by
/// default, so single-GPU and today's split both keep composite-on-scanout exactly.
pub fn composite_on_render_node() -> bool {
    primary_mode().contains(ModeFlags::LOCAL_RENDER)
}

fn trim_opt(s: &Option<String>) -> Option<PathBuf> {
    s.as_ref()
        .map(|x| x.trim())
        .filter(|x| !x.is_empty())
        .map(PathBuf::from)
}

fn desugar() -> ResolvedRouter {
    let env = cfg::get();

    // Simple variant → a single entry with an empty behaviour mode (= stable HEAD).
    if let Some(r) = &env.render_node {
        let pr = PathBuf::from(r.trim());
        let ps = trim_opt(&env.scanout_node);
        return ResolvedRouter {
            primary_render: pr.clone(),
            primary_scanout: ps.clone(),
            entries: vec![ResolvedEntry { render: pr, scanout: ps, mode: ModeFlags::empty() }],
        };
    }

    // Advanced variant.
    let map = env.gpu_router.as_ref().expect("validate guarantees exactly one variant");
    let single = map.len() == 1;
    let mut entries = Vec::with_capacity(map.len());
    let mut primary: Option<(PathBuf, Option<PathBuf>)> = None;
    for (key, e) in map {
        let render = PathBuf::from(key.trim());
        let scanout = trim_opt(&e.scanout);
        let flags = mode::fold(&e.mode);
        // A single-entry map's lone entry is the anchor regardless of the token —
        // this is what makes `{ "<r>": { "mode": [] } }` == a bare `render_node`.
        if single || flags.contains(ModeFlags::SESSION_PRIMARY) {
            primary = Some((render.clone(), scanout.clone()));
        }
        entries.push(ResolvedEntry { render, scanout, mode: flags });
    }
    entries.sort_by(|a, b| a.render.cmp(&b.render));
    let (primary_render, primary_scanout) =
        primary.expect("validate guarantees a session_primary");
    ResolvedRouter { primary_render, primary_scanout, entries }
}
