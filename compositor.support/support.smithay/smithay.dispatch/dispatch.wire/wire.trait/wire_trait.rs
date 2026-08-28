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
    /// The Space of the world that OWNS `surface`'s window (root resolved through the
    /// subsurface tree), or `host_space` when no world maps it yet — a commit belongs
    /// to the window's world, which is not necessarily the one the user is in.
    fn owning_space(&self, surface: &WlSurface) -> &SpaceState;
    fn owning_space_mut(&mut self, surface: &WlSurface) -> &mut SpaceState;
    /// Every SPATIAL world's window Space (skips overlay worlds with no Space). Used by
    /// the foreign-toplevel mirror when `protocol_foreign_all_worlds` is set to advertise
    /// windows from all worlds; otherwise only `host_space` is reconciled.
    fn all_world_spaces(&self) -> Vec<&SpaceState>;
    /// The monitor the user is currently on (cursor's output, else primary), or
    /// `None` if nothing is mapped yet. Used to place a NULL-output layer surface
    /// on the focused monitor instead of always the first one.
    fn active_output(&self) -> Option<smithay::output::Output>;
    fn initialize_surface_data(&mut self, window: Window);
    /// The size the placeholder bound to this surface's SESSION remembers for it.
    ///
    /// Answers before the INITIAL configure, which is the point: a returning window
    /// is otherwise told `0x0` ("you pick"), lays out at its own default, and is
    /// corrected only once the placeholder match runs after its first buffer — the visible
    /// two-step resize on every restore. `restore_toplevel` must precede the first
    /// commit, so the identity is already on the surface and the answer is knowable.
    ///
    /// Searched across every world (a session is bound to a placeholder, whose world
    /// owns the window). NOT gated on the transient-capture preference: that governs
    /// whether a placeholder may CLAIM a window it did not launch.
    fn session_restore_size(
        &self,
        surface: &WlSurface,
    ) -> Option<smithay::utils::Size<i32, Logical>>;
    /// `drag_discard`: the toplevel was mid-flight in an `xdg_toplevel_drag_v1`
    /// when it was destroyed, i.e. some other toplevel adopted the tab and the
    /// carrier was thrown away. Nothing was closed, so it leaves no placeholder.
    /// Sampled in the wire layer (which owns the drag registry) at destroy time,
    /// because the placeholder policy lives above this trait and the drain runs
    /// later than the answer is valid for.
    fn destroy_surface_data(&mut self, surface: ToplevelSurface, drag_discard: bool);
    /// Warp the pointer to a world-space point. The handler reads its own
    /// hosted space internally (it owns it now), so no space is passed in.
    fn apply_pointer(&mut self, storage_point: Point<f64, Logical>);
    /// Where a surface-local point of `surface` (root or subsurface) is displayed in the
    /// world, through the window's fit (world side). `None` when no window owns it.
    fn surface_point_to_world(&self, surface: &WlSurface, local: Point<f64, Logical>) -> Option<Point<f64, Logical>>;
    /// The inverse of [`apply_pointer`]: pin the cursor's own hardware position into
    /// the output and return where it lands in the FOCUSED world, so the caller can
    /// re-state the seat's location there. Re-seats the camera's pan accumulator on
    /// the same point, exactly as `apply_pointer` does.
    ///
    /// Used after a world switch: the seat holds one global world coordinate while
    /// every world has its own camera, so the location carried across a switch lands
    /// wherever the incoming camera projects it. The hardware position is the value
    /// that survives — re-derive the world point from it.
    fn reanchor_pointer(&mut self) -> Point<f64, Logical>;
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
    /// An `xdg_toplevel_drag_v1` just stopped carrying `surface`. Queues the
    /// placeholder re-sync; like `request_activation` this only RECORDS, because
    /// the placeholder model lives above this trait.
    fn settle_toplevel_drag(&mut self, surface: WlSurface);
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
