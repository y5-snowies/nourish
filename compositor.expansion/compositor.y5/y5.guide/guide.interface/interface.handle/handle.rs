//! Compositor-side handling of guide-menu clicks (drained from the surface
//! channel by the global pump).
//!
//! Both entries dismiss the menu that raised them — the rim's "any click outside
//! closes it" rule cannot, because a click ON the menu is by definition not
//! outside it, so the action itself has to.

use compositor_orchestration_core_state_base::Loop;
use compositor_y5_guide_interface_base::base;
use compositor_y5_guide_menu_view::GuideMessage;
use compositor_y5_overview_state_base::base::{Tab, OVERVIEW_TAB_MUT};

pub fn delegate(state: &mut Loop, message: GuideMessage) {
    match message {
        GuideMessage::OpenSettings => {
            base::close(state);
            open_settings(state);
        }
        GuideMessage::OpenHelp => base::open_help(state),
        // The editor is meant to be used WITH the desktop it edits, so unlike the
        // other two entries it is not a modal takeover — dismiss only the menu.
        GuideMessage::OpenShader => {
            base::close(state);
            base::open_shader(state);
        }
        // Surface-internal (drives the menu's own caption) — never forwarded.
        GuideMessage::Hover(_) => {}
    }
}

/// Open the overview ON the Settings tab. `toggle` seeds the opening world's
/// slot from the session-wide last-used tab, so setting that first is what
/// steers it; the overlay being already open (it should not be — the menu only
/// summons on bare canvas) just switches tabs in place.
fn open_settings(state: &mut Loop) {
    *state.inner.kernel.get_mut(&OVERVIEW_TAB_MUT) = Tab::Settings;
    if state.inner.overview().visible {
        state.inner.overview_mut().tab = Tab::Settings;
        return;
    }
    compositor_y5_overview_interface_base::base::toggle(state);
}
