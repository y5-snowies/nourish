//! What an edit in the inline shader editor does.
//!
//! It DELEGATES to the settings handler rather than writing the world's `Two` slot
//! itself. That handler already resolves the name-keyed override onto the live
//! instance's param slot, persists it (debounced, because drags fire fast) and
//! keeps the Settings World tab's copy in step. A second implementation of that
//! would be a second place for the slot mapping to be wrong, and the slot mapping
//! is the part that took the longest to get right.

use compositor_configurator_settings_surface_message::message::SettingsMessage;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_guide_shader_view::ShaderMessage;
use compositor_y5_guide_state_base::state::GUIDE_MUT;
use smithay::backend::renderer::gles::GlesRenderer;

pub fn delegate(state: &mut Loop, renderer: &mut GlesRenderer, message: ShaderMessage) {
    match message {
        // Every control in this panel is one `@prop`, and a `@prop` is an in-place
        // write to the live element's param array that the next frame's push
        // carries. Nothing here rebuilds the pipeline, so nothing here is worth
        // debouncing — the expensive edit (changing the bundle) is deliberately
        // not offered by this panel.
        ShaderMessage::SetParams(values) => {
            compositor_configurator_settings_interface_handle::handle::handle(
                state,
                renderer,
                SettingsMessage::SetWorldShaderParams(values),
            );
        }
        ShaderMessage::Close => close(state),
        // Compositor → surface only; it never comes back this way.
        ShaderMessage::Sync(_) => {}
    }
}

/// Clear the DESIRE only; the reconciler tears the surface down next frame.
pub fn close(state: &mut Loop) {
    state.inner.kernel.get_mut(&GUIDE_MUT).shader_open = false;
}
