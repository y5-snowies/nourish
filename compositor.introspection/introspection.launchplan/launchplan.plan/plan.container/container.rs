//! Which container a plan targets — the ONE answer, shared by the launch path
//! and the placeholder UI.
//!
//! These were two independent reads of the same attributes, and they drifted
//! into disagreeing about what "has a container" means. Keeping them apart is
//! what let a tile show a container badge while the launch built a broken
//! command; a single query is the fix, not tidier duplicates.

use compositor_introspection_extraction_window_base::attributes::{ContainerId, ContainerName};
use compositor_introspection_launchplan_plan_base::LaunchPlan;

/// The container to launch into: the NAME if one is set, else the ID.
/// `None` means launch on the host — the normal case.
///
/// The name is preferred because a container ID is minted fresh by every
/// `podman rm && podman run`, and a placeholder easily outlives that.
pub fn container_of(plan: &LaunchPlan) -> Option<String> {
    container_name(plan).or_else(|| non_blank(plan.current::<ContainerId>()))
}

/// The container's name alone, for display. `None` when the app is in a
/// container we could not name — the UI says so rather than showing nothing.
pub fn container_name(plan: &LaunchPlan) -> Option<String> {
    non_blank(plan.current::<ContainerName>())
}

/// What to SHOW for the container: its name, else a short id.
///
/// The name needs the container runtime to resolve and that lookup refuses to
/// block extraction, so a placeholder can know exactly WHICH container it is in
/// and still have no name for it — and once the window is gone there is no
/// process left to re-extract from, so the name may never arrive. Falling back
/// to the id is the difference between telling the user which container this is
/// and telling them we have no idea, which is not what we mean.
///
/// Same precedence as [`container_of`], so the label can never name something
/// other than what a launch would target.
pub fn container_label(plan: &LaunchPlan) -> Option<String> {
    container_name(plan).or_else(|| non_blank(plan.current::<ContainerId>()).map(short_id))
}

/// A container id is 64 hex characters; podman prints and accepts the first
/// twelve, so that is the form a user recognises and can paste back.
fn short_id(id: String) -> String {
    match id.char_indices().nth(SHORT_ID_LEN) {
        Some((at, _)) => id[..at].to_string(),
        None => id,
    }
}

const SHORT_ID_LEN: usize = 12;

/// Whether this plan's app runs inside a container at all. True when either
/// identifier survives [`non_blank`], so an unnamed container still counts.
pub fn is_containerised(plan: &LaunchPlan) -> bool {
    container_of(plan).is_some()
}

/// Treat a blank value as absent.
///
/// This is the load-bearing part. `current()` returns the user's OVERRIDE
/// before falling back to the inferred hint, and clearing a text field in the
/// placeholder's settings stores an empty-string override rather than
/// removing one — so the answer is `Some("")`, not `None`. Taken at face
/// value that kept a tile marked as containerised forever (the override is
/// persisted, so it survived restarts), never showed the "name unavailable"
/// state, and put an empty container name into `podman exec`, breaking the
/// launch outright.
///
/// Trimming as well as emptiness-checking, because a field holding only
/// spaces is the same user intent and would fail the same way.
fn non_blank(value: Option<String>) -> Option<String> {
    value.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}
