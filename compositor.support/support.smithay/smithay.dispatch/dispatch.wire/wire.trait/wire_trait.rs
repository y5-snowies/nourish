use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::desktop::{Space, Window};
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::dmabuf::{DmabufGlobal, ImportNotifier};
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use compositor_support_smithay_dispatch_state_base::state::Dispatch;
use compositor_support_smithay_state_space_base::state::SpaceState;

/// Where a window-activation request came from. Activation may be treated differently by
/// source; extensible — more origins (xdg-activation, urgency, etc.) will be added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationOrigin {
    /// A dock/taskbar via `zwlr_foreign_toplevel_handle_v1.activate`.
    Foreign,
}

pub trait WireTrait {
    /// Driverdata: the window `Space` hosted by the active spatial world.
    /// Smithay handlers (neutral `Wire`) reach the space ONLY through here —
    /// they must not touch worlds directly; orchestration thin-routes to its
    /// space-host slice (document/ARCHITECTURE.md → "Window tracking").
    fn host_space(&self) -> &SpaceState;
    fn host_space_mut(&mut self) -> &mut SpaceState;
    /// Every SPATIAL world's window Space (skips overlay worlds with no Space). Used by
    /// the foreign-toplevel mirror when `protocol_foreign_all_worlds` is set to advertise
    /// windows from all worlds; otherwise only `host_space` is reconciled.
    fn all_world_spaces(&self) -> Vec<&SpaceState>;
    /// The monitor the user is currently on (cursor's output, else primary), or
    /// `None` if nothing is mapped yet. Used to place a NULL-output layer surface
    /// on the focused monitor instead of always the first one.
    fn active_output(&self) -> Option<smithay::output::Output>;
    fn initialize_surface_data(&mut self, window: Window);
    fn destroy_surface_data(&mut self, surface: ToplevelSurface);
    /// Warp the pointer to a world-space point. The handler reads its own
    /// hosted space internally (it owns it now), so no space is passed in.
    fn apply_pointer(&mut self, storage_point: Point<f64, Logical>);
    fn place_window(&mut self, window: Window, geometry: Rectangle<i32, Logical>);
    /// A client asked to (un)fullscreen `window`. The actual sizing/placement is
    /// deferred to the Loop-level lifecycle hook, since it needs concrete state
    /// (group bounds) unavailable behind the generic `WireTrait` boundary.
    fn fullscreen_request(&mut self, window: Window, fullscreen: bool);
    /// Record a request to activate `window` (bring it into view + focus it). The neutral wire
    /// layer can't run the camera/`view` logic (it lives in a higher crate), so this only
    /// QUEUES the request; a higher-level system drains it and applies the navigator `view`.
    /// `origin` lets the applier treat sources differently (more origins coming).
    fn request_activation(&mut self, window: Window, origin: ActivationOrigin);
    fn dmabuf_import(
        &mut self,
        dispatch: &mut Dispatch,
        _global: &DmabufGlobal,
        _dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) -> Option<(Dmabuf, ImportNotifier)>;
    /// Remember `surface`'s window as the keyboard focus of the world it lives on, so a
    /// later switch back to that world can restore it. No-op if the surface is not a
    /// mapped toplevel (e.g. a layer/iced surface). Called on a world switch with the
    /// still-current (outgoing) focus — see `Wire::apply_world_switch_focus`.
    fn remember_focus_of(&mut self, surface: &WlSurface);
    /// Restore the keyboard focus for the CURRENT (spawn-target) world: activate its
    /// remembered window exclusively and return its surface to focus, or deactivate all
    /// and return `None` when the world has no live remembered window (pruning a stale
    /// entry). The caller applies the returned focus to the seat.
    fn restore_focus_for_current_world(&mut self) -> Option<WlSurface>;
}
