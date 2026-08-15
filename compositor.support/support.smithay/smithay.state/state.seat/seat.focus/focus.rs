use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};

// `focus_changed` moved INLINE into `impl SeatHandler for Dispatch`
// (state.base), as part of the P2 flip (document/SMITHAY_DECOUPLING.md): the
// `Seat<Dispatch>` field requires the SeatHandler impl at the struct
// definition, so the focus logic lives there and the deferred
// `set_data_device_focus` is recorded into `pending_data_focus`. This crate now
// only carries the shared restoration-token type.
pub type RestorationToken = (WlSurface, Point<f64, Logical>);
