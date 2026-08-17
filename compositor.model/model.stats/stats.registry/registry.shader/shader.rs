use std::sync::{OnceLock, RwLock};

fn slot() -> &'static RwLock<Option<String>> {
    static SLOT: OnceLock<RwLock<Option<String>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

/// Set the default background shader (bundle folder name or absolute path) for
/// new worlds. Called at startup from the loaded preference; `None` = built-in.
pub fn set_background_shader_default(value: Option<String>) {
    *slot().write().unwrap_or_else(|e| e.into_inner()) = value;
}

/// The current default background shader selection, if any.
pub fn background_shader_default() -> Option<String> {
    slot().read().unwrap_or_else(|e| e.into_inner()).clone()
}
