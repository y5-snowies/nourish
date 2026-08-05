//! [`match_window`]: the pure matching entry point.

use uuid::Uuid;
use compositor_introspection_extraction_window_base::{InferredHints, MetaNode};
use compositor_introspection_extraction_window_hints_codec::codec;
use compositor_introspection_extraction_window_hints_codec_register::register;
use compositor_introspection_launchplan_plan_capture::capture as plan_capture;
use compositor_introspection_restoration_state_matcher::matcher::MatchResult;
use compositor_introspection_restoration_state_pending::pending::{PendingRestoration, SessionKey};
use compositor_introspection_restoration_state_registry::registry::MatcherRegistry;

/// Decide which pending restoration (if any) a newly-appeared window
/// satisfies. Pure function.
///
/// Runs three FIFO sweeps over `pendings`, each pass complete across *all*
/// pendings before the next begins:
///
/// 0. **Session pass.** For each pending, exact equality against the session
///    identity the window's client declared through
///    `xdg_session_management_v1`. First because it is the only signal that is
///    *declared* rather than inferred: the client re-states it before its
///    first commit on every run, so a hit here is not a heuristic. It also
///    means the window may not have been launched by us at all — the other two
///    passes can only ever claim windows a placeholder spawned.
/// 1. **Explicit-launch pass.** For each pending, dispatch to the matcher
///    registered for `pending.plan.active_handler` (falling back to the
///    registry's generic matcher) and bind the first
///    [`MatchResult::Yes`].
/// 2. **Transient-capture pass.** Only if no explicit-launch signal claimed
///    the window: bind the first pending whose capture-armed attributes
///    equal the window's values.
///
/// The passes are kept fully separate — not interleaved per pending — so an
/// explicit-launch signal on *any* placeholder always outranks a transient
/// capture on an earlier one in the list. Interleaving would let a
/// capture-armed placeholder near the front of the FIFO adopt a window that
/// an explicit launch elsewhere should have claimed.
///
/// Inputs:
/// - `pendings`: in-flight launches the compositor is waiting on.
/// - `candidate`: the new window's captured metadata.
/// - `candidate_hints`: hints inferred for the new window (matchers may
///   use them for handler-specific signals like terminal kind).
/// - `candidate_token`: the activation token the surface received via
///   Wayland's xdg-activation protocol (set by the compositor from
///   `request_activation`). `None` if the surface didn't carry one.
/// - `candidate_session`: the session identity the new window's client
///   declared via `xdg_session_management_v1`, read off the toplevel's surface
///   data. `None` if the client does not speak the protocol (most, today).
/// - `matchers`: the per-handler matcher registry.
pub fn match_window(
    pendings: &[PendingRestoration],
    candidate: &MetaNode,
    candidate_hints: &InferredHints,
    candidate_token: Option<&str>,
    candidate_session: Option<&SessionKey>,
    matchers: &MatcherRegistry,
) -> Option<Uuid> {
    // PASS 0 (declared session identity): exact, client-declared, and durable
    // across restarts of both sides. Nothing below can be more certain than
    // this, so it sweeps first and alone.
    if let Some(candidate_session) = candidate_session {
        for pending in pendings {
            if pending.session.as_ref() == Some(candidate_session) {
                return Some(pending.id);
            }
        }
    }

    // PASS 1 (explicit launch): activation token / PID tree take precedence
    // across the whole list — a handler matcher claim on any pending beats a
    // transient capture on an earlier one.
    for pending in pendings {
        let matcher = pending
            .plan
            .active_handler
            .and_then(|id| matchers.get(id))
            .or_else(|| matchers.fallback());

        if let Some(matcher) = matcher {
            if matcher.matches(pending, candidate, candidate_hints, candidate_token) == MatchResult::Yes {
                return Some(pending.id);
            }
        }
    }

    // PASS 2 (transient capture): only once no explicit launch claimed the
    // window does a placeholder with capture-armed attributes adopt a window
    // whose values match, even though no Launch spawned it.
    for pending in pendings {
        if capture_matches(pending, candidate_hints) {
            return Some(pending.id);
        }
    }

    None
}

/// Transient-capture predicate: every attribute the placeholder armed for
/// capture must exactly equal the new window's value. Empty capture set =>
/// never matches (the placeholder only restores via an explicit Launch).
///
/// Values are type-erased (`Arc<dyn Any>`), so we compare their codec-encoded
/// JSON — the same encoding persistence uses — which gives value equality
/// without per-type downcasts. A missing value on either side, or an
/// unregistered codec, fails the match (exact equality requires both present).
fn capture_matches(pending: &PendingRestoration, candidate_hints: &InferredHints) -> bool {
    let keys = plan_capture::capture_keys(&pending.plan);
    if keys.is_empty() {
        return false;
    }
    register::register_standard_codecs();
    for key in keys {
        let (Some(stored), Some(live)) =
            (plan_capture::current_raw_by_key(&pending.plan, key), candidate_hints.best_raw(key))
        else {
            return false;
        };
        let stored_json = codec::encode(key, &stored);
        if stored_json.is_none() || stored_json != codec::encode(key, &live) {
            return false;
        }
    }
    true
}
