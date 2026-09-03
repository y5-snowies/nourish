//! [`RestorationMatcher`] trait + [`MatchResult`] enum.

use compositor_introspection_extraction_window_base::{HandlerId, InferredHints, MetaNode};

use compositor_introspection_restoration_state_pending::pending::PendingRestoration;

/// Result of asking a matcher about a (pending, candidate) pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchResult {
    /// Not this restoration.
    No,
    /// Confidently this restoration's window.
    Yes,
}

/// Per-handler restoration matcher.
///
/// Implementors decide whether a newly-appeared window satisfies a
/// pending restoration. Matchers are stateless — all required data is
/// in the arguments. The matcher MAY mix multiple signals:
///
/// - Activation token (via `token_matches`): the strongest signal when
///   present.
/// - PID-tree match against `pending.launched_pid`.
/// - Handler-specific window properties (app_id, exe, hint values).
///
/// Different handlers favor different signals. Single-instance Chrome
/// can't rely on PID tree because the launched PID often exits; for it,
/// activation token is the only reliable signal. Terminals do fine on
/// PID-tree + app_id alone.
pub trait RestorationMatcher: Send + Sync {
    /// Handler this matcher applies to.
    fn handler_id(&self) -> HandlerId;

    /// Whether the candidate satisfies the pending restoration.
    fn matches(
        &self,
        pending: &PendingRestoration,
        candidate: &MetaNode,
        candidate_hints: &InferredHints,
        candidate_tokens: &[&str],
    ) -> MatchResult;
}
