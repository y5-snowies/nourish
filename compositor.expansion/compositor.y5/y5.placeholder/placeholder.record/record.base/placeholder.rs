use std::process::Child;
use std::time::Instant;
use uuid::Uuid;
use compositor_introspection_launchplan_plan_base::LaunchPlan;

#[derive(Clone, Debug)]
pub struct Placeholder {
    pub position: (i32, i32),
    pub size: (i32, i32),
    pub launch: Option<LaunchPlan>,
    pub launch_session: Option<LaunchPlan>, // <-- Retains session based launc for refresh logic
    pub uuid: Uuid,
    pub session_time: Instant,
    pub persistent: bool,
    /// Marked by the selection toolbar's Shift-close: when this window is
    /// destroyed, do NOT leave a visible placeholder tile behind.
    pub discard_on_close: bool,
}

#[derive(Clone, Debug)]
pub struct PlaceholderVisible {
    pub position: (i32, i32),
    pub size: (i32, i32),
    pub launch: LaunchPlan,
    pub launching: bool,
    pub uuid: Uuid,
    pub restoration: Option<PlaceholderLaunchToken>
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
            uuid: self.uuid
        }
    }
}


