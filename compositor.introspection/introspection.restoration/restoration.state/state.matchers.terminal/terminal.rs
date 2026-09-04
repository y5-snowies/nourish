//! Terminal matcher: token, or PID-tree + terminal-kind match.

use compositor_introspection_extraction_window_base::handlers::terminal::{
    attributes::TerminalKindAttr, id as terminal_id,
};
use compositor_introspection_extraction_window_base::{HandlerId, InferredHints, MetaNode};

use compositor_introspection_restoration_state_matcher::matcher::{MatchResult, RestorationMatcher};
use compositor_introspection_restoration_state_matchers_generic::generic::pid_tree_contains;
use compositor_introspection_restoration_state_pending::pending::PendingRestoration;
use compositor_introspection_restoration_state_token::token::token_matches;

pub struct TerminalMatcher;

impl RestorationMatcher for TerminalMatcher {
    fn handler_id(&self) -> HandlerId {
        terminal_id()
    }

    fn matches(
        &self,
        pending: &PendingRestoration,
        candidate: &MetaNode,
        candidate_hints: &InferredHints,
        candidate_tokens: &[&str],
    ) -> MatchResult {
        if token_matches(pending, candidate, candidate_tokens) {
            return MatchResult::Yes;
        }

        if !pid_tree_contains(candidate, pending.launched_pid) {
            return MatchResult::No;
        }

        // PID matches; require the new window's inferred terminal kind to
        // agree with the plan's terminal kind. This rejects unrelated
        // windows that happen to share a process tree.
        let plan_kind = pending.plan.current::<TerminalKindAttr>();
        let candidate_kind = candidate_hints.best_value::<TerminalKindAttr>();
        match (plan_kind, candidate_kind) {
            (Some(p), Some(c)) if p == c => MatchResult::Yes,
            // If the plan doesn't pin a terminal kind, accept the PID match.
            (None, _) => MatchResult::Yes,
            _ => MatchResult::No,
        }
    }
}
