//! `GuideSystem` — seeds the guide-popup slot on its world.
//!
//! Like the overview slot this one is rim-driven (summoned from the pointer
//! handlers, dismissed from the keyboard ones, reconciled on the render path),
//! so the system owns no buffer or channel — it only inserts the default slot at
//! world build time.
//!
//! PER-WORLD, and that is the whole point: the slot stores `HandleId`s into an
//! iced registry, and the registry is itself per-world. A global slot would name
//! surfaces belonging to whichever world happened to build them, so a world
//! switch stranded them — the reconcilers stopped finding the handles, rebuilt in
//! the new world, and the surfaces left behind stayed up untracked and
//! impossible to dismiss. Keeping the handles beside the registry that owns them
//! makes that unrepresentable.
//!
//! Register it on every SPATIAL world: the spawn target is always spatial, and
//! that is the world the guide accessors read every frame.

use compositor_support_system_trait_system_base::base::{System, WorldBuilder};
use compositor_y5_guide_state_base::state::{GuideState, GUIDE};

#[derive(Default)]
pub struct GuideSystem;

impl System for GuideSystem {
    fn name(&self) -> &'static str {
        "guide"
    }

    fn register(&mut self, builder: &mut WorldBuilder) {
        builder.storage.insert(&GUIDE, GuideState::default());
    }
}
