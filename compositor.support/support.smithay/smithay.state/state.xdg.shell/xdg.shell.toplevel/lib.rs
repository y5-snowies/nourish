// `new_toplevel` / `destroy_toplevel` were removed: toplevel create/destroy now
// records into the Dispatch outbox (handler) and applies at drain
// (document/SMITHAY_DECOUPLING.md), so support.smithay no longer touches the
// world here.

pub mod toplevel;
pub use toplevel::*;
