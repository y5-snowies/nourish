//! The saved cursor state a touch sequence displaces.
//!
//! Split from the restore behaviour (`pointer.restore`) so the state root can hold
//! the value without depending on a crate that itself depends on the state root.

use compositor_orchestration_driver_output_base::base::OutputKey;
use smithay::utils::{Logical, Point};

/// Snapshot of the shared pointer cursor, taken when a touch sequence takes over the
/// single active output and restored when a pointer/pen event next arrives.
///
/// y5 has ONE seat pointer shared across every output, but a touch must act on the
/// panel it physically happened on — so the touch path re-pins `cursor_output` and
/// the active output view to that panel. Without this snapshot, touching a secondary
/// touchscreen would permanently teleport the mouse cursor there.
///
/// `Option<CursorSnapshot>` therefore does double duty on the Orchestrator: it is
/// both the saved value AND the "a touch owns the cursor, so don't draw it" flag.
pub struct CursorSnapshot {
    /// The pointer motion accumulator (`pointer().motion`) at snapshot time.
    ///
    /// NB: this holds PHYSICAL pixels despite the `Logical` type parameter — it
    /// mirrors `PointerState::motion`, which is mislabelled the same way (see
    /// `pointer.pan`, which writes a `Point<f64, Physical>` into it). The type is
    /// kept identical to the field it snapshots rather than "fixed" here, so the two
    /// cannot silently drift apart; correcting it belongs with `PointerState`.
    pub motion: Point<f64, Logical>,
    /// The active output the cursor was on. Re-validated against the live output set
    /// on restore — a monitor can be unplugged mid-touch.
    pub output: Option<OutputKey>,
    /// The seat pointer's world location.
    pub location: Point<f64, Logical>,
}
