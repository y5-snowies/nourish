use smithay::utils::{Logical, Point, Size};
use uuid::Uuid;
use compositor_support_smithay_state_xdg_activation_dispatch::wire::ActivationDetails;

pub struct WindowData {
    pub UUID: Uuid,
}

/// Present in a toplevel `wl_surface`'s data_map when the selection toolbar's
/// Shift-close asked that this window leave NO placeholder on destroy.
/// Rides the surface (like `WindowData`) instead of the `Placeholder` record so
/// the record stays pure and the mark can never reach persistence.
pub struct DiscardPlaceholder;

/// Set on a window while it is fullscreen. Holds the y5-world geometry to
/// restore to once fullscreen is cleared. Stored in the window's `user_data`
/// inside a `RefCell<Option<..>>` so it can be toggled through a shared ref.
#[derive(Clone, Copy, Debug)]
pub struct WindowFullscreen {
    pub restore_loc: Point<i32, Logical>,
    pub restore_size: Size<i32, Logical>,
}
