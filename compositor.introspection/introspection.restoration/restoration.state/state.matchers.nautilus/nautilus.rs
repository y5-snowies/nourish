//! Nautilus matcher: token, or PID-tree + app_id match.

use compositor_introspection_extraction_window_base::handlers::nautilus::id as nautilus_id;
use compositor_introspection_extraction_window_base::{HandlerId, InferredHints, MetaNode};

use compositor_introspection_restoration_state_matcher::matcher::{MatchResult, RestorationMatcher};
use compositor_introspection_restoration_state_matchers_generic::generic::pid_tree_contains;
use compositor_introspection_restoration_state_pending::pending::PendingRestoration;
use compositor_introspection_restoration_state_token::token::token_matches;

pub struct NautilusMatcher;

impl RestorationMatcher for NautilusMatcher {
    fn handler_id(&self) -> HandlerId {
        nautilus_id()
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
        if !pid_tree_contains(candidate, pending.launched_pid) {
            return MatchResult::No;
        }
        let app_id_ok = candidate.meta.app_id.as_deref() == Some("org.gnome.Nautilus");
        if app_id_ok {
            MatchResult::Yes
        } else {
            MatchResult::No
        }
    }
}
