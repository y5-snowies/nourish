//! Keyboard-shortcut overrides, persisted to `~/.config/y5.compositor/
//! keybinding.json`. A sparse array: only shortcuts the user re-bound appear;
//! everything else uses its built-in default. Stored as `Key combo` STRINGS
//! (e.g. "Super+Period") — the overlay parses them via `keyboard.format`, so this
//! low-level store stays free of the input-layer types. Reloaded inline (live).
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One shortcut override: the stable action id and its new combo string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyBinding {
    pub action: String,
    pub combo: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyBindings {
    pub bindings: Vec<KeyBinding>,
}

/// One row in the settings Keys tab: stable id, human label, the built-in default
/// combo string, and the effective (override-or-default) combo. Runtime only —
/// built by the overlay's shortcut registry, not persisted.
#[derive(Debug, Clone)]
pub struct KeyRow {
    pub id: String,
    pub label: String,
    pub default: String,
    pub combo: String,
    /// `false` for built-in, non-rebindable shortcuts (e.g. the Super-held canvas
    /// grab tools) — shown read-only at the end of the Keys tab.
    pub editable: bool,
}

impl KeyBindings {
    /// The override combo for `action`, if one is set.
    pub fn combo_for(&self, action: &str) -> Option<&str> {
        self.bindings.iter().find(|b| b.action == action).map(|b| b.combo.as_str())
    }
    /// Set (or replace) the override for `action`.
    pub fn set(&mut self, action: &str, combo: String) {
        if let Some(b) = self.bindings.iter_mut().find(|b| b.action == action) {
            b.combo = combo;
        } else {
            self.bindings.push(KeyBinding { action: action.to_string(), combo });
        }
    }
    /// Remove the override for `action` (revert it to its default).
    pub fn clear(&mut self, action: &str) {
        self.bindings.retain(|b| b.action != action);
    }
}

/// `keybinding.json`, in the same config dir as `settings.json`.
fn path() -> PathBuf {
    compositor_model_environment_config_base::base::resolve_path().with_file_name("keybinding.json")
}

/// Load overrides fresh from disk (missing/invalid → empty: all defaults).
pub fn load() -> KeyBindings {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Persist overrides atomically (temp + rename).
pub fn save(b: &KeyBindings) -> Result<(), String> {
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(b).map_err(|e| format!("serialize keybinding: {e}"))?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &p).map_err(|e| format!("rename {}: {e}", p.display()))?;
    Ok(())
}
