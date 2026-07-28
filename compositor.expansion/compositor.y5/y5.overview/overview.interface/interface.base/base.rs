//! Overview rim interface: toggle the overlay and reconcile the menu-bar
//! surface.
//!
//! `toggle`/`request_close` flip `Overview::visible` synchronously (so input
//! gating and the scene react this frame), then defer the menu-bar surface
//! create/destroy to the surface pump via `OverviewSurfaceMessage::Reconcile` —
//! the pump holds the GLES renderer `interface.surface::open` needs.

use smithay::backend::renderer::gles::GlesRenderer;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_overview_interface_surface::surface;
use compositor_y5_overview_state_base::base::OverviewSurfaceMessage;
use compositor_y5_surface_protocol_base::protocol::{SurfaceMessage, SurfaceMessageType};

/// Super+Tab: flip the overlay on/off. Defers the menu-bar surface op.
pub fn toggle(state: &mut Loop) {
    let now = !state.inner.overview().visible;
    state.inner.overview_mut().visible = now;
    if now {
        // Reopen on the SESSION's last-used tab, freshly scrolled — retained
        // across world switches via the kernel `OVERVIEW_TAB` token.
        let tab = *state.inner.kernel.get(&compositor_y5_overview_state_base::base::OVERVIEW_TAB);
        let overview = state.inner.overview_mut();
        overview.scroll = 0.0;
        overview.tab = tab;
    }
    defer_reconcile(state);
}

/// Escape: close the overlay if it is open (no-op otherwise).
pub fn request_close(state: &mut Loop) -> bool {
    if !state.inner.overview().visible {
        return false;
    }
    state.inner.overview_mut().visible = false;
    defer_reconcile(state);
    true
}

/// Click-username / Escape / click-outside: defer a logout-popup toggle to the
/// surface pump (it holds the GLES renderer the popup surface needs to open).
pub fn toggle_logout(state: &mut Loop) {
    defer(state, OverviewSurfaceMessage::ToggleLogout);
}

fn defer_reconcile(state: &mut Loop) {
    defer(state, OverviewSurfaceMessage::Reconcile);
}

fn defer(state: &mut Loop, message: OverviewSurfaceMessage) {
    let _ = state
        .inner
        .surface_mut()
        .surface_message_buffer_channel
        .0
        .send(SurfaceMessage {
            message: SurfaceMessageType::Overview(message),
        });
}

/// Surface-pump entry: apply a deferred overview action (has the GLES renderer).
pub fn handle(state: &mut Loop, renderer: &mut GlesRenderer, message: OverviewSurfaceMessage) {
    match message {
        OverviewSurfaceMessage::Reconcile => {
            if state.inner.overview().visible {
                surface::open(state, renderer);
            } else {
                surface::close(state);
            }
        }
        OverviewSurfaceMessage::SetTab(tab) => {
            state.inner.overview_mut().tab = tab;
            // Session-wide tab memory (survives world switches).
            *state.inner.kernel.get_mut(&compositor_y5_overview_state_base::base::OVERVIEW_TAB_MUT) = tab;
        }
        OverviewSurfaceMessage::Close => {
            request_close(state);
        }
        OverviewSurfaceMessage::ToggleLogout => {
            if state.inner.overview().logout.is_some() {
                surface::close_logout(state);
            } else {
                surface::open_logout(state, renderer);
            }
        }
        OverviewSurfaceMessage::Logout => {
            // End the session: stop the calloop event loop, which exits the
            // compositor process and returns to the login manager. (Inlined rather
            // than calling `draw.state.lifecycle::stop` to avoid a dependency cycle
            // back through the seat/input chain into this very crate.)
            state.inner.loader.loop_signal.stop();
        }
    }
}
