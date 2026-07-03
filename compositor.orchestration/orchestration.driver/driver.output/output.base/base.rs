//! Output-mode driver channel between the rim (settings window, holds `&mut Loop`)
//! and the kernel (owns the live `NativeDrmOutput` inside its calloop sources).
//! The rim cannot touch the DRM output directly, so — like the lid driver — it
//! writes a primitive request the kernel loop drains, and reads back a primitive
//! snapshot/result the kernel writes. All values are primitive: no smithay/DRM
//! types cross the layer boundary.

use compositor_support_system_storage_token_base::base::{Token, TokenMut};

/// Stable cross-layer key for one physical monitor: the EDID identity
/// "make model serial" (the same string as `DisplayInfo::edid_key` and
/// `MonitorIdentity::key()`). Used to key per-output view state and the
/// cursor-teleport layout. A plain `String` so this primitive-only driver crate
/// stays free of smithay/DRM types; the `smithay::Output` → key mapping lives in
/// the orchestration core (`output_key`), where an `Output` is available.
pub type OutputKey = String;

/// One advertised mode, primitive form (refresh in mHz to match DRM `vrefresh*1000`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeInfo {
    pub width: u16,
    pub height: u16,
    pub refresh_mhz: u32,
}

/// Rim → kernel: a step in the user-confirmed mode-change transaction.
/// `Apply` provisionally switches and arms the confirm/revert watchdog; `Confirm`
/// (user kept it) makes it permanent; `Revert` (user declined / dialog closed)
/// restores the previous mode now. The kernel drains at most one per loop.
///
/// `Apply` carries the target monitor's `edid_key` so the kernel changes the mode
/// of the SELECTED output's pipe (multi-output), not always the primary. Only one
/// provisional change is in flight at a time, so `Confirm`/`Revert` need no target
/// — the kernel resolves the pipe with the armed watchdog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputModeRequest {
    Apply { edid_key: String, width: u16, height: u16, refresh_mhz: u32 },
    Confirm,
    Revert,
}

/// Kernel → rim: the connector's advertised modes for the settings UI to list,
/// plus its current mode and a stable EDID identity ("make model serial").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputModesSnapshot {
    pub edid_key: String,
    pub current: Option<ModeInfo>,
    pub available: Vec<ModeInfo>,
}

/// Kernel → rim: outcome of the last `Apply`, driving the UI confirmation dialog.
/// `Provisional` = applied, awaiting Keep/Revert (show countdown); `Confirmed` =
/// kept (UI then persists to preferences.json); `Reverted` = restored (user
/// declined, dialog timed out, or no signal); `Failed` = could not apply at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyResult {
    Provisional,
    Confirmed,
    Reverted,
    Failed,
}

/// Rim-issued mode request, drained by the kernel loop (`None` when idle).
pub static OUTPUT_MODE_REQUEST: Token<Option<OutputModeRequest>> = Token::new();
pub static OUTPUT_MODE_REQUEST_MUT: TokenMut<Option<OutputModeRequest>> =
    TokenMut::new(&OUTPUT_MODE_REQUEST);

/// Kernel-written advertised-mode snapshot, read by the settings UI.
pub static OUTPUT_MODES_SNAPSHOT: Token<OutputModesSnapshot> = Token::new();
pub static OUTPUT_MODES_SNAPSHOT_MUT: TokenMut<OutputModesSnapshot> =
    TokenMut::new(&OUTPUT_MODES_SNAPSHOT);

/// Kernel-written result of the last apply (`None` until the first transaction).
pub static OUTPUT_MODE_RESULT: Token<Option<ApplyResult>> = Token::new();
pub static OUTPUT_MODE_RESULT_MUT: TokenMut<Option<ApplyResult>> =
    TokenMut::new(&OUTPUT_MODE_RESULT);

/// Rim → kernel: request a hotplug `reconcile` pass on the next drain. Set by the
/// settings window after activating/deactivating a monitor (an active-set change,
/// not a hardware hotplug), so the kernel brings the newly-active output up or tears
/// a now-inactive one down. Cleared by the kernel when it runs the pass.
pub static OUTPUT_RECONCILE_REQUEST: Token<bool> = Token::new();
pub static OUTPUT_RECONCILE_REQUEST_MUT: TokenMut<bool> = TokenMut::new(&OUTPUT_RECONCILE_REQUEST);

/// Baseline of a PROVISIONAL activate/deactivate awaiting Keep/Revert — the settings
/// "check changes" fault gate for an active-set change, mirroring the mode gate
/// (`OutputModeRequest`). The rim applies the toggle live (persist + reconcile) and
/// records this baseline; APPLY clears it (keep), while REVERT or a timeout restores
/// `prior_active` and reconciles back. `epoch` lets a superseded one-shot revert
/// watchdog tell it is stale (a prior gate confirmed, a new one armed within the
/// timeout) and no-op instead of reverting the current gate. `None` = no gate armed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveRevert {
    /// EDID identity ("make model serial") of the toggled monitor.
    pub edid: String,
    /// The `active` flag to restore on revert — the state BEFORE the provisional apply.
    pub prior_active: bool,
    /// Monotonic arm id; the auto-revert watchdog acts only if it still matches.
    pub epoch: u64,
}

/// Rim-armed provisional activate/deactivate awaiting confirm/revert (`None` = idle).
pub static OUTPUT_ACTIVE_REVERT: Token<Option<ActiveRevert>> = Token::new();
pub static OUTPUT_ACTIVE_REVERT_MUT: TokenMut<Option<ActiveRevert>> = TokenMut::new(&OUTPUT_ACTIVE_REVERT);

/// Kernel → rim: one connected connector, primitive form, for the monitor picker.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DisplayInfo {
    /// Stable per-monitor key: the EDID identity "make model serial" (incl. the
    /// unit's serial, so two identical monitors differ). The picker selection key,
    /// switch-request target, and persistence key — the SAME key the standalone
    /// settings-editor writes, so preferences match across both.
    pub edid_key: String,
    /// Friendly label: the EDID make/model + connector name when readable, else the
    /// connector name.
    pub name: String,
    pub connected: bool,
    /// True for the connector treated as the primary/anchor output.
    pub active: bool,
    /// Whether the user has this monitor ENABLED (driven). `false` = deactivated in
    /// the settings Display tab ("Inactive"). Default `true`. Distinct from `active`
    /// (which marks the primary): a monitor can be enabled but not the primary.
    pub enabled: bool,
    pub current: Option<ModeInfo>,
    /// The mode saved in preferences for THIS monitor (its per-output profile),
    /// if any — so the picker defaults an inactive monitor to its saved mode rather
    /// than just the recommended one. `None` when no profile mode is set.
    pub preferred: Option<ModeInfo>,
    pub available: Vec<ModeInfo>,
}

/// Kernel → rim: every connected connector on the driven device (the active one
/// plus connected-but-inactive monitors), so the UI can offer a preferred-monitor
/// picker and list each monitor's modes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputsSnapshot {
    pub displays: Vec<DisplayInfo>,
}

/// Kernel-written full connector list, read by the settings Display panel.
pub static OUTPUTS_SNAPSHOT: Token<OutputsSnapshot> = Token::new();
pub static OUTPUTS_SNAPSHOT_MUT: TokenMut<OutputsSnapshot> = TokenMut::new(&OUTPUTS_SNAPSHOT);

// ── Cursor-teleport layout ────────────────────────────────────────────────────
// The output arrangement the cursor crosses between monitors. Primitive (String /
// f32 / u64 / bool), so it lives in this smithay-free rim output-data crate — NOT on
// the Orchestrator (kept slim). Rebuilt from preferences by the settings handler and
// the kernel reconcile; read by the pointer-motion path. Moved here from the former
// `orchestration.seat.pointer.teleport` crate.

/// The side of a placement the cursor crossed (or entered through).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    /// The edge on the far side — the side a neighbor is entered through.
    pub fn opposite(self) -> Edge {
        match self {
            Edge::Left => Edge::Right,
            Edge::Right => Edge::Left,
            Edge::Top => Edge::Bottom,
            Edge::Bottom => Edge::Top,
        }
    }
}

/// One placed monitor square in abstract layout space (`size` = side length; the
/// square spans `[x, x+size] × [y, y+size]`).
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub id: u64,
    pub key: String,
    pub x: f32,
    pub y: f32,
    /// Width + height of the teleport zone (a free rectangle, not a square).
    pub w: f32,
    pub h: f32,
}

/// The result of a successful crossing: the entered placement plus where along its
/// entry edge the cursor lands (proportional, clamped to `[0, 1]`).
#[derive(Clone, Debug, PartialEq)]
pub struct Neighbor {
    pub id: u64,
    pub key: String,
    /// The edge of the entered placement the cursor comes in through.
    pub entry_edge: Edge,
    /// Position along `entry_edge`, `0.0` = its start (top for L/R, left for T/B).
    pub entry_frac: f32,
}

/// The full arrangement of teleport zones (empty on single-monitor / no layout).
/// Only ACTIVE + CONNECTED monitors' placements are present — [`build_teleport`]
/// filters inactive/disconnected ones out (they stay in preferences, just not here).
#[derive(Clone, Debug, Default)]
pub struct TeleportLayout {
    pub placements: Vec<Placement>,
    /// Wrap around the layout edges instead of clamping when the pointer exits a
    /// side with no monitor across it (the settings "cyclic" option).
    pub cyclic: bool,
}

impl TeleportLayout {
    pub fn new(placements: Vec<Placement>, cyclic: bool) -> Self {
        TeleportLayout { placements, cyclic }
    }

    pub fn is_empty(&self) -> bool {
        self.placements.is_empty()
    }

    pub fn get(&self, id: u64) -> Option<&Placement> {
        self.placements.iter().find(|p| p.id == id)
    }

    /// The first placement of monitor `key`, if any — the zone the cursor starts in
    /// when it lands on that monitor with no more specific placement known.
    pub fn first_of(&self, key: &str) -> Option<&Placement> {
        self.placements.iter().find(|p| p.key == key)
    }

    /// Given the cursor is leaving placement `from_id` across `edge` at `exit_frac`
    /// (`0.0` = start of that edge: top for Left/Right, left for Top/Bottom), find the
    /// placement to enter by ORTHOGONAL PROJECTION: cast a ray perpendicular to the
    /// crossed edge and pick the NEAREST placement in that direction whose facing span
    /// covers the crossing point. Placements need not touch (unlike snap-adjacency), so
    /// gapped layouts still cross. If nothing lies across that edge and `cyclic` is set,
    /// wrap around and enter from the opposite side of the layout; otherwise `None`
    /// (the caller clamps at the edge).
    pub fn neighbor(&self, from_id: u64, edge: Edge, exit_frac: f32) -> Option<Neighbor> {
        let from = self.get(from_id)?;
        let f = exit_frac.clamp(0.0, 1.0);
        // The crossing point in abstract space, on `from`'s `edge`.
        let (ex, ey) = match edge {
            Edge::Right => (from.x + from.w, from.y + f * from.h),
            Edge::Left => (from.x, from.y + f * from.h),
            Edge::Bottom => (from.x + f * from.w, from.y + from.h),
            Edge::Top => (from.x + f * from.w, from.y),
        };
        let entry_edge = edge.opposite();
        // The crossing coordinate on the axis PARALLEL to the crossed edge (y for a
        // Left/Right crossing, x for Top/Bottom) — the ray's fixed coordinate. The
        // entered placement's facing edge must span it.
        let cross = match edge { Edge::Left | Edge::Right => ey, Edge::Top | Edge::Bottom => ex };
        let covers = |q: &Placement| match edge {
            Edge::Left | Edge::Right => within(cross, q.y, q.y + q.h),
            Edge::Top | Edge::Bottom => within(cross, q.x, q.x + q.w),
        };

        // Nearest placement in the travel direction whose facing span covers `cross`.
        let mut best: Option<(&Placement, f32)> = None; // (placement, ranking key; lower = better)
        for q in &self.placements {
            if q.id == from_id || !covers(q) {
                continue;
            }
            // Distance along the travel direction; `None` if `q` isn't in that direction.
            let dist = match edge {
                Edge::Right => (q.x + q.w > ex).then(|| (q.x - ex).max(0.0)),
                Edge::Left => (q.x < ex).then(|| (ex - (q.x + q.w)).max(0.0)),
                Edge::Bottom => (q.y + q.h > ey).then(|| (q.y - ey).max(0.0)),
                Edge::Top => (q.y < ey).then(|| (ey - (q.y + q.h)).max(0.0)),
            };
            if let Some(d) = dist {
                if best.map_or(true, |(_, bd)| d < bd) {
                    best = Some((q, d));
                }
            }
        }

        // Cyclic wrap: nothing across that edge → re-enter from the far side of the
        // whole layout (the extreme placement on the opposite side that covers `cross`).
        if best.is_none() && self.cyclic {
            for q in &self.placements {
                if q.id == from_id || !covers(q) {
                    continue;
                }
                // Rank so the wrap-side extreme wins: exit Right → leftmost (min x),
                // Left → rightmost, Bottom → topmost, Top → bottommost.
                let key = match edge {
                    Edge::Right => q.x,
                    Edge::Left => -(q.x + q.w),
                    Edge::Bottom => q.y,
                    Edge::Top => -(q.y + q.h),
                };
                if best.map_or(true, |(_, bk)| key < bk) {
                    best = Some((q, key));
                }
            }
        }

        let (q, _) = best?;
        // Where along `q`'s entry edge the cursor lands, proportionally.
        let entry_frac = match entry_edge {
            Edge::Left | Edge::Right => (ey - q.y) / q.h,
            Edge::Top | Edge::Bottom => (ex - q.x) / q.w,
        };
        Some(Neighbor {
            id: q.id,
            key: q.key.clone(),
            entry_edge,
            entry_frac: entry_frac.clamp(0.0, 1.0),
        })
    }
}

/// `v` within `[lo, hi]` (inclusive), tolerant of ordering noise.
fn within(v: f32, lo: f32, hi: f32) -> bool {
    v >= lo - f32::EPSILON && v <= hi + f32::EPSILON
}

/// Build the runtime cursor-teleport layout from the persisted placements
/// (`preferences.json` → `outputs_layout`). Only ACTIVE (not user-deactivated) AND
/// CONNECTED monitors' placements enter the live map; inactive/disconnected ones stay
/// in preferences but are dropped here (so reactivating/replugging restores them in
/// place). Empty when no layout is set (single-monitor default → the pointer clamps).
pub fn build_teleport(
    prefs: &compositor_developer_environment_preference_base::base::Preference,
    connected_keys: &[String],
) -> TeleportLayout {
    use compositor_developer_environment_preference_base::base::output_active;
    TeleportLayout::new(
        prefs
            .outputs_layout
            .iter()
            .filter(|p| {
                connected_keys.iter().any(|k| k == &p.identity) && output_active(&prefs.outputs, &p.identity)
            })
            .map(|p| Placement { id: p.id, key: p.identity.clone(), x: p.x, y: p.y, w: p.w, h: p.h })
            .collect(),
        prefs.teleport_cyclic,
    )
}

/// The live cursor-teleport layout (rebuilt on activate/deactivate/hotplug + layout
/// commit). Read by the pointer-motion path to cross the cursor between monitors.
pub static TELEPORT_LAYOUT: Token<TeleportLayout> = Token::new();
pub static TELEPORT_LAYOUT_MUT: TokenMut<TeleportLayout> = TokenMut::new(&TELEPORT_LAYOUT);

/// The teleport placement the cursor currently occupies (disambiguates duplicate
/// placements of one monitor). `None` until the pointer resolves it; reset on reconcile.
pub static CURSOR_PLACEMENT: Token<Option<u64>> = Token::new();
pub static CURSOR_PLACEMENT_MUT: TokenMut<Option<u64>> = TokenMut::new(&CURSOR_PLACEMENT);

/// Teleport-suppression LOCK COUNTER (a refcount). The pointer-motion path suppresses
/// cursor teleportation between monitors while this is > 0 — it knows nothing about WHY.
/// Any world system may acquire (increment) / release (decrement) it around an operation
/// that must pin the cursor to its output (e.g. a canvas pan holds it for the pan's
/// duration). A counter, not a bool, so overlapping lockers compose. Lives in WORLD
/// storage (written by systems via `cx.storage`; read by the rim on the spawn-target
/// world). Balanced acquire/release is each locker's responsibility.
pub static TELEPORT_SUPPRESS: Token<u32> = Token::new();
pub static TELEPORT_SUPPRESS_MUT: TokenMut<u32> = TokenMut::new(&TELEPORT_SUPPRESS);
