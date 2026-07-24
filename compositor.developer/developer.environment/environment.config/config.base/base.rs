use std::sync::OnceLock;

/// The compositor's complete runtime configuration, read from a JSON file
/// (`~/.config/y5.compositor/settings.json`, override with `--config-file=<path>`).
/// Every field is REQUIRED — no optionals, no defaults; startup panics otherwise.
/// This is the ONE place the compositor reads its own configuration.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    /// Settings-schema version ([`SCHEMA_VERSION`]). Files with an older (or
    /// absent — treated as 1) version are migrated in memory on every load:
    /// fields added since get their defaults. The file itself is NOT rewritten
    /// — it stays in its authored version until an explicit [`save`] (the
    /// settings editor), which always writes the current schema.
    pub version: u32,
    /// Renderer backend: `"vulkan"` or `"gles"`.
    pub renderer: String,
    /// Fall back to GLES if Vulkan initialization fails.
    pub renderer_fallback: bool,
    /// Frame-sync: `""` (synchronous, default), `"infence"` (KMS IN_FENCE,
    /// raw — no fence validation, for observing actual hardware behavior), or
    /// `"infence_fallback_sync"` (KMS IN_FENCE with a first-frame fence
    /// self-test, degrading to synchronous if it fails; hand-set only — the
    /// settings editor never writes it).
    pub renderer_sync: String,
    /// Enable HDR output (Vulkan only).
    pub hdr: bool,
    /// Scanout bit depth: `8` (SDR) or `10` (deep color).
    pub depth: u8,
    /// Enable adaptive sync / VRR.
    pub vrr: bool,
    /// DRM render node path, e.g. `/dev/dri/renderD128`.
    pub render_node: String,
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
    /// CPU scheduling boost: `""` (default scheduling), `"auto"` (raise the
    /// compositor's priority — direct nice first, rtkit over D-Bus second — so
    /// frame deadlines survive all-core loads like shader-compile storms), or
    /// `"realtime"` (SCHED_RR, inherited by compositor threads; needs the
    /// CAP_SYS_NICE the build scripts setcap, degrades to `"auto"` otherwise).
    /// In both modes children stop inheriting the boost once startup completes
    /// (SCHED_RESET_ON_FORK is armed after the initial threads exist).
    pub priority: String,
    /// `false` = compositor-tracked window sizing; `true` = client xdg geometry.
    pub window_client_size_fallback: bool,
    /// `false` = fit only the root toplevel; `true` = fit the whole surface tree.
    pub window_subsurface_shrinks: bool,
    // NOTE: live user preferences (cursor sensitivity, touchpad natural-scroll,
    // per-EDID output modes) intentionally do NOT live here. They are not
    // reboot-bound, so they live in `environment.preference` (preferences.json),
    // which is reloaded inline instead of cached once at startup.
}

/// Current settings-schema version. Bump when adding fields, and teach
/// [`migrate`] to fill the new fields' defaults for older files.
pub const SCHEMA_VERSION: u32 = 2;

/// Migrate a parsed settings JSON object in place to [`SCHEMA_VERSION`],
/// chaining version steps up to the current schema (now just v1 → v2).
/// Returns `true` if anything changed. In-memory only — callers never persist
/// the result implicitly; the file keeps its authored version until an
/// explicit [`save`]. Fields are only ever ADDED with their defaults —
/// existing values are never touched, so a migration cannot alter configured
/// behavior.
pub fn migrate(root: &mut serde_json::Value) -> bool {
    let Some(obj) = root.as_object_mut() else {
        return false;
    };
    let version = obj.get("version").and_then(|v| v.as_u64()).unwrap_or(1);
    if version >= SCHEMA_VERSION as u64 {
        return false;
    }
    if version < 2 {
        // v1 → v2: `priority` (CPU scheduling boost, default off) + `version`.
        obj.entry("priority").or_insert_with(|| serde_json::Value::String(String::new()));
    }
    obj.insert("version".into(), serde_json::Value::from(SCHEMA_VERSION));
    true
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
    let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|e| {
        panic!("settings file {} is not valid JSON: {e}.", path.display())
    });
    migrate(&mut value);
    let parsed: Environment = serde_json::from_value(value).unwrap_or_else(|e| {
        panic!("settings file {} is invalid: {e}. Every field is required.", path.display())
    });
    if ENV.set(parsed).is_err() {
        panic!("environment already initialized");
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
/// across the full schema, so any seeded file is always complete and valid.
pub fn default_settings() -> Environment {
    Environment {
        version: SCHEMA_VERSION,
        renderer: "vulkan".to_string(),
        renderer_fallback: true,
        renderer_sync: String::new(),
        hdr: false,
        depth: 8,
        vrr: false,
        render_node: "/dev/dri/renderD128".to_string(),
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
        priority: String::new(),
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
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|mut value| {
            migrate(&mut value);
            serde_json::from_value::<Environment>(value).ok()
        })
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

#[cfg(test)]
mod tests {
    /// A v1 file (no `version`, no `priority`) migrates to a complete v2
    /// Environment with the new fields at their defaults — and the migration
    /// is idempotent.
    #[test]
    fn migrates_v1_settings() {
        let mut v = serde_json::to_value(super::default_settings()).unwrap();
        let obj = v.as_object_mut().unwrap();
        obj.remove("version");
        obj.remove("priority");
        assert!(super::migrate(&mut v));
        let env: super::Environment = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(env.version, super::SCHEMA_VERSION);
        assert_eq!(env.priority, "");
        assert!(!super::migrate(&mut v));
    }

    /// Migration never overwrites an existing value.
    #[test]
    fn migrate_preserves_existing_values() {
        let mut v = serde_json::to_value(super::default_settings()).unwrap();
        let obj = v.as_object_mut().unwrap();
        obj.remove("version");
        obj.insert("priority".into(), serde_json::Value::String("auto".into()));
        assert!(super::migrate(&mut v));
        let env: super::Environment = serde_json::from_value(v).unwrap();
        assert_eq!(env.priority, "auto");
    }
}
