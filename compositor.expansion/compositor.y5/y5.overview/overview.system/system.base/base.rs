//! `OverviewSystem` — seeds the overview-mode slot on its world.
//!
//! The slot is rim-driven (toggled from the keyboard shortcut, tab-set from the
//! surface message pump) rather than mutated through the input bus, so this
//! system owns no buffer or channel — it only inserts the default slot at world
//! build time. Register it on every SPATIAL world (the overview renders that
//! world's own windows).

use std::any::Any;

use compositor_orchestration_driver_settings_base::base::{SETTINGS_SURFACE, SETTINGS_SURFACE_MUT};
use compositor_support_system_buffer_token_base::y5_buffer;
use compositor_support_system_trait_system_base::base::{BufferCx, System, SystemCx, WorldBuilder};
use compositor_y5_overview_state_base::base::{Overview, OVERVIEW};

/// Self-buffer signal: drop this world's settings panel.
struct CloseSettings;
y5_buffer!(CLOSE_BUF: CloseSettings);

#[derive(Default)]
pub struct OverviewSystem;

impl System for OverviewSystem {
    fn name(&self) -> &'static str {
        "overview"
    }

    fn register(&mut self, builder: &mut WorldBuilder) {
        builder.storage.insert(&OVERVIEW, Overview::new());
        // The settings panel is a tab OF this slot, and its surface lives in this
        // world's registry — so its handle belongs here too, not beside the
        // session-wide `SETTINGS` state. See `SETTINGS_SURFACE`.
        builder.storage.insert(&SETTINGS_SURFACE, None);
    }

    /// This world stopped being the active one — drop its settings panel.
    ///
    /// The surface is per-world; everything it DRIVES is not. One audio
    /// subscription, one bluetooth scan toggle, one provisional-output-mode
    /// confirm, one set of de-dup caches, all process-wide in `draw.settings` and
    /// all paired against a single live panel. Leaving a world with settings open
    /// used to strand its panel — the reconciler only ever visits the focused
    /// world — and opening settings in the next world then took those resources
    /// over, dropping the first panel's audio watch and later stopping its scan.
    ///
    /// So: at most one panel, ever. This half is what a system owns — the surface
    /// and the slot, both in its own storage. The process-wide half is
    /// `SETTINGS.open` outliving the handle, which the reconciler picks up on its
    /// next frame (its `(false, None, true)` arm). Textures are not the motive; the
    /// hidden-surface path already releases those.
    fn on_disable(&mut self, cx: &mut SystemCx) {
        cx.write(&CLOSE_BUF, CloseSettings);
    }

    /// `each_system` flushes buffers per system, so this lands inside the same
    /// `disable()` call — a world that is no longer active never runs `dispatch`.
    fn buffer(&mut self, cx: &mut BufferCx, _message: Box<dyn Any>) {
        let Some(id) = cx.storage.try_get_mut(&SETTINGS_SURFACE_MUT).and_then(|h| h.take()) else {
            return;
        };
        if let Some(reg) = cx
            .storage
            .try_get_mut(&compositor_y5_surface_system_base::base::SURFACE_MUT)
            .and_then(|s| s.registry.as_mut())
        {
            reg.destroy_by_id(id);
            reg.set_keyboard_focus(None);
        }
    }
}
