use std::any::Any;

use compositor_support_system_buffer_token_base::y5_buffer;
use compositor_support_system_trait_system_base::base::{BufferCx, System, SystemCx, WorldBuilder};
use compositor_y5_select_system_base::base::{SelectionChanged, SELECTION_CHANGED};
use compositor_orchestration_driver_selection_base::base::{
    SelectionOverlayState, SELECTION_OVERLAY, SELECTION_REANCHOR, SELECTION_REANCHOR_MUT,
};

/// Self-buffer signal: re-anchor the toolbar to the cursor.
struct Reanchor;
y5_buffer!(REANCHOR_BUF: Reanchor);

/// Reacts to the selection-change *event* (`SELECTION_CHANGED`, announced by
/// `SelectSystem` on every applied change, incl. primary-only) by raising a
/// one-shot `SELECTION_REANCHOR` flag in this world's storage.
///
/// The actual move can't happen here: re-anchoring needs the live cursor, which
/// is only readable from the seat on the render path (it is not mirrored into
/// system storage). So the render-path reconciler consumes the flag and applies
/// the move with the seat cursor. This system owns the event subscription; the
/// render hook owns the seat-bound apply.
#[derive(Default)]
pub struct SelectionOverlaySystem;

impl System for SelectionOverlaySystem {
    fn name(&self) -> &'static str {
        "select_overlay"
    }

    fn register(&mut self, builder: &mut WorldBuilder) {
        // The toolbar's own slot lives here too, beside the iced registry that
        // holds its surfaces and the selection it is reconciled against — all
        // three per-world, all three resolved through the same spawn target.
        builder.storage.insert(&SELECTION_OVERLAY, SelectionOverlayState::default());
        builder.storage.insert(&SELECTION_REANCHOR, false);
        builder.receive(&SELECTION_CHANGED, Self::on_selection_changed);
    }

    fn buffer(&mut self, cx: &mut BufferCx, _message: Box<dyn Any>) {
        *cx.storage.get_mut(&SELECTION_REANCHOR_MUT) = true;
    }
}

impl SelectionOverlaySystem {
    /// Any selection change: raise the re-anchor flag (applied on the render
    /// path, where the seat cursor is available).
    fn on_selection_changed(&mut self, cx: &mut SystemCx, _event: &SelectionChanged) {
        cx.write(&REANCHOR_BUF, Reanchor);
    }
}
