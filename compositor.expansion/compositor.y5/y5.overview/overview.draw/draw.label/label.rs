//! What the overview's hover card SAYS about a window: its title, and the window
//! itself. The icon is a subject of its own — see `overview.draw/draw.icon`.
//!
//! The title is read live off the toplevel role. It is always available, so it
//! never waits on — or degrades to — an introspection sample.

use smithay::desktop::Window;
use uuid::Uuid;
use compositor_introspection_extraction_window_base::meta::wayland::read_surface_identity;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_window_interface_record::window::LoopWindow;

/// The live window carrying this uuid on the focused world, if it is still there.
pub fn window_of(state: &Loop, uuid: Uuid) -> Option<Window> {
    state.inner.space_state().state.elements().find(|w| w.uuid() == Some(uuid)).cloned()
}

/// The live toplevel title, falling back to the app_id and then to a label —
/// never to nothing, since the card is the only thing naming the thumbnail.
pub fn title_of(window: &Window) -> String {
    let (app_id, title, _) = read_surface_identity(window);
    title
        .filter(|t| !t.trim().is_empty())
        .or(app_id.filter(|a| !a.trim().is_empty()))
        .unwrap_or_else(|| "Untitled".to_string())
}
