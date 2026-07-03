//! Typed per-monitor preference keyed by EDID identity. Populated from the live
//! preferences document (`preferences.json`) via [`get`]; the kernel keeps its own
//! self-contained value type so the hardware path never depends on the on-disk serde
//! shape directly — [`get`] maps the developer-side schema onto it. Order is
//! preserved: the FIRST profile is the default output (see `display.base`'s
//! `profiles.first()`), matching how the settings UI orders them.

#[derive(Debug, Clone)]
pub enum ModeRequest {
    /// Pick from advertised modes (current default policy applies when None).
    Advertised { width: u16, height: u16, refresh_mhz: u32 },
    /// Synthesize via CVT (requires the mode-synthesis safety enable).
    Cvt { width: u16, height: u16, refresh: f64 },
    /// Raw modeline string (requires the mode-synthesis safety enable).
    Modeline(String),
}

#[derive(Debug, Clone)]
pub struct OutputProfile {
    /// EDID identity string this profile applies to ("make model serial").
    /// `None` = applies to any output (single-output era default).
    pub identity: Option<String>,
    pub mode: Option<ModeRequest>,
    /// Whether this monitor is DRIVEN (user active/inactive). `false` = deactivated.
    pub active: bool,
}

impl Default for OutputProfile {
    fn default() -> Self {
        Self { identity: None, mode: None, active: true }
    }
}

/// Whether the monitor keyed by `edid_key` is DRIVEN (active). Unknown / profile-less
/// monitors default to active — so this is behavior-neutral until the user deactivates.
pub fn active(edid_key: &str) -> bool {
    get()
        .iter()
        .find(|p| p.identity.as_deref() == Some(edid_key))
        .map(|p| p.active)
        .unwrap_or(true)
}

/// Load the per-monitor profiles from `preferences.json`, mapped onto the kernel's
/// value type. A missing/invalid file yields an empty vec (default policy), so this
/// is behavior-neutral when the user has set no output preferences.
pub fn get() -> Vec<OutputProfile> {
    compositor_developer_environment_preference_base::base::load()
        .outputs
        .into_iter()
        .map(map_profile)
        .collect()
}

fn map_profile(p: compositor_developer_environment_preference_base::base::OutputProfile) -> OutputProfile {
    OutputProfile { identity: p.identity, mode: p.mode.map(map_mode), active: p.active }
}

/// The hand-set fallback mode for monitors lacking a per-output profile, as an
/// `Advertised` request (mHz refresh, already normalized on load). `None` when
/// unset — callers fall back to the default mode-selection policy.
pub fn default_mode() -> Option<ModeRequest> {
    compositor_developer_environment_preference_base::base::load()
        .outputs_default_mode
        .map(|m| ModeRequest::Advertised {
            width: m.width,
            height: m.height,
            refresh_mhz: m.refresh_mhz,
        })
}

fn map_mode(m: compositor_developer_environment_preference_base::base::ModeRequest) -> ModeRequest {
    use compositor_developer_environment_preference_base::base::ModeRequest as Src;
    match m {
        Src::Advertised { width, height, refresh_mhz } => ModeRequest::Advertised { width, height, refresh_mhz },
        Src::Cvt { width, height, refresh } => ModeRequest::Cvt { width, height, refresh },
        Src::Modeline(s) => ModeRequest::Modeline(s),
    }
}
