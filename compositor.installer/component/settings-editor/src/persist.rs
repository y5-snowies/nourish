//! Serialize an `Environment` and write `settings.json` atomically (sibling temp +
//! rename, so a reader never sees a partial file). Shared by the installer flow and
//! the menu's Settings entry. Errors are reported but non-fatal to the menu loop.

use compositor_model_environment_config_base::base::Environment;
use std::io::Write;
use std::path::Path;

/// Write `settings` to `path` atomically, printing the outcome.
pub fn write_settings(path: &Path, settings: &Environment) {
    let json = serde_json::to_string_pretty(settings).expect("Environment serializes to JSON");
    match atomic_write(path, json.as_bytes()) {
        Ok(()) => println!("\nWrote {} ✓", path.display()),
        Err(e) => eprintln!("\nfailed to write {}: {e}", path.display()),
    }
}

/// Create parent dirs, write a sibling temp file, then rename over the target.
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}
