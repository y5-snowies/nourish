use compositor_support_system_storage_slot_base::base::Storage;
use compositor_support_system_world_host_base::base::World;
use std::collections::HashMap;
use uuid::Uuid;

/// Fixed identities for the static worlds, stable across restarts so their saved
/// state reloads. Picker-created worlds get generated `Uuid::now_v7()` instead.
pub const MAIN_WORLD: Uuid = Uuid::from_u128(0x59350000_0000_4000_8000_000000000001);
pub const LOCK_WORLD: Uuid = Uuid::from_u128(0x59350000_0000_4000_8000_000000000002);
pub const PICKER_WORLD: Uuid = Uuid::from_u128(0x59350000_0000_4000_8000_000000000003);

/// Owns every world, identified by UUID. Exactly one world is ACTIVE (receives
/// input, dispatch, update, draw). Worlds never close — switching disables the
/// outgoing world's systems and enables the incoming ones; state is kept.
pub struct WorldManager {
    worlds: Vec<World>,
    index: HashMap<Uuid, usize>,
    active: Uuid,
    /// The spatial world new client windows map into. Invariant: always a valid
    /// SPATIAL world, so there is always somewhere to spawn.
    spawn_target: Uuid,
}

impl WorldManager {
    /// Start with the initial (main, SPATIAL) world, already active and the
    /// spawn-target. Activation fires `on_enable`.
    pub fn new(mut main: World, kernel: &Storage) -> Self {
        main.enable(kernel);
        let id = main.id;
        let mut index = HashMap::new();
        index.insert(id, 0);
        Self { worlds: vec![main], index, active: id, spawn_target: id }
    }

    fn idx(&self, id: Uuid) -> usize {
        *self.index.get(&id).unwrap_or_else(|| panic!("unknown world {id}"))
    }

    /// The spatial world new toplevels map into (the space-hosting world).
    pub fn spawn_target(&self) -> Uuid {
        self.spawn_target
    }

    /// Reassign the spawn-target spatial world. Caller ensures `id` is SPATIAL.
    /// Returns `true` if the target actually changed (the space the foreign mirror
    /// advertises is the spawn-target's — see `Orchestrator::space_state`).
    pub fn set_spawn_target(&mut self, id: Uuid) -> bool {
        assert!(self.index.contains_key(&id), "spawn-target to unknown world {id}");
        if self.spawn_target == id {
            return false;
        }
        self.spawn_target = id;
        true
    }

    /// Add a dormant world (no enable). Returns its id.
    pub fn add(&mut self, world: World) -> Uuid {
        let id = world.id;
        self.index.insert(id, self.worlds.len());
        self.worlds.push(world);
        id
    }

    pub fn active_id(&self) -> Uuid {
        self.active
    }
    /// Every world id (insertion order); the loader prewarms each at startup.
    pub fn ids(&self) -> Vec<Uuid> { self.worlds.iter().map(|w| w.id).collect() }

    pub fn active(&self) -> &World {
        &self.worlds[self.idx(self.active)]
    }

    pub fn active_mut(&mut self) -> &mut World {
        let i = self.idx(self.active);
        &mut self.worlds[i]
    }

    pub fn get(&self, id: Uuid) -> &World {
        &self.worlds[self.idx(id)]
    }

    pub fn get_mut(&mut self, id: Uuid) -> &mut World {
        let i = self.idx(id);
        &mut self.worlds[i]
    }

    /// True if a world with this id exists.
    pub fn contains(&self, id: Uuid) -> bool {
        self.index.contains_key(&id)
    }

    /// Swap the active world: on_disable(outgoing) → on_enable(incoming). No-op if
    /// already active. Panics on an unknown id (wiring bug).
    pub fn switch(&mut self, id: Uuid, kernel: &Storage) {
        assert!(self.index.contains_key(&id), "switch to unknown world {id}");
        if id == self.active {
            return;
        }
        let (out, inc) = (self.idx(self.active), self.idx(id));
        self.worlds[out].disable(kernel);
        self.active = id;
        self.worlds[inc].enable(kernel);
    }
}
