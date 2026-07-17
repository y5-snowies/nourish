use smithay::desktop::Window;
use uuid::Uuid;
use compositor_support_smithay_state_xdg_activation_dispatch::wire::ActivationDetails;
use compositor_support_smithay_dispatch_wire_trait::wire_trait::ActivationOrigin;

pub enum WindowLifecycleEvent {
    InitialMap(Window),
    // Resize(Window),
    Destroyed(Uuid, Option<ActivationDetails>),
    /// (Un)fullscreen request for a window. `true` = enter fullscreen.
    Fullscreen(Window, bool),
    /// Bring `window` into view (camera `view`) and activate it. Queued by the neutral wire
    /// layer (which can't run the camera logic); `origin` records the source (e.g. a dock via
    /// wlr foreign-toplevel `activate`) for source-specific treatment later.
    Activate(Window, ActivationOrigin),
}