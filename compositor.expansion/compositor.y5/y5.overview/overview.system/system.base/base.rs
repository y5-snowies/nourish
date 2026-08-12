//! `OverviewSystem` — seeds the overview-mode slot on its world.
//!
//! The slot is rim-driven (toggled from the keyboard shortcut, tab-set from the
//! surface message pump) rather than mutated through the input bus, so this
//! system owns no buffer or channel — it only inserts the default slot at world
//! build time. Register it on every SPATIAL world (the overview renders that
//! world's own windows).

use compositor_orchestration_driver_settings_base::base::SETTINGS_SURFACE;
use compositor_support_system_trait_system_base::base::{System, WorldBuilder};
use compositor_y5_overview_state_base::base::{Overview, OVERVIEW};

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
}
