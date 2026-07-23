//! The `.y5` package format for the 1c native path: a ZIP holding a JSON manifest
//! (read without extracting the payload → fast import-time compat checks) and the
//! plugin `.so`. See document/EXTENSIONS.md §6. This is the subset of the manifest
//! sufficient for a trusted native-stable plugin; the wasm / process / scripting
//! kinds extend `Manifest.kind` + `compat`.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Path of the manifest inside the `.y5` ZIP.
pub const MANIFEST_PATH: &str = "y5.manifest.json";
/// Path of the plugin shared object inside the `.y5` ZIP.
pub const PAYLOAD_SO: &str = "payload/plugin.so";
/// The host's current 1c contract version (bump on any `abi` boundary change).
/// v2: root-module export + `AbiQuad.band` (plugin-selected compositing band).
pub const HOST_Y5_API: &str = "2";
/// Every contract the host can load — the versions whose adapter chain to current
/// is complete (each historical version lives in its own `plugin.vN/` layer). A
/// manifest outside this set is refused at import, before any code is mapped.
pub const SUPPORTED_Y5_APIS: &[&str] = &["1", "2"];
/// The `abi_stable` major line the host links; a plugin MUST match it.
pub const HOST_ABI_STABLE: &str = "0.11";

#[derive(thiserror::Error, Debug)]
pub enum PackError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("incompatible: {0}")]
    Incompatible(String),
}

/// Compatibility fingerprints, stamped at pack time, re-checked at import time.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Compat {
    /// Host protocol/contract version the plugin was built against.
    pub y5_api: String,
    /// Shared `abi_stable` major line (the real cross-toolchain contract).
    pub abi_stable: String,
    /// Target triple; the host gates on its arch prefix.
    pub target_triple: String,
}

/// The `.y5` manifest (native-stable subset).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Manifest {
    pub schema: u32,
    /// Stable, immutable identity (reverse-DNS).
    pub id: String,
    pub name: String,
    pub version: String,
    /// Delivery tier; only `"native-stable"` is handled here.
    pub kind: String,
    pub compat: Compat,
    /// Exported constructor symbol (see `abi::PLUGIN_NEW`).
    pub entry: String,
}

/// Write a `.y5` (ZIP: manifest + the plugin `.so`).
pub fn write_y5(out: &Path, manifest: &Manifest, so_bytes: &[u8]) -> Result<(), PackError> {
    let file = std::fs::File::create(out)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::FileOptions<()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(MANIFEST_PATH, opts)?;
    zip.write_all(serde_json::to_string_pretty(manifest)?.as_bytes())?;
    zip.start_file(PAYLOAD_SO, opts)?;
    zip.write_all(so_bytes)?;
    zip.finish()?;
    Ok(())
}

/// Read only the manifest from a `.y5` (no payload extraction).
pub fn read_manifest(y5: &Path) -> Result<Manifest, PackError> {
    let file = std::fs::File::open(y5)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let mut entry = zip.by_name(MANIFEST_PATH)?;
    let mut s = String::new();
    entry.read_to_string(&mut s)?;
    Ok(serde_json::from_str(&s)?)
}

/// Gate a manifest against the host contract (the import-time refusal, not UB).
pub fn check_compat(m: &Manifest) -> Result<(), PackError> {
    if m.kind != "native-stable" {
        return Err(PackError::Incompatible(format!("kind `{}` not native-stable", m.kind)));
    }
    if !SUPPORTED_Y5_APIS.contains(&m.compat.y5_api.as_str()) {
        return Err(PackError::Incompatible(format!(
            "y5_api {} not in supported set {:?}",
            m.compat.y5_api, SUPPORTED_Y5_APIS
        )));
    }
    if m.compat.abi_stable != HOST_ABI_STABLE {
        return Err(PackError::Incompatible(format!("abi_stable {} != host {}", m.compat.abi_stable, HOST_ABI_STABLE)));
    }
    if !m.compat.target_triple.starts_with(std::env::consts::ARCH) {
        return Err(PackError::Incompatible(format!("target {} != host arch {}", m.compat.target_triple, std::env::consts::ARCH)));
    }
    Ok(())
}

/// Validate a `.y5` and extract its plugin `.so` to `dest_dir`, returning the path
/// AND the manifest (the loader routes by `compat.y5_api`).
pub fn extract_so(y5: &Path, dest_dir: &Path) -> Result<(PathBuf, Manifest), PackError> {
    let m = read_manifest(y5)?;
    check_compat(&m)?;
    let file = std::fs::File::open(y5)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let mut so = zip.by_name(PAYLOAD_SO)?;
    std::fs::create_dir_all(dest_dir)?;
    let out = dest_dir.join(format!("{}.so", m.id.replace('/', "_")));
    let mut f = std::fs::File::create(&out)?;
    std::io::copy(&mut so, &mut f)?;
    Ok((out, m))
}
