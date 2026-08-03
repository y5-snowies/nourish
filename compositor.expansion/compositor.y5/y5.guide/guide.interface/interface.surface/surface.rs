//! Bits both guide reconcilers (`menu.create`, `help.create`) need: the icon
//! font, the cursor-output gate, handle liveness, teardown, and the one channel
//! every guide click leaves the surface through.

use std::sync::mpsc::Sender;
use std::sync::Once;

use compositor_monitor_compositor_iced_base::HandleId;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_guide_menu_view::GuideMessage;
use compositor_y5_guide_state_base::state::{GuideState, GUIDE};
use compositor_y5_surface_protocol_base::protocol::{SurfaceMessage, SurfaceMessageType};

/// Register the Material Symbols font into iced's global DB once (shared with
/// the selection toolbar and the touch pane) so the glyphs render.
pub fn ensure_font() {
    static ONCE: Once = Once::new();
    ONCE.call_once(compositor_monitor_selection_font_base::font::load);
}

/// True only on the CURSOR's output pass. The reconciler runs once per output,
/// and every position it computes resolves against `render_output`; letting the
/// last-drawn monitor win the stored location while input is hit-tested against
/// the cursor's output makes clicks miss on multi-monitor. `None` = winit/single
/// pass. Mirrors the selection toolbar's gate of the same name.
pub fn on_active_output(state: &Loop) -> bool {
    state.inner.render_output.as_ref().is_none_or(|k| *k == state.inner.active_output_key())
}

/// The stored handle, but only while the CURRENT world's registry still holds
/// it. A world switch swaps the registry out from under us, so a bare `Option`
/// would name a surface that no longer exists.
pub fn live(state: &Loop, pick: fn(&GuideState) -> Option<HandleId>) -> Option<HandleId> {
    let id = pick(state.inner.kernel.get(&GUIDE))?;
    state.inner.surface().registry.as_ref().filter(|r| r.contains(id)).map(|_| id)
}

/// Destroy a guide surface (also clears its keyboard focus / pointer / grab).
pub fn destroy(state: &mut Loop, id: HandleId) {
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        reg.destroy_by_id(id);
    }
}

/// Forward an actionable click to the surface pump, which runs it against the
/// compositor (`interface.handle`). `Hover` is surface-internal — it only drives
/// the menu's own caption — so it never leaves.
pub fn dispatch(message: &GuideMessage, tx: &Sender<SurfaceMessage>) {
    if matches!(message, GuideMessage::Hover(_)) {
        return;
    }
    let _ = tx.send(SurfaceMessage { message: SurfaceMessageType::Guide(message.clone()) });
}
