use uuid::Uuid;

/// Fixed identities for the static worlds, stable across restarts so their saved
/// state reloads under the same id. Worlds created at runtime (the picker's scene
/// worlds) get a generated `Uuid::now_v7()` instead, so these three are the only
/// ids a build can predict.
///
/// The SPATIAL session world: it hosts a window `Space` and is the initial
/// spawn-target.
pub const MAIN_WORLD: Uuid = Uuid::from_u128(0x59350000_0000_4000_8000_000000000001);

/// The lock screen's OVERLAY world — no `Space`, hosts no client windows.
pub const LOCK_WORLD: Uuid = Uuid::from_u128(0x59350000_0000_4000_8000_000000000002);

/// The world-selection screen's OVERLAY world — no `Space`, hosts no client
/// windows, and owns the picker's cell registry.
pub const PICKER_WORLD: Uuid = Uuid::from_u128(0x59350000_0000_4000_8000_000000000003);

/// The OVERLAY worlds: they own no window `Space` and no persisted records, so a
/// table entry filed under one of these ids is not data.
pub const OVERLAY_WORLDS: [Uuid; 2] = [LOCK_WORLD, PICKER_WORLD];
