use std::process::Child;
use std::time::Instant;
use uuid::Uuid;
use compositor_introspection_launchplan_plan_base::LaunchPlan;
use compositor_introspection_restoration_state_pending::pending::SessionKey;
use compositor_introspection_extraction_window_base::IconPixels;

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
    /// The captured window was an X11 (XWayland) client.
    ///
    /// Recorded at map, because it cannot be recovered at restore: what comes back
    /// is a launch plan and a pid, neither of which says which protocol the window
    /// spoke. Purely a display fact — it tints the placeholder panel — but it is a
    /// useful one, because an X11 placeholder restores through a different route
    /// (`DESKTOP_STARTUP_ID` in the child's environment rather than a declared
    /// wayland identity) and is the one to look at first when a restore misses.
    pub from_x11: bool,
    /// The icon the captured window committed for ITSELF, as decoded pixels.
    ///
    /// The hint path cannot carry this. `push_toplevel_icon_hints` starts with
    /// `let Some(name) = &icon.name else { return }`, and a pixel-only icon has no name
    /// — which `_NET_WM_ICON` never does (it is pixels by definition) and an
    /// `xdg_toplevel_icon_v1` client that sends buffers without `set_name` also does
    /// not. So every such window fell through to its desktop entry, and one with no
    /// entry — a Proton game, most of all — showed nothing at all, while the overview
    /// showed its icon perfectly from the same source.
    ///
    /// IN-SESSION ONLY, and deliberately not persisted: a placeholder record outlives
    /// its window, and decoded pixels are unbounded client state with no name to
    /// re-resolve from. A restored placeholder falls back to the entry, which is what
    /// it did before. Bounded meanwhile by the decoder's own `PREFERRED_EDGE`/`MAX_EDGE`
    /// and by one image per placeholder.
    pub icon_pixels: Option<IconPixels>,
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
    /// See [`Placeholder::from_x11`]. Carried across so the visible placeholder's
    /// surface can tint itself.
    pub from_x11: bool,
    /// See [`Placeholder::icon_pixels`]. Carried across so the visible placeholder
    /// draws the icon its window committed.
    pub icon_pixels: Option<IconPixels>,
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
            icon_pixels: self.icon_pixels,
            from_x11: self.from_x11,
        }
    }
}


