//! [`match_window`]: the pure matching entry point.

use uuid::Uuid;
use compositor_introspection_extraction_window_base::{InferredHints, MetaNode};
use compositor_introspection_extraction_window_hints_codec::codec;
use compositor_introspection_extraction_window_hints_codec_register::register;
use compositor_introspection_launchplan_plan_capture::capture as plan_capture;
use compositor_introspection_restoration_state_handoff::handoff;
use compositor_introspection_restoration_state_matcher::matcher::MatchResult;
use compositor_introspection_restoration_state_pending::pending::{PendingRestoration, SessionKey};
use compositor_introspection_restoration_state_registry::registry::MatcherRegistry;

/// Decide which pending restoration (if any) a newly-appeared window
/// satisfies. Pure function.
///
/// Runs four FIFO sweeps over `pendings`, each pass complete across *all*
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
/// 2. **Handoff pass.** Only if no matcher claimed the window: the launch whose
///    spawned process is already gone, moments after we spawned it, for an app of
///    which this window is one. See [`handoff::claim`] — the case where the app
///    forwarded the launch to an instance that was already running, which every
///    signal above is structurally unable to see.
/// 3. **Transient-capture pass.** Only if nothing above claimed the window: bind
///    the first pending whose capture-armed attributes equal the window's values.
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
/// - `candidate_tokens`: every activation token the surface has been named by
///   via Wayland's xdg-activation protocol (recorded by the compositor from
///   `request_activation`), oldest first. Empty if the surface
///   never carried one.
/// - `candidate_sessions`: every session identity the new window's client has
///   declared via `xdg_session_management_v1`, read off the toplevel's surface
///   data — the name it RESTORED under first, then the name it currently holds.
///   Empty if the client does not speak the protocol (most, today).
///
///   More than one because a client may retire a name between declaring it and
///   mapping the window: Chrome calls `restore_toplevel` with the name the placeholder
///   is filed under, then immediately re-files the same toplevel under a fresh
///   name, all before the buffered commit this match runs on. Matching only the
///   current name misses every such window and mints a duplicate placeholder per run.
/// - `matchers`: the per-handler matcher registry.
pub fn match_window(
    pendings: &[PendingRestoration],
    candidate: &MetaNode,
    candidate_hints: &InferredHints,
    candidate_tokens: &[&str],
    candidate_sessions: &[SessionKey],
    matchers: &MatcherRegistry,
) -> Option<Uuid> {
    // PASS 0 (declared session identity): exact, client-declared, and durable
    // across restarts of both sides. Nothing below can be more certain than
    // this, so it sweeps first and alone.
    //
    // The identity is not unique, though: two placeholders can carry the same key — a
    // session restored twice, or a placeholder duplicated — and taking the first in
    // iteration order made the winner an artefact of list position. Worse, this
    // pass runs BEFORE the token/pid pass, so an arbitrary session hit could beat
    // the activation token of the placeholder the user actually clicked.
    //
    // Tie-broken by the most recent launch instead. Not by "is launching" alone:
    // a placeholder whose launch never produced a window stays in that state, and a
    // stale stuck one must not outrank a fresh click. A candidate with no launch
    // at all loses to any that has one, and only an all-tie falls back to order.
    //
    // Any declared key may hit. They are tried as one set rather than in order:
    // a placeholder is filed under exactly one name, so at most one of the candidate's
    // keys can be the one it holds, and the launch tie-break below stays the
    // only thing that decides between placeholders.
    if !candidate_sessions.is_empty() {
        let claimed = pendings.iter().filter(|pending| {
            pending
                .session
                .as_ref()
                .is_some_and(|held| candidate_sessions.contains(held))
        });
        let best = claimed.max_by(|a, b| match (a.launch_at, b.launch_at) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            // `max_by` keeps the LAST maximum, so report the earlier one as
            // greater to leave the first in order winning an all-tie.
            (None, None) => std::cmp::Ordering::Greater,
        });
        if let Some(pending) = best {
            return Some(pending.id);
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
            if matcher.matches(pending, candidate, candidate_hints, candidate_tokens) == MatchResult::Yes {
                return Some(pending.id);
            }
        }
    }

    // PASS 2 (handoff): the second window of a single-instance app, which no matcher
    // above can see — the process we spawned forwarded the launch and exited, so the
    // window belongs to a process that predates us. Ranked here because it identifies an
    // APPLICATION rather than a window: weaker than anything a matcher proves, stronger
    // than a capture, which claims a window no launch produced at all.
    if let Some(id) = handoff::claim(pendings, candidate, candidate_hints) {
        return Some(id);
    }

    // PASS 3 (transient capture): only once no launch signal claimed the
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
