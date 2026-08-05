//! # compositor_introspection_restoration_state_base
//!
//! Façade: match newly-appeared windows to pending placeholder
//! restorations. Implementation lives in the sibling `state.*` crates;
//! this crate re-exports the full public tree. No iced, no Smithay, no
//! async, no I/O — pure data + logic.

pub mod matcher {
    pub use compositor_introspection_restoration_state_matcher::matcher::*;
}
pub mod matchers {
    pub use compositor_introspection_restoration_state_matchers::matchers::*;
}
pub mod pending {
    pub use compositor_introspection_restoration_state_pending::pending::*;
}
pub mod registry {
    pub use compositor_introspection_restoration_state_registry::registry::*;
}
pub mod token {
    pub use compositor_introspection_restoration_state_token::token::*;
}

pub use matcher::{MatchResult, RestorationMatcher};
pub use matchers::default_matchers;
pub use pending::{PendingRestoration, SessionKey};
pub use registry::MatcherRegistry;
pub use token::{
    candidate_token_from_env, token_matches, ACTIVATION_TOKEN_ENV, STARTUP_ID_ENV,
};

pub use compositor_introspection_restoration_state_match::match_window;
