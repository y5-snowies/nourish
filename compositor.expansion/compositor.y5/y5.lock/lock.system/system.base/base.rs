use compositor_y5_lock_state_base::state::LockState;
use compositor_support_system_storage_token_base::base::{Token, TokenMut};
use compositor_support_system_trait_system_base::base::{System, WorldBuilder};

/// The lock overlay's fixed world id (single source is `WorldManager`). The
/// session world is resolved dynamically via `WorldManager::spawn_target()` / the
/// Orchestrator focus accessors, never a literal id (document/WORLD_DELEGATION.md).
pub use compositor_orchestration_world_manager_base::manager::LOCK_WORLD;

pub static LOCK: Token<LockState> = Token::new();
/// TRANSITIONAL pub: the legacy lock interface/scene paths still drive this
/// slot directly until they become this world's systems.
pub static LOCK_MUT: TokenMut<LockState> = TokenMut::new(&LOCK);

/// The LOCK world's OWN surface slot — never the session world's.
///
/// The lock world already owns its bevy registry rather than borrowing the
/// session's; this is the same rule for iced, and it exists because the session
/// one is not stable underneath a live lock. Every lock path used to resolve the
/// registry through `Orchestrator::surface_mut()`, which is the SPAWN TARGET's:
/// locking does not move the spawn target, but the picker's `enter()` does, so
/// `Super+K` → `Super+Alt+L` → choose a world built the auth panel in the
/// outgoing world's registry and then rendered from the incoming one. The lock
/// screen came up with no PIN input, recoverable only by a VT switch.
///
/// Takes `WorldManager` rather than `Loop` deliberately: `orchestration.core.state`
/// depends on this crate, so naming `Loop` here would be a cycle.
pub fn surface(
    worlds: &mut compositor_orchestration_world_manager_base::manager::WorldManager,
) -> Option<&mut compositor_y5_surface_state_base::state::SurfaceState> {
    worlds
        .get_mut(LOCK_WORLD)
        .storage_mut()
        .try_get_mut(&compositor_y5_surface_system_base::base::SURFACE_MUT)
}

/// The LOCK world's iced registry, or `None` before the prewarm has built it.
pub fn registry(
    worlds: &mut compositor_orchestration_world_manager_base::manager::WorldManager,
) -> Option<&mut compositor_monitor_compositor_iced_base::IcedRegistry> {
    surface(worlds).and_then(|s| s.registry.as_mut())
}

/// Owns the lock-screen state slot — registered in the LOCK world, not main.
/// Locking is a world switch: WorldManager::switch(LOCK_WORLD) fires
/// on_disable on the session systems and on_enable here.
#[derive(Default)]
pub struct LockSystem;

impl System for LockSystem {
    fn name(&self) -> &'static str {
        "lock"
    }

    fn register(&mut self, builder: &mut WorldBuilder) {
        builder.storage.insert(&LOCK, LockState::new());
    }
}
