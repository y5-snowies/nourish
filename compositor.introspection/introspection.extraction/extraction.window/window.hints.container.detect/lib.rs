//! Container detection from a captured `/proc` snapshot (cgroup + env). Pure
//! parsing — the runtime lookups that need a subprocess live in the sibling
//! `window.hints.container.query`.
pub mod detect;
