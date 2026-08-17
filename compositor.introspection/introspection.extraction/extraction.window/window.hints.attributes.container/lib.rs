//! Container identity hint attributes: which OCI container (podman/docker) a
//! window's process lives inside. Split from `window.hints.attributes.identity`
//! to stay within the per-crate size policy.
pub mod attributes;
