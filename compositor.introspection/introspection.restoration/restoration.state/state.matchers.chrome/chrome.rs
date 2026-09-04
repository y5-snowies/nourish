//! Chrome matcher.
//!
//! Chrome single-instance behavior often means `launched_pid` exits
//! almost immediately and the new window belongs to an unrelated
//! already-running Chrome process. PID-tree matching is unreliable.
//!
//! Primary signal: activation token.
//! Secondary signal: PID-tree match (covers the multi-instance Chrome
//! case where each launch does spawn its own process tree).
//! No exe-name-only fallback — too false-positive-prone (it'd bind
//! any Chrome window to any pending Chrome restoration).
//!
//! The single-instance case neither can reach — the second window, produced by the
//! FIRST process — is not handled here. It is not Chrome-specific, so it is a pass in
//! `state.handoff` that runs once every matcher has declined.

use compositor_introspection_extraction_window_base::handlers::chrome::id as chrome_id;
use compositor_introspection_extraction_window_base::{HandlerId, InferredHints, MetaNode};

use compositor_introspection_restoration_state_matcher::matcher::{MatchResult, RestorationMatcher};
use compositor_introspection_restoration_state_matchers_generic::generic::pid_tree_contains;
use compositor_introspection_restoration_state_pending::pending::PendingRestoration;
use compositor_introspection_restoration_state_token::token::token_matches;

pub struct ChromeMatcher;

impl RestorationMatcher for ChromeMatcher {
    fn handler_id(&self) -> HandlerId {
        chrome_id()
    }

    fn matches(
        &self,
        pending: &PendingRestoration,
        candidate: &MetaNode,
        _candidate_hints: &InferredHints,
        candidate_tokens: &[&str],
    ) -> MatchResult {
        if token_matches(pending, candidate, candidate_tokens) {
            return MatchResult::Yes;
        }
        if pid_tree_contains(candidate, pending.launched_pid) {
            return MatchResult::Yes;
        }
        MatchResult::No
    }
}
