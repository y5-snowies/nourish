use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use compositor_developer_environment_config_mode::mode::ModeToken;

/// One render node's routing in the advanced (`gpu_router`) variant. The map KEY is
/// the render-node path; this is its value. `mode` COMPOSES per-node behaviour tokens
/// (see `config.mode`); `scanout` is the KMS card this node scans out to (`None` =
/// auto-anchor to this node's own card).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteEntry {
    #[serde(default)]
    pub mode: Vec<ModeToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scanout: Option<String>,
}

/// The compositor's complete runtime configuration, read from a JSON file
/// (`~/.config/y5.compositor/settings.json`, override with `--config-file=<path>`).
/// Every field is REQUIRED — no defaults; startup panics otherwise — EXCEPT the GPU
/// routing fields, which form two mutually-exclusive variants (exactly one must be
/// present; both or neither panics in [`Environment::validate`]):
/// - **simple**: `render_node` (+ optional `scanout_node`), the single-GPU shape; or
/// - **advanced**: `gpu_router`, a render-node → [`RouteEntry`] map for N GPUs/scanouts.
/// This is the ONE place the compositor reads its own configuration.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    /// Renderer backend: `"vulkan"` or `"gles"`.
    pub renderer: String,
    /// Fall back to GLES if Vulkan initialization fails.
    pub renderer_fallback: bool,
    /// Frame-sync: `""` (off / synchronous `device_wait_idle`), `"infence"` (KMS
    /// IN_FENCE via a binary-semaphore sync_file — original, fragile), or
    /// `"infence_2"` (KMS IN_FENCE via a VkFence `external_fence_fd` sync_file —
    /// fd-guarded + strict, the less-fragile path).
    pub renderer_sync: String,
    /// Enable HDR output (Vulkan only).
    pub hdr: bool,
    /// Scanout bit depth: `8` (SDR) or `10` (deep color).
    pub depth: u8,
    /// Enable adaptive sync / VRR.
    pub vrr: bool,
    /// DRM render node path, e.g. `/dev/dri/renderD128`. The RENDER GPU (the
    /// "active GPU"): where the compositor composites frames. On a normal desktop
    /// this same device also scans out; on split render/scanout systems (PRIME —
    /// e.g. Jetson/Tegra, where `nvgpu` renders but `nvidia-drm` owns the display)
    /// it does not, and `scanout_node` (below) names the KMS card.
    ///
    /// **Simple variant** (mutually exclusive with `gpu_router`). An existing bare
    /// `"render_node": "..."` still deserializes to `Some(..)`; an advanced-only file
    /// omits it and [`Environment::validate`] routes via `gpu_router` instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_node: Option<String>,
    /// Optional DRM **scanout** card path (a KMS-capable card, e.g. `/dev/dri/card0`)
    /// for PRIME split render/scanout systems — valid **only** alongside `render_node`.
    /// Absent/`None` = auto-discover the KMS card, anchored to `render_node`. When set,
    /// it is an EXPLICIT override with NO fallback to auto-discovery. Serialized only
    /// when present, so a normal single-GPU config never carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scanout_node: Option<String>,
    /// **Advanced variant** (mutually exclusive with `render_node`/`scanout_node`): a
    /// render-node-path → [`RouteEntry`] map describing N render→scanout pairs. Exactly
    /// one entry must carry the `session_primary` token when the map has ≥2 entries; for
    /// a single-entry map the lone entry is the implicit anchor. Serialized only when
    /// present, so a normal single-GPU config never carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_router: Option<BTreeMap<String, RouteEntry>>,
    /// XDG desktop name advertised to clients, e.g. `Y5Compositor`.
    pub desktop_name: String,
    /// Developer-log level spec, e.g. `"info,warn,error"`.
    pub log_level: String,
    /// Vulkan diagnostics overlay: `""`, `"vk"`, or `"blit"`.
    pub vk_diag: String,
    /// Capture encoder: `"mesa"`/`"vaapi"` for VAAPI, else NVENC.
    pub capture_encoder: String,
    /// Live-capture video codec: `"av1"` | `"h265"` | `"h264"`. Falls back to
    /// the first available NVENC encoder along av1 → h265 → h264 (VP9 has no
    /// NVENC; reachable only via the optimized software re-encode).
    pub capture_codec: String,
    /// Live-capture quality: `"lossless"` (near-lossless, CQ 19) or
    /// `"optimized"` (smaller, higher CQ — still real-time hardware). Sets the
    /// live NVENC CQ; independent of the optional software re-encode below.
    pub capture_quality: String,
    /// Max live-capture frame rate, clamped to `30..=120`.
    pub capture_refresh_rate_max: u32,
    /// Optional post-capture **software** re-encode (much smaller): `""` = off
    /// (offer it as an "Optimized encoding" checkbox in the save dialog);
    /// `"ffmpeg"` = run it automatically in the background after every recording
    /// (no checkbox; writes a `.y5-encoding` file renamed to the target on done).
    pub capture_background_encoder: String,
    /// `false` (default) = a failed NVENC zero-copy start aborts the capture with
    /// an error dialog. `true` = fall back to the slower GPU→CPU readback encoder
    /// instead. (The readback path also flips correctly on winit, unlike
    /// zero-copy — but it's not used unless this is enabled.)
    pub capture_nvenc_allow_readback_fallback: bool,
    /// `true` = keep the capture's natural variable frame rate (exact timing,
    /// smallest). `false` = produce a constant frame rate, snapped to a standard
    /// rate (else nearest 5), for editors/players that reject VFR. CFR is applied
    /// during the re-encode pass (it can't be done without re-timing frames), so
    /// `false` forces a re-encode even for an otherwise plain save.
    pub capture_variable_frame_rate: bool,
    /// `false` = compositor-tracked window sizing; `true` = client xdg geometry.
    pub window_client_size_fallback: bool,
    /// `false` = fit only the root toplevel; `true` = fit the whole surface tree.
    pub window_subsurface_shrinks: bool,
    // NOTE: live user preferences (cursor sensitivity, touchpad natural-scroll,
    // per-EDID output modes) intentionally do NOT live here. They are not
    // reboot-bound, so they live in `environment.preference` (preferences.json),
    // which is reloaded inline instead of cached once at startup.
}

static ENV: OnceLock<Environment> = OnceLock::new();

/// Resolve the settings-file path: `--config-file=<path>`/`--config-file <path>`
/// from process args if present, else `$XDG_CONFIG_HOME/y5.compositor/settings.json`,
/// else `$HOME/.config/y5.compositor/settings.json`. Shared with the companion tool.
pub fn resolve_path() -> std::path::PathBuf {
    if let Some(p) = config_file_arg(std::env::args()) {
        return std::path::PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .expect("neither XDG_CONFIG_HOME nor HOME set; cannot locate settings.json");
            home.join(".config")
        });
    base.join("y5.compositor").join("settings.json")
}

/// Extract a `--config-file` value from an argument iterator (split out for testing).
pub fn config_file_arg(args: impl Iterator<Item = String>) -> Option<String> {
    let mut it = args;
    while let Some(a) = it.next() {
        if let Some(v) = a.strip_prefix("--config-file=") {
            return Some(v.to_string());
        }
        if a == "--config-file" {
            return it.next();
        }
    }
    None
}

/// Read and parse the settings file exactly once, as the very first thing in
/// `main()` (before logging, which reads `log_level`). Panics with a clear message
/// if the file is unavailable or any required field is missing/invalid. This crate
/// has no logging dep, so it uses `panic!` rather than `abort!`.
pub fn init() {
    let path = resolve_path();
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read settings file {}: {e}. Create it with `y5.compositor.settings`, \
             or pass --config-file=<path>.",
            path.display()
        )
    });
    let parsed: Environment = serde_json::from_str(&raw).unwrap_or_else(|e| {
        panic!("settings file {} is invalid: {e}. Every field is required.", path.display())
    });
    parsed.validate(&path);
    if ENV.set(parsed).is_err() {
        panic!("environment already initialized");
    }
}

impl Environment {
    /// Strict, load-time validation of the two mutually-exclusive GPU routing
    /// variants. Runs in [`init`] after parse, before the config is published, so the
    /// compositor never runs with an ambiguous topology. No logging dep → `panic!`.
    pub fn validate(&self, path: &Path) {
        let where_ = || path.display().to_string();
        let simple = self.render_node.is_some();
        let advanced = self.gpu_router.is_some();
        if simple && advanced {
            panic!(
                "settings {}: set EITHER `render_node` (simple) OR `gpu_router` (advanced), never both.",
                where_()
            );
        }
        if !simple && !advanced {
            panic!(
                "settings {}: must set `render_node` (single-GPU) or `gpu_router` (multi-GPU).",
                where_()
            );
        }
        if advanced && self.scanout_node.is_some() {
            panic!(
                "settings {}: `scanout_node` is only valid with `render_node`; put `scanout` inside each `gpu_router` entry.",
                where_()
            );
        }
        if let Some(r) = &self.render_node
            && r.trim().is_empty()
        {
            panic!("settings {}: `render_node` is empty.", where_());
        }
        let Some(map) = &self.gpu_router else { return };
        if map.is_empty() {
            panic!(
                "settings {}: `gpu_router` is empty; add at least one render-node entry, or use `render_node`.",
                where_()
            );
        }
        let mut primary_count = 0usize;
        for (key, e) in map {
            if key.trim().is_empty() {
                panic!("settings {}: `gpu_router` has an empty render-node key.", where_());
            }
            let has = |t: ModeToken| e.mode.contains(&t);
            let is_primary = has(ModeToken::SessionPrimary);
            if is_primary {
                primary_count += 1;
            }
            let routes_somewhere = e.scanout.is_some() || is_primary;
            if !routes_somewhere
                && (has(ModeToken::ZeroCopy)
                    || has(ModeToken::LocalRender)
                    || has(ModeToken::BlitFallback))
            {
                panic!(
                    "settings {}: `gpu_router[{key}]` has a routing token (zero_copy/local_render/blit_fallback) but no `scanout` and is not `session_primary` — it routes nowhere.",
                    where_()
                );
            }
            if has(ModeToken::BlitFallback) && !has(ModeToken::UntileBlit) {
                panic!(
                    "settings {}: `gpu_router[{key}]` has `blit_fallback` without `untile_blit` — nothing to fall back to.",
                    where_()
                );
            }
        }
        // session_primary is implicit for a single-entry map; required (exactly one)
        // only to disambiguate a multi-entry map.
        if map.len() >= 2 && primary_count != 1 {
            panic!(
                "settings {}: a multi-entry `gpu_router` needs exactly one `session_primary` entry (found {primary_count}).",
                where_()
            );
        }
    }
}

/// The parsed environment. Panics if called before [`init`].
pub fn get() -> &'static Environment {
    ENV.get().expect("environment not initialized; call init() first in main()")
}

/// Canonical complete starting settings — the single source of default values shared by
/// the configuration TOOLS: the `y5.compositor.settings` editor and the installer's seed.
/// NOT used by the compositor at runtime — [`init`] still requires a fully-populated file
/// and never falls back to these, so a real config can't be silently half-default. Living
/// here (with the struct) means the editor and the installer agree on one set of values
/// across the full schema (the required fields plus the simple `render_node` variant; the
/// optional `scanout_node`/`gpu_router` are omitted), so any seeded file is complete and valid.
pub fn default_settings() -> Environment {
    Environment {
        renderer: "vulkan".to_string(),
        renderer_fallback: true,
        renderer_sync: String::new(),
        hdr: false,
        depth: 8,
        vrr: false,
        render_node: Some("/dev/dri/renderD128".to_string()),
        scanout_node: None,
        gpu_router: None,
        desktop_name: "Y5Compositor".to_string(),
        log_level: "info,warn,error".to_string(),
        vk_diag: String::new(),
        capture_encoder: "nvenc".to_string(),
        capture_codec: "av1".to_string(),
        capture_quality: "optimized".to_string(),
        capture_refresh_rate_max: 120,
        capture_background_encoder: "ffmpeg".to_string(),
        capture_nvenc_allow_readback_fallback: false,
        capture_variable_frame_rate: false,
        window_client_size_fallback: false,
        window_subsurface_shrinks: false,
    }
}

/// Re-read and parse the settings file from disk **right now** — NOT the cached
/// startup snapshot from [`get`]. The in-compositor settings window calls this
/// every time it opens so a settings file edited from the terminal (or by a
/// previous settings session) is reflected on the next launch. Falls back to the
/// startup snapshot, then the canonical defaults, if the file is missing/invalid
/// (the window should still open with sane values).
pub fn read_current() -> Environment {
    let path = resolve_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Environment>(&raw).ok())
        .or_else(|| ENV.get().cloned())
        .unwrap_or_else(default_settings)
}

/// Persist `env` to the settings file atomically (write to a sibling `.tmp`, then
/// rename over the target — a partial write can never replace a good file). Used
/// by the settings window to save edits. This only updates the on-disk file:
/// every `Environment` field is read once at startup, so a change takes effect at
/// the next launch (the window surfaces a "reboot to apply" banner). Live,
/// inline-reloaded settings live in `environment.preference`, not here.
pub fn save(env: &Environment) -> Result<(), String> {
    let path = resolve_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    let json =
        serde_json::to_string_pretty(env).map_err(|e| format!("serialize settings: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename {}: {e}", path.display()))?;
    Ok(())
}
