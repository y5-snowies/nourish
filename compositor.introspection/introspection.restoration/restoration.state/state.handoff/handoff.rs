//! The single-instance HANDOFF: the pass that can claim a SECOND window of an app that
//! only ever runs one process.
//!
//! Measured, not assumed. Launch an app that is already running and the process y5 spawned
//! forwards its command line to the running instance and exits; the window is produced by
//! the FIRST process. Every stronger signal is then structurally absent —
//!
//! - the token: the app does not forward `XDG_ACTIVATION_TOKEN` through that channel. It
//!   mints its own with `xdg_activation_v1.get_activation_token` and activates with that,
//!   so the token on the surface is never the one y5 put in the child's environment;
//! - the env: `/proc/<pid>/environ` is the FIRST process's, still holding the FIRST
//!   launch's token;
//! - the pid tree: the window's pid is the first process, which is not a descendant of
//!   the one just spawned;
//! - the session key: `session_id` matches but `name` does not, correctly — it genuinely
//!   is a different toplevel.
//!
//! What remains is the shape of the event: a launch made moments ago whose spawned process
//! is ALREADY GONE, and a window of the same application appearing now.
//!
//! **Not app-specific, which is why it is a pass and not a matcher.** The forwarding
//! itself is near-universal — Chrome and Chromium over their singleton socket, every
//! GApplication (Nautilus and most of GTK) over the session bus, Electron's
//! single-instance lock, Firefox's remote — and it is not even limited to single-instance
//! apps: a `.desktop` `Exec` wrapper that forks the real binary and exits leaves exactly
//! the same evidence. The alternative was to teach each handler's matcher the same rule
//! separately and leave every app without a matcher unable to restore a second window.
//!
//! Two things follow from living here rather than in a matcher. It sees the WHOLE pending
//! list, so it breaks a tie by recency; a matcher is asked about one pending at a time and
//! cannot. And it is reached only after every matcher has declined, which is the right
//! rank: weaker than the token and the pid tree, stronger than a transient capture (which
//! claims windows no launch produced at all).

use uuid::Uuid;
use compositor_introspection_extraction_window_base::attributes::DesktopEntryPath;
use compositor_introspection_extraction_window_base::{InferredHints, MetaNode};
use compositor_introspection_restoration_state_pending::pending::PendingRestoration;

/// How long after a launch a handoff may still be claimed.
///
/// Deliberately short. The window is the gap between the spawned process handing its
/// command line to the running instance and that instance mapping a toplevel — a local
/// socket or bus write and a window, both fast; the measured case was 253ms. It is short
/// because it is the ONLY thing bounding the heuristic: nothing here identifies the window
/// beyond "same application", so two launches of one app inside the same second are
/// interchangeable to it.
const HANDOFF_WINDOW: std::time::Duration = std::time::Duration::from_secs(1);

/// Which pending launch, if any, this window was handed off to.
///
/// Only a placeholder launch can be claimed, and that is structural rather than a check
/// here: a [`PendingRestoration`] is built in exactly one place, immediately after
/// `LaunchPlan::execute_with_env` succeeds, so `launch_at` and `launched_pid` exist only
/// for a placeholder the user clicked. A window that merely appeared is invisible to this
/// pass.
///
/// What it CANNOT claim, by the three conditions: a window of another application; a
/// launch still running — that is the multi-instance case the pid tree already owns, so
/// the two passes are disjoint; and a launch older than [`HANDOFF_WINDOW`]. A launch also
/// claims at most ONE window either way: a matched placeholder leaves `visible` and is
/// never offered again.
///
/// What it CAN still claim wrongly, and the reason it is bounded by time rather than by
/// identity: a window of the same app that appears within the same second for an unrelated
/// reason — Ctrl+N, a background tab opening a window — may take the placeholder instead
/// of the forwarded one. After the handoff there is no information left tying a window to
/// the second launch; every other signal is the first process's. The cost is small in
/// kind: both windows are the same application, so one lands in the placeholder and one
/// opens camera-centred, and only which-is-which differs.
///
/// Requiring the window's process to PREDATE the launch (`/proc/<pid>/stat` starttime
/// against `/proc/uptime`) was considered and rejected: a process started after the launch
/// is one the pid tree already catches, so it rules out nothing this does not.
pub fn claim(
    pendings: &[PendingRestoration],
    candidate: &MetaNode,
    candidate_hints: &InferredHints,
) -> Option<Uuid> {
    pendings
        .iter()
        .filter(|pending| handed_off(pending, candidate, candidate_hints))
        // The most recent launch is the one the user just asked for — the same tie-break
        // the session pass makes, and the reason this is a sweep and not a matcher.
        .max_by_key(|pending| pending.launch_at)
        .map(|pending| pending.id)
}

fn handed_off(
    pending: &PendingRestoration,
    candidate: &MetaNode,
    candidate_hints: &InferredHints,
) -> bool {
    let Some(age) = pending.launch_at.map(|at| at.elapsed()) else {
        return false;
    };
    if age > HANDOFF_WINDOW || pending.launched_pid <= 0 {
        return false;
    }
    // Gone — which includes a ZOMBIE. The forwarding process exits within milliseconds
    // but y5 collects it on its own schedule, so `/proc/<pid>` still
    // exists for a window that overlaps the one this rule lives in. Existence is
    // therefore the wrong test; the process STATE is the right one, and `Z` means it has
    // already exited. A pid recycled inside one second onto a process that also happens
    // to match the app is not a case worth defending against.
    if alive(pending.launched_pid) {
        return false;
    }
    same_application(pending, candidate, candidate_hints)
}

/// Is the window that appeared the same APPLICATION the launch was for?
///
/// `DesktopEntryPath` first, and it is the reason this is not a one-liner on `app_id`: a
/// placeholder's `MetaNode` is deliberately NOT persisted (`PersistedLaunch` stores hints,
/// prefs and the handler — "reconstructable without the process tree / pid"), so a
/// placeholder restored from disk has no `app_id` at all and every comparison against it
/// answers `None`. The desktop entry IS a hint, so it survives the round trip, and it is
/// the more exact identity anyway: two entries for the same binary (a Chrome web app, a
/// second profile) are different applications and should not claim each other.
///
/// `app_id` is the fallback for a placeholder captured this session, whose meta is intact
/// but which may carry no entry — an app started outside any `.desktop` file.
fn same_application(
    pending: &PendingRestoration,
    candidate: &MetaNode,
    candidate_hints: &InferredHints,
) -> bool {
    if let (Some(want), Some(got)) = (
        pending.plan.current::<DesktopEntryPath>(),
        candidate_hints.best_value::<DesktopEntryPath>(),
    ) {
        return want == got;
    }
    match (
        pending.plan.application_data.meta.meta.app_id.as_deref(),
        candidate.meta.app_id.as_deref(),
    ) {
        (Some(a), Some(b)) => !a.is_empty() && a.eq_ignore_ascii_case(b),
        _ => false,
    }
}

/// Is `pid` a process that has NOT yet exited?
///
/// `/proc/<pid>` existing is not the question — a zombie is an exited process whose entry
/// survives until its parent reaps it, and that is precisely the state the spawned
/// forwarder is in during the handoff window. So read the state field: `Z` is exited, a
/// missing entry is exited, anything else is running.
///
/// `/proc/<pid>/stat` field 3, and it is read by index from the LAST `)` rather than by
/// splitting the line: field 2 is the comm, which is parenthesised and may itself contain
/// spaces and brackets.
fn alive(pid: i32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some(after_comm) = stat.rsplit_once(')') else {
        return false;
    };
    !matches!(after_comm.1.split_whitespace().next(), Some("Z") | None)
}
