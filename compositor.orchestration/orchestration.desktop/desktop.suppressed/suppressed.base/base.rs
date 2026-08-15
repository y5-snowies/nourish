//! Is the desktop suppressed — i.e. is something OTHER than the focused world's
//! canvas what the user is currently pointing at?
//!
//! Three states put another surface on screen entirely:
//!
//! * the **world picker** — its own overlay world, with its own background;
//! * the **lock** screen — likewise;
//! * the **overview** — the grid is drawn over a frozen backdrop capture, so the
//!   live bundle is not what the user is pointing at.
//!
//! Its own crate because it is not one feature's question. It began as the
//! pointer-warp gate (correcting the pointer for a displacement nobody drew is
//! worse than not correcting it), then the screen-extent policy needed the same
//! answer (a push into an edge belongs to the camera only while the camera's
//! world is the thing on screen), and the cursor sprite reads it through the
//! warp. Every reader must get the SAME answer — a frame where motion warps and
//! the sprite does not is a cursor that renders away from where it clicks — and
//! the way to guarantee that is one definition nobody owns, rather than a
//! predicate living inside whichever feature happened to need it first.
//!
//! Hit testing deliberately does NOT consult this. It reads the pointer's world
//! position like everything else and stays unaware that a warp happened.

use compositor_orchestration_core_state_base::Loop;
use compositor_y5_picker_system_base::base::PICKER_WORLD;

/// Whether an overlay owns the screen right now.
pub fn suppressed(state: &Loop) -> bool {
    // The picker runs as its own world; when it is the active one, the bundle
    // whose warp is published is not what is on screen.
    if state.inner.worlds.active_id() == PICKER_WORLD {
        return true;
    }
    // The lock screen replaces the world's content wholesale.
    if matches!(
        state.inner.status,
        compositor_orchestration_core_state_base::state::Status::Locked { .. }
    ) {
        return true;
    }
    // The overview draws its grid over a FROZEN capture, so even the world's own
    // bundle is not producing what the pointer is over.
    state.inner.overview().visible
}
