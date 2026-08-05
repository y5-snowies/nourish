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
    pub uuid: Uuid,
    pub restoration: Option<PlaceholderLaunchToken>,
    /// See [`Placeholder::session`]. Carried onto the visible tile so the
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
            uuid: self.uuid,
            session: self.session,
        }
    }
}


