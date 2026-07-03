use std::path::PathBuf;
use uuid::Uuid;

/// The root of all persisted state: the general XDG state dir `$XDG_STATE_HOME/y5`,
/// falling back to `$HOME/.local/state/y5`, then `/tmp/y5`. Never panics.
pub fn state_dir() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| !p.as_os_str().is_empty())
                .map(|home| home.join(".local").join("state"))
        })
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("y5")
}

/// The root for user-supplied data assets (e.g. background shaders): the XDG data
/// dir `$XDG_DATA_HOME/y5`, falling back to `$HOME/.local/share/y5`, then `/tmp/y5`.
/// Never panics. Distinct from `state_dir()` (which is `~/.local/state`).
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| !p.as_os_str().is_empty())
                .map(|home| home.join(".local").join("share"))
        })
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("y5")
}

/// A table directory `<state>/<table>` for the document store (e.g. `"world"`,
/// `"world.placeholder"`).
pub fn table_dir(table: &str) -> PathBuf {
    state_dir().join(table)
}

/// The directory holding one world's persisted slots, keyed by world UUID.
pub fn world_dir(world: Uuid) -> PathBuf {
    state_dir().join(world.to_string())
}

/// The file for one (world, key): `<state>/<world_uuid>/<key>.json`.
pub fn file_path(world: Uuid, key: &str) -> PathBuf {
    world_dir(world).join(format!("{key}.json"))
}

/// Where a corrupt/unmigratable file is moved so it stops tripping load and is
/// preserved for debugging.
pub fn quarantine_path(world: Uuid, key: &str, ts: u64) -> PathBuf {
    world_dir(world).join(format!("{key}.json.corrupt.{ts}"))
}
