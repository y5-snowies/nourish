//! Experimental GPU/dmabuf-bridge flags, read once from a JSON **array of strings**
//! at `~/.config/y5.compositor/experimental.json` (shared config dir). Each `gpu_*`
//! string maps to one independent bit in [`GpuFlags`]; the flags COMPOSE. Lenient:
//! a missing/invalid file yields no flags (experiments must never crash startup).
//! No logging dep (init runs first); unrecognized flags go to [`unknown`] to warn.

use std::sync::OnceLock;

use bitflags::bitflags;

bitflags! {
    /// OR of every recognized `gpu_*` flag. See the plan for per-flag semantics;
    /// only FORCE_LINEAR vs FORCE_TILED conflict (resolved via [`raw`] order).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct GpuFlags: u16 {
        /// Opt OUT of the now-default modifier negotiation → pre-bridge implicit path.
        const NO_NEGOTIATE_MODIFIERS = 1 << 0;
        const FORCE_LINEAR = 1 << 1;
        const FORCE_TILED = 1 << 2;
        const NEGOTIATE_FORMATS = 1 << 3;
        const ALLOW_DCC = 1 << 4;
        const FORCE_MULTIPLANE = 1 << 5;
        const PROBE_MODIFIERS = 1 << 6;
        /// Opt OUT of the now-default render-node pin → wgpu's default adapter pick.
        const NO_PIN_WGPU_NODE = 1 << 7;
    }
}

fn bit_for(flag: &str) -> Option<GpuFlags> {
    Some(match flag {
        "gpu_no_negotiate_modifiers" => GpuFlags::NO_NEGOTIATE_MODIFIERS,
        "gpu_force_linear" => GpuFlags::FORCE_LINEAR,
        "gpu_force_tiled" => GpuFlags::FORCE_TILED,
        "gpu_negotiate_formats" => GpuFlags::NEGOTIATE_FORMATS,
        "gpu_allow_dcc" => GpuFlags::ALLOW_DCC,
        "gpu_force_multiplane" => GpuFlags::FORCE_MULTIPLANE,
        "gpu_probe_modifiers" => GpuFlags::PROBE_MODIFIERS,
        "gpu_no_pin_wgpu_node" => GpuFlags::NO_PIN_WGPU_NODE,
        _ => return None,
    })
}

struct Experimental {
    flags: GpuFlags,
    raw: Vec<String>,
    unknown: Vec<String>,
}

static EXPERIMENTAL: OnceLock<Experimental> = OnceLock::new();

fn path() -> std::path::PathBuf {
    compositor_developer_environment_config_base::base::resolve_path()
        .with_file_name("experimental.json")
}

/// Fold the ordered raw list into the typed bitflags, collecting unknowns.
fn aggregate(raw: Vec<String>) -> Experimental {
    let mut flags = GpuFlags::empty();
    let mut unknown = Vec::new();
    for f in &raw {
        match bit_for(f) {
            Some(bit) => flags |= bit,
            None => unknown.push(f.clone()),
        }
    }
    if flags.contains(GpuFlags::FORCE_MULTIPLANE) {
        flags |= GpuFlags::ALLOW_DCC; // no meaning without the multi-plane path
    }
    Experimental { flags, raw, unknown }
}

/// Read + aggregate `experimental.json` once, in `main()` after the settings init.
pub fn init() {
    let raw: Vec<String> = std::fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default();
    let _ = EXPERIMENTAL.set(aggregate(raw));
}

/// Lazily default to no experiments if `init()` never ran — never panics.
fn cell() -> &'static Experimental {
    EXPERIMENTAL.get_or_init(|| aggregate(Vec::new()))
}

/// The aggregated experimental bitflags (empty if `init()` never ran).
pub fn get() -> GpuFlags {
    cell().flags
}

/// The raw ordered flag array — for last-wins FORCE_LINEAR/FORCE_TILED resolution.
pub fn raw() -> &'static [String] {
    &cell().raw
}

/// Unrecognized flag strings (warn these at a logging-capable site).
pub fn unknown() -> &'static [String] {
    &cell().unknown
}
