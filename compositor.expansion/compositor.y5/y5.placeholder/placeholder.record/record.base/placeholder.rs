use std::process::Child;
use std::time::Instant;
use uuid::Uuid;
use compositor_introspection_launchplan_plan_base::LaunchPlan;
use compositor_introspection_restoration_state_pending::pending::SessionKey;

#[derive(Clone, Debug)]
pub struct Placeholder {
    pub position: (i32, i32),
    pub size: (i32, i32),
    pub launch: Option<LaunchPlan>,
    pub launch_session: Option<LaunchPlan>, // <-- Retains session based launc for refresh logic
    pub uuid: Uuid,
    pub session_time: Instant,
    pub persistent: bool,
    /// `xdg_session_management_v1` identity of the window this placeholder
    /// captured, when its client speaks the protocol. Unlike `restoration`
    /// (a per-launch token) this is durable: it is persisted with the
    /// placeholder and the client re-declares the same pair on every later
    /// run, so it identifies the window rather than the launch.
    pub session: Option<SessionKey>,
}

#[derive(Clone, Debug)]
pub struct PlaceholderVisible {
    pub position: (i32, i32),
    pub size: (i32, i32),
    pub launch: LaunchPlan,
    pub launching: bool,
    /// When `launching` was last set. Two placeholders can carry the same session key —
    /// a session restored twice, say — and the matcher has to pick one; the most
    /// recently launched is the one the user just clicked. `launching` alone will
    /// not do, because a placeholder whose launch never produced a window stays stuck in
    /// that state, and a stale stuck placeholder must not outrank a fresh click.
    pub launch_at: Option<Instant>,
    /// Whether that launch had to START this plan's container first.
    ///
    /// Only that case earns the longer grace: `podman start` plus an image pull
    /// plus the container's own init is a different order of magnitude from
    /// `podman exec` into one already up, which is as quick as a host launch.
    /// Recorded rather than re-probed, because answering it again at match time
    /// would mean another blocking `podman ps` on the calloop thread.
    pub launch_started_container: bool,
    pub uuid: Uuid,
    pub restoration: Option<PlaceholderLaunchToken>,
    /// See [`Placeholder::session`]. Carried onto the visible placeholder so the
    /// pending it builds can match on it.
    pub session: Option<SessionKey>,
}

#[derive(Clone, Debug)]
pub struct PlaceholderLaunchToken {
    pub token: String,
    pub child: Option<u32>
}

impl Into<PlaceholderVisible> for Placeholder {
    fn into(self) -> PlaceholderVisible {
        PlaceholderVisible {
            restoration: None,
            size: self.size,
            position: self.position,
            launch: self.launch.unwrap_or_else(|| abort!("launch to exist")),
            launching: false,
            launch_at: None,
            launch_started_container: false,
            uuid: self.uuid,
            session: self.session,
        }
    }
}


