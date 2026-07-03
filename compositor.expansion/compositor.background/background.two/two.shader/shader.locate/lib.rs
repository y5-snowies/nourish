//! Locating user shader bundles on disk and enumerating their source formats.
//!
//! A bundle is a directory under `<data>/background/shader/<name>/` containing
//! one or more format subfolders. The active renderer tries the formats in its
//! own preferred order (Vulkan-native first on Vulkan, the raw GLES source on
//! GLES); the loader compiles the first that exists and succeeds, else falls
//! back to the built-in parallax.

use compositor_support_system_persist_path_base::base::data_dir;
use std::path::{Path, PathBuf};

/// A shader source format a bundle may provide.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    /// `vulkan/shader.wgsl` — explicit, hand-tuned Vulkan source (WGSL → SPIR-V).
    VulkanWgsl,
    /// `gles/shader.frag` — explicit ES-1.00 GLES source (runs raw via smithay).
    GlesFrag,
    /// `wgsl/shader.wgsl` — single WGSL source (cross-compiled).
    Wgsl,
    /// `glsl/shader.frag` — single desktop-450-core GLSL source (cross-compiled).
    Glsl,
}

/// `<data>/background/shader` — the root holding user shader bundles.
pub fn background_shader_dir() -> PathBuf {
    data_dir().join("background").join("shader")
}

/// Resolve a selection value to a bundle directory: an absolute path is used
/// verbatim, anything else is a folder name under `background_shader_dir()`.
pub fn resolve_ref(value: &str) -> PathBuf {
    let p = Path::new(value);
    if p.is_absolute() { p.to_path_buf() } else { background_shader_dir().join(value) }
}

/// The available bundle names: every subdirectory of `background_shader_dir()`,
/// sorted. Empty if the directory does not exist yet. Used to populate the
/// settings shader picker.
pub fn list_bundles() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(background_shader_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// The source file for `fmt` inside `bundle`, if it exists on disk.
pub fn source_path(bundle: &Path, fmt: Format) -> Option<PathBuf> {
    let (sub, file) = match fmt {
        Format::VulkanWgsl => ("vulkan", "shader.wgsl"),
        Format::GlesFrag => ("gles", "shader.frag"),
        Format::Wgsl => ("wgsl", "shader.wgsl"),
        Format::Glsl => ("glsl", "shader.frag"),
    };
    let p = bundle.join(sub).join(file);
    p.is_file().then_some(p)
}

/// The format preference order for the active renderer. Vulkan (dmabuf) takes
/// any SPIR-V-capable format; GLES can only run the native ES-1.00 source.
pub fn order(prefers_dmabuf: bool) -> &'static [Format] {
    if prefers_dmabuf {
        &[Format::VulkanWgsl, Format::Wgsl, Format::Glsl]
    } else {
        &[Format::GlesFrag]
    }
}
