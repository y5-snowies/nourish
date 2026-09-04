//! JetBrains matcher: token, or PID-tree + product match + app_id prefix.

use compositor_introspection_extraction_window_base::handlers::jetbrains::{
    attributes::ProductAttr, id as jetbrains_id,
};
use compositor_introspection_extraction_window_base::{HandlerId, InferredHints, MetaNode};

use compositor_introspection_restoration_state_matcher::matcher::{MatchResult, RestorationMatcher};
use compositor_introspection_restoration_state_matchers_generic::generic::pid_tree_contains;
use compositor_introspection_restoration_state_pending::pending::PendingRestoration;
use compositor_introspection_restoration_state_token::token::token_matches;

pub struct JetBrainsMatcher;

impl RestorationMatcher for JetBrainsMatcher {
    fn handler_id(&self) -> HandlerId {
        jetbrains_id()
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

        // PID matches. Tighten with app_id prefix + product agreement.
        let app_id_ok = candidate
            .meta
            .app_id
            .as_deref()
            .map(|s| s.starts_with("jetbrains-") || s.contains("idea") || s.contains("pycharm"))
            .unwrap_or(false);
        if !app_id_ok {
            return MatchResult::No;
        }

        let plan_product = pending.plan.current::<ProductAttr>();
        let candidate_product = candidate_hints.best_value::<ProductAttr>();
        match (plan_product, candidate_product) {
            (Some(p), Some(c)) if p == c => MatchResult::Yes,
            (None, _) => MatchResult::Yes,
            _ => MatchResult::No,
        }
    }
}
