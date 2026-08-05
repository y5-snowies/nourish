//! [`PendingRestoration`]: one in-flight launch.

use std::collections::HashMap;

use uuid::Uuid;
use compositor_introspection_launchplan_plan_base::LaunchPlan;

/// A toplevel's identity under `xdg_session_management_v1`: the session id the
/// compositor minted, plus the client's own name for the window inside it.
///
/// Unlike the activation token this is not tied to a launch — the client
/// re-declares it on every run, before its first commit, whether or not we
/// were the one who started it. Plain strings so this crate stays free of any
/// wayland dependency; the y5 layer converts from the surface-data form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionKey {
    pub session_id: String,
    pub name: String,
}

/// One in-flight placeholder launch awaiting the new window.
///
/// The compositor builds this immediately after [`LaunchPlan::execute_with_env`]
/// succeeds and keeps it in a list until `match_window` pairs it with a
/// newly-appeared window.
///
/// All matching is done off this data; no methods are called on the
/// `Child` (the compositor manages that separately).
#[derive(Debug, Clone)]
pub struct PendingRestoration {
    /// Stable identifier — typically the placeholder's id. Returned by
    /// `match_window` so the compositor can look up its own placeholder
    /// struct.
    pub id: Uuid,

    /// The plan that was executed. Matchers read it for handler-specific
    /// expectations (terminal kind, JetBrains product, etc.).
    pub plan: LaunchPlan,

    /// PID returned by `Child::id()` immediately after `spawn`. Used by
    /// matchers as the PID-tree match target. The PID may exit before a
    /// window appears (single-instance Chrome, etc.), so matchers must
    /// not assume the process is still alive.
    pub launched_pid: i32,

    /// Env vars set on the launched process. The compositor populates
    /// this with the values it actually passed (e.g., `XDG_ACTIVATION_TOKEN`,
    /// `DESKTOP_STARTUP_ID`) so matchers can compare against the new
    /// window's env.
    pub activation_env: HashMap<String, String>,

    /// The session identity this placeholder bound to on a previous run, if
    /// its client speaks `xdg_session_management_v1`. Unlike the other fields
    /// this survives a reboot — it is persisted with the placeholder — which
    /// is why it outranks them in `match_window`.
    pub session: Option<SessionKey>,
}
