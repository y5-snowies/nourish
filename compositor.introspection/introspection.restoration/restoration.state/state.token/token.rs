//! Activation-token comparison helpers shared by matchers.

use compositor_introspection_extraction_window_base::MetaNode;

use compositor_introspection_restoration_state_pending::pending::PendingRestoration;

/// Env var name for the XDG activation token. Set by the compositor on
/// the launched process; carried into the new window's `/proc/<pid>/environ`
/// (allowlisted in the extraction crate's `ENV_ALLOWLIST`).
pub const ACTIVATION_TOKEN_ENV: &str = "XDG_ACTIVATION_TOKEN";

/// Legacy startup-notification env var. Some clients consume
/// `XDG_ACTIVATION_TOKEN` and unset it but leave `DESKTOP_STARTUP_ID`
/// behind, or vice-versa. We check both.
pub const STARTUP_ID_ENV: &str = "DESKTOP_STARTUP_ID";

/// Read the activation token from the candidate's allowlisted env, if
/// present. Tries `XDG_ACTIVATION_TOKEN` first, falls back to
/// `DESKTOP_STARTUP_ID`.
pub fn candidate_token_from_env(candidate: &MetaNode) -> Option<&str> {
    let env = candidate.meta.selected_env.as_ref()?;
    env.get(ACTIVATION_TOKEN_ENV)
        .or_else(|| env.get(STARTUP_ID_ENV))
        .map(String::as_str)
}

/// True if any activation token the candidate is known by (from surface
/// data OR env) matches the pending restoration's stored token.
///
/// Three sources are checked in order:
/// 1. `candidate_tokens` — EVERY token the surface has been named by, via
///    `request_activation`. This is the protocol-level
///    signal, and the only one that can work for a single-instance app:
///    its launched process exits immediately and the window belongs to an
///    already-running instance whose environment holds an older token, so
///    sources 2 and 3 are both stale there.
/// 2. `XDG_ACTIVATION_TOKEN` in the candidate's allowlisted env. The
///    last-resort signal for clients that received the token but never
///    called `xdg_activation_v1.activate`.
/// 3. `DESKTOP_STARTUP_ID` in the candidate's allowlisted env. Same
///    role for legacy clients (and apps that strip one but not the other).
///
/// The pending's stored token is checked under both env-var names so
/// either side of the asymmetry works.
pub fn token_matches(
    pending: &PendingRestoration,
    candidate: &MetaNode,
    candidate_tokens: &[&str],
) -> bool {
    // What tokens does the pending know about? Usually the same value
    // is set under both var names, but allow either.
    let pending_xdg = pending.activation_env.get(ACTIVATION_TOKEN_ENV);
    let pending_startup = pending.activation_env.get(STARTUP_ID_ENV);

    let pending_tokens: [Option<&String>; 2] = [pending_xdg, pending_startup];
    let pending_tokens: Vec<&str> = pending_tokens
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect();
    if pending_tokens.is_empty() {
        return false;
    }

    // Source 1: surface-data tokens.
    if candidate_tokens.iter().any(|t| pending_tokens.contains(t)) {
        return true;
    }

    // Sources 2 & 3: candidate env.
    if let Some(env) = candidate.meta.selected_env.as_ref() {
        if let Some(t) = env.get(ACTIVATION_TOKEN_ENV) {
            if pending_tokens.iter().any(|p| *p == t.as_str()) {
                return true;
            }
        }
        if let Some(t) = env.get(STARTUP_ID_ENV) {
            if pending_tokens.iter().any(|p| *p == t.as_str()) {
                return true;
            }
        }
    }

    false
}
