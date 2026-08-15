//! Locating user shader bundles on disk and enumerating their source formats.
//!
//! A bundle is a directory under `<data>/background/shader/<name>/` containing
//! one or more format subfolders. The active renderer tries the formats in its
//! own preferred order (Vulkan-native first on Vulkan, the raw GLES source on
//! GLES); the loader compiles the first that exists and succeeds, else falls
//! back to the built-in parallax.

pub mod base;
pub use base::*;
