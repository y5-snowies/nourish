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
    /// Frame-sync. The KMS IN_FENCE path is the DEFAULT and the only opt-OUT is
    /// the explicit [`RENDERER_SYNC_SYNCHRONOUS`]; see [`renderer_sync_fence`]
    /// for why every other spelling — including the empty string older files
    /// carry — resolves to the fence path. [`RENDERER_SYNC_SELF_TEST`] is the
    /// fence path plus a first-frame validation, hand-set only (the settings
    /// editor never writes it).
    pub renderer_sync: String,
    /// Enable HDR output (Vulkan only).
    pub hdr: bool,
    /// Scanout bit depth: `8` (SDR) or `10` (deep color).
    pub depth: u8,
    /// Enable adaptive sync / VRR.
    pub vrr: bool,
    /// DRM render node path, e.g. `/dev/dri/renderD128`.
    pub render_node: String,
    /// DRM device to SCAN OUT on, e.g. `/dev/dri/card1`. Empty (the default) =
    /// pick it with the udev heuristic, which is what every release before this
    /// field did unconditionally.
    ///
    /// Set it when the heuristic picks the wrong device — notably where the
    /// render engine and the display engine are SEPARATE DRM devices, so the
    /// device holding [`Self::render_node`] has no connectors at all. A
    /// Raspberry Pi 4/5 is the standard case: `v3d` owns `renderD128` and has no
    /// CRTCs, `vc4` owns the HDMI connectors and has no render node, and
    /// smithay's `primary_gpu()` prefers "the device that has a render node" —
    /// so it selects `v3d` and assembly dies probing its (absent) resources.
    ///
    /// This is the SCANOUT half only. It does not move the renderer:
    /// [`Self::render_node`] still drives the wgpu/bevy pin, and the GLES
    /// `GpuManager` is still paired with the scanout device's GBM — a pairing
    /// that is only correct while the two are the same device.
    pub scanout_node: String,
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
    /// [`PRIORITY_DEFAULT`] (SCHED_RR, inherited by compositor threads; needs
    /// the CAP_SYS_NICE the build scripts setcap, degrades to exactly what
    /// `"auto"` does otherwise, rtkit included). In both modes children stop
    /// inheriting the boost once startup completes (SCHED_RESET_ON_FORK is
    /// armed after the initial threads exist).
    ///
    /// Not prompted anywhere: the shipped value is the right one, and the two
    /// weaker settings exist for hand-editing a machine that misbehaves under
    /// SCHED_RR, not as a choice to put in front of someone installing.
    pub priority: String,
    // NOTE: live user preferences (cursor sensitivity, touchpad natural-scroll,
    // per-EDID output modes) intentionally do NOT live here. They are not
    // reboot-bound, so they live in `environment.preference` (preferences.json),
    // which is reloaded inline instead of cached once at startup.
}

/// Current settings-schema version. Bump when adding fields, and teach
/// [`migrate`] to fill the new fields' defaults for older files.
pub const SCHEMA_VERSION: u32 = 5;

/// The stored [`Environment::renderer_sync`] for a fresh install. `"infence"`
/// is the historical spelling and stays the canonical stored form;
/// [`RENDERER_SYNC_NATIVE`] is the advertised one.
pub const RENDERER_SYNC_DEFAULT: &str = "infence";

/// The advertised alias for [`RENDERER_SYNC_DEFAULT`] — what the settings
/// editor calls it, since "in-fence" names a KMS property rather than anything
/// a person installing a compositor is choosing between.
pub const RENDERER_SYNC_NATIVE: &str = "native";

/// The ONE value that selects the synchronous `device_wait_idle` submit — the
/// path everything used to get from an empty string.
pub const RENDERER_SYNC_SYNCHRONOUS: &str = "sync";

/// The fence path plus a first-frame fence self-test that degrades to
/// synchronous if the exported fence never signals. Hand-set only.
pub const RENDERER_SYNC_SELF_TEST: &str = "infence_fallback_sync";

/// Does `raw` select the KMS IN_FENCE path? Everything that is not the explicit
/// [`RENDERER_SYNC_SYNCHRONOUS`] opt-out does.
///
/// Deliberately a denylist rather than an allowlist. The fence path is the
/// default, and a file can carry any of several dead spellings for it (the
/// empty string this field shipped with, `"kms"`, a typo); resolving all of
/// them to the default means the only way to end up on the slow path is to ask
/// for it by name. [`migrate`] rewrites the dead spellings so the file agrees
/// with the behaviour, but this is what actually decides.
pub fn renderer_sync_fence(raw: &str) -> bool {
    !raw.eq_ignore_ascii_case(RENDERER_SYNC_SYNCHRONOUS)
}

/// Does `raw` ask for the first-frame fence self-test? Implies
/// [`renderer_sync_fence`].
pub fn renderer_sync_self_test(raw: &str) -> bool {
    raw.eq_ignore_ascii_case(RENDERER_SYNC_SELF_TEST)
}

/// The canonical spelling of `raw` — what a file should hold to mean what `raw`
/// resolves to. Only the three live values survive verbatim; every dead
/// spelling collapses onto [`RENDERER_SYNC_DEFAULT`].
pub fn normalized_renderer_sync(raw: &str) -> String {
    match () {
        _ if !renderer_sync_fence(raw) => RENDERER_SYNC_SYNCHRONOUS.to_string(),
        _ if renderer_sync_self_test(raw) => RENDERER_SYNC_SELF_TEST.to_string(),
        _ => RENDERER_SYNC_DEFAULT.to_string(),
    }
}

/// The shipped [`Environment::priority`]. SCHED_RR, because the failure it
/// prevents (a commit missing vblank because the compositor was not scheduled
/// in time) is one the user experiences as the compositor stuttering, and the
/// degradation path when the binary is not setcap'd is exactly `"auto"`.
pub const PRIORITY_DEFAULT: &str = "realtime";

/// Migrate a parsed settings JSON object in place to [`SCHEMA_VERSION`],
/// chaining version steps up to the current schema. Returns `true` if anything
/// changed. In-memory only — callers never persist the result implicitly; the
/// file keeps its authored version until an explicit [`save`].
///
/// Normally a step only ADDS a field at its default, so it cannot alter
/// configured behavior. v2 → v3 is the exception and is deliberate: see the
/// step's own comment.
pub fn migrate(root: &mut serde_json::Value) -> bool {
    let Some(obj) = root.as_object_mut() else {
        return false;
    };
    let version = obj.get("version").and_then(|v| v.as_u64()).unwrap_or(1);
    if version >= SCHEMA_VERSION as u64 {
        return false;
    }
    if version < 2 {
        // v1 → v2: `priority` (CPU scheduling boost) + `version`.
        obj.entry("priority")
            .or_insert_with(|| serde_json::Value::String(PRIORITY_DEFAULT.to_string()));
    }
    if version < 3 {
        // v2 → v3: `renderer_sync`'s default flipped from synchronous to the KMS
        // IN_FENCE path, and the opt-out moved from "the empty string" to the
        // explicit `"sync"`. So every v2 file that is not already asking for the
        // self-test carries a spelling that no longer means what it meant, and
        // rewriting it here is what keeps the file honest about the behavior it
        // gets. This DOES change a v2 user's configured sync mode — intentionally;
        // the empty string was the absence of a choice, not a choice.
        let raw = obj.get("renderer_sync").and_then(|v| v.as_str()).unwrap_or_default();
        let normalized = normalized_renderer_sync(raw);
        obj.insert("renderer_sync".into(), serde_json::Value::String(normalized));
    }
    if version < 4 {
        // v3 → v4: `scanout_node`, added at its default (empty = the udev
        // heuristic), so an existing file keeps selecting the scanout device
        // exactly as it did before the field existed.
        obj.entry("scanout_node")
            .or_insert_with(|| serde_json::Value::String(String::new()));
    }
    if version < 5 {
        // v4 → v5: `window_client_size_fallback` and `window_subsurface_shrinks`
        // REMOVED. Both were experiments in letting the client's own geometry override
        // the compositor's slot, and both shipped `false`; the code paths behind them
        // are gone, so the fields describe nothing.
        //
        // A REMOVAL step, which no other version needed — `Environment` is
        // `deny_unknown_fields`, so a file still carrying either key fails to parse
        // outright rather than degrading. Dropping the keys here is what lets an
        // existing settings.json load at all.
        obj.remove("window_client_size_fallback");
        obj.remove("window_subsurface_shrinks");
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
        renderer_sync: RENDERER_SYNC_DEFAULT.to_string(),
        hdr: false,
        depth: 8,
        vrr: false,
        render_node: "/dev/dri/renderD128".to_string(),
        // Empty = the udev heuristic, i.e. the behavior of every release before
        // this field existed.
        scanout_node: String::new(),
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
        priority: PRIORITY_DEFAULT.to_string(),
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
    /// A v1 file (no `version`, no `priority`) migrates to a complete current
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
        assert_eq!(env.priority, super::PRIORITY_DEFAULT);
        assert!(!super::migrate(&mut v));
    }

    /// Migration never overwrites a value it did not set out to rewrite —
    /// `priority` is added-if-absent, so an authored one survives.
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

    /// v2 → v3 rewrites every dead `renderer_sync` spelling onto the fence
    /// default, and leaves the two live opt-outs exactly as authored.
    #[test]
    fn migrates_v2_renderer_sync() {
        let cases = [
            ("", super::RENDERER_SYNC_DEFAULT),
            ("kms", super::RENDERER_SYNC_DEFAULT),
            ("native", super::RENDERER_SYNC_DEFAULT),
            ("infence", super::RENDERER_SYNC_DEFAULT),
            ("sync", super::RENDERER_SYNC_SYNCHRONOUS),
            ("infence_fallback_sync", super::RENDERER_SYNC_SELF_TEST),
        ];
        for (authored, expected) in cases {
            let mut v = serde_json::to_value(super::default_settings()).unwrap();
            let obj = v.as_object_mut().unwrap();
            obj.insert("version".into(), serde_json::Value::from(2));
            obj.insert("renderer_sync".into(), serde_json::Value::String(authored.into()));
            assert!(super::migrate(&mut v));
            let env: super::Environment = serde_json::from_value(v).unwrap();
            assert_eq!(env.renderer_sync, expected, "authored {authored:?}");
        }
    }

    /// v3 → v4 adds `scanout_node` at the empty default (= the udev heuristic),
    /// and an authored value survives the step.
    #[test]
    fn migrates_v3_scanout_node() {
        let mut v = serde_json::to_value(super::default_settings()).unwrap();
        let obj = v.as_object_mut().unwrap();
        obj.insert("version".into(), serde_json::Value::from(3));
        obj.remove("scanout_node");
        assert!(super::migrate(&mut v));
        let env: super::Environment = serde_json::from_value(v).unwrap();
        assert_eq!(env.scanout_node, "");

        let mut v = serde_json::to_value(super::default_settings()).unwrap();
        let obj = v.as_object_mut().unwrap();
        obj.insert("version".into(), serde_json::Value::from(3));
        obj.insert("scanout_node".into(), serde_json::Value::String("/dev/dri/card1".into()));
        assert!(super::migrate(&mut v));
        let env: super::Environment = serde_json::from_value(v).unwrap();
        assert_eq!(env.scanout_node, "/dev/dri/card1");
    }

    /// The fence path is the default, so only the explicit opt-out leaves it —
    /// including for values no migration has (or ever will) rewrite.
    #[test]
    fn only_sync_opts_out_of_the_fence_path() {
        assert!(!super::renderer_sync_fence("sync"));
        assert!(!super::renderer_sync_fence("SYNC"));
        for on in ["", "kms", "native", "infence", "infence_fallback_sync", "typo"] {
            assert!(super::renderer_sync_fence(on), "{on:?}");
        }
        assert!(super::renderer_sync_self_test("infence_fallback_sync"));
        assert!(!super::renderer_sync_self_test("infence"));
    }
}
