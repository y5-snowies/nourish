//! Preset-independent config file text generators: xdg-portal preference, PAM
//! lock policy, and the polkit agent service.

pub mod policy;
pub use policy::*;
