use smithay::desktop::Window;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use uuid::Uuid;
use compositor_support_smithay_state_xdg_activation_dispatch::wire::ActivationDetails;
use compositor_support_smithay_dispatch_wire_trait::wire_trait::ActivationOrigin;

pub enum WindowLifecycleEvent {
    InitialMap(Window),
    // Resize(Window),
    /// The `bool` is the surface's `DiscardPlaceholder` mark (Shift-close from the
    /// selection toolbar): destroy must leave no placeholder behind.
    Destroyed(Uuid, Vec<ActivationDetails>, bool),
    /// An X11 window WITHDREW — unmapped, but still alive and able to map again.
    ///
    /// Distinct from [`Self::Destroyed`] because the two differ in what happens to the
    /// window's own record: a destroy MOVES it into the placeholder, a withdrawal COPIES
    /// it and leaves the original in place. The `bool` is the same `DiscardPlaceholder`
    /// verdict.
    Withdrawn(Uuid, bool),
    /// (Un)fullscreen request for a window. `true` = enter fullscreen.
    Fullscreen(Window, bool),
    /// Bring `window` into view (camera `view`) and activate it. Queued by the neutral wire
    /// layer (which can't run the camera logic); `origin` records the source (e.g. a dock via
    /// wlr foreign-toplevel `activate`) for source-specific treatment later.
    Activate(Window, ActivationOrigin),
    /// An `xdg_toplevel_drag_v1` finished carrying this toplevel, so its
    /// placeholder record has to be re-synced to where the carry left it.
    ///
    /// Queued rather than applied directly so it stays ORDERED against the rest:
    /// a tab torn off and dropped inside one frame produces `InitialMap` and this
    /// in the same drain, and the record the settle needs does not exist until
    /// the `InitialMap` ahead of it has been processed.
    DragSettled(WlSurface),
}