//! Generic fallback matcher: token, then PID-tree.
//!
//! Used as the registry's fallback and for any handler that doesn't
//! register its own matcher. Two signals:
//!
//! 1. Activation-token match via [`token_matches`].
//! 2. PID-tree match: `pending.launched_pid` equals the candidate's PID
//!    or appears in the candidate's parent chain.
//!
//! Either signal is sufficient. Both are checked so a handler that has
//! one but not the other still binds.

use compositor_introspection_extraction_window_base::{handlers::generic::id as generic_id, HandlerId, InferredHints, MetaNode};

use compositor_introspection_restoration_state_matcher::matcher::{MatchResult, RestorationMatcher};
use compositor_introspection_restoration_state_pending::pending::PendingRestoration;
use compositor_introspection_restoration_state_token::token::token_matches;

pub struct GenericMatcher;

impl RestorationMatcher for GenericMatcher {
    fn handler_id(&self) -> HandlerId {
        generic_id()
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

/// Walk the candidate's parent chain. Returns true if `target` matches
/// the candidate's PID or any ancestor PID.
pub fn pid_tree_contains(candidate: &MetaNode, target: i32) -> bool {
    if let Some(pid) = candidate.meta.pid {
        let pid = pid as i32;
        if pid == target {
            return true;
        }
    }

    let mut cursor = candidate.parent.as_deref();
    while let Some(node) = cursor {
        if let Some(pid) = node.meta.pid {
            let pid = pid as i32;
            if pid == target {
                return true;
            }
        }

        cursor = node.parent.as_deref();
    }
    false
}
