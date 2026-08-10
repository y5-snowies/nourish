//! Whether the active bundle's pointer warp applies to THIS event.
//!
//! A warp is published by whichever bundle the world is running, and it describes
//! the displacement that bundle drew. Three states put something else on screen
//! entirely, and correcting the pointer for a displacement nobody drew is worse
//! than not correcting it at all:
//!
//! * the **world picker** — its own overlay world, with its own background;
//! * the **lock** screen — likewise;
//! * the **overview** — the grid is drawn over a frozen backdrop capture, so the
//!   live bundle is not what the user is pointing at.
//!
//! One predicate, three readers (motion, pan, the cursor sprite). They must agree:
//! a frame where motion warps and the sprite does not is a cursor that renders
//! away from where it clicks, which is the exact failure the warp exists to fix.
//!
//! Hit testing deliberately does NOT consult this. It reads the pointer's world
//! position like everything else and stays unaware that a warp happened.

use compositor_orchestration_core_state_base::Loop;
use compositor_y5_picker_system_base::base::PICKER_WORLD;

/// Whether the pointer warp applies right now.
///
/// `false` whenever no bundle published one, which is the overwhelmingly common
/// case and costs a single atomic read.
pub fn applies(state: &Loop) -> bool {
    bundle_warps(state) && !suppressed(state)
}

/// Whether the FOCUSED world's bundle displaces the pointer.
///
/// Read from that world's slot rather than a process global: a warp belongs to
/// whichever bundle a world is running, and a global could only ever describe one
/// of them — in practice the last one selected.
fn bundle_warps(state: &Loop) -> bool {
    let target = state.inner.worlds.spawn_target();
    state
        .inner
        .worlds
        .get(target)
        .storage()
        .try_get(&compositor_background_two_storage_base::base::BG_TWO)
        .and_then(|t| t.bundle())
        .is_some_and(compositor_pipeline_build_seam_base::base::warps)
}

/// Correct one point through the focused world's warp chain.
///
/// `&mut` because the world owns the baked map for `evaluate: "map_static"` and a
/// stale one is rebaked in place — per world, so two worlds cannot share a bake of
/// a shader only one of them is running.
pub fn apply(state: &mut Loop, u: f64, v: f64, res: [f32; 2]) -> (f64, f64) {
    let target = state.inner.worlds.spawn_target();
    // A world with no pipeline warps nothing, and this runs on EVERY motion event
    // — so answer that from one token read before touching anything else.
    if !compositor_pipeline_world_system_base::base::active(state.inner.worlds.get(target).storage()) {
        return (u, v);
    }
    // Read BEFORE the mutable borrow below: both live in this world's storage.
    // The pointer is on ONE output, and the geometry it hit-tests is that
    // output's — a set normalised to the other monitor's extent would place every
    // rect wrong. Two grid producers, reader picks: an offloaded bundle's comes
    // back through that pane's readback, an inline one lands in this world's slot.
    let out: std::sync::Arc<str> =
        std::sync::Arc::from(state.inner.current_output_key().as_str());
    let produced = compositor_pipeline_world_system_base::base::frame(state.inner.worlds.get(target).storage(), &out);
    let (w, inline) = (produced.world_set, produced.warp_grid);
    let two = state
        .inner
        .worlds
        .get_mut(target)
        .storage_mut()
        .try_get_mut(&compositor_background_two_storage_base::base::BG_TWO_MUT);
    let Some(two) = two else { return (u, v) };
    // The bundle AND the live param values it is currently being drawn with. Both
    // off the same instance: correcting the pointer with the declared defaults
    // while the picture is drawn from edited values makes the two different
    // functions, which is exactly what this whole path exists to prevent.
    let Some((cp, live)) = two
        .instance
        .as_ref()
        .and_then(|i| i.pipeline.clone().map(|cp| (cp, i.params)))
    else {
        return (u, v);
    };
    let grid = two
        .instance
        .as_ref()
        .and_then(|i| Some((i.worker.as_ref()?, i.pane.as_ref()?)))
        .and_then(|(wk, p)| wk.readback(p))
        .and_then(|r| r.warp_grid)
        .or(inline);
    compositor_pipeline_build_seam_base::base::warp_point(
        &cp, &live, &mut two.warp_map, w.as_deref(), grid.as_deref().map(|g| &g[..]), u, v, res,
    )
}

/// The three surfaces the live bundle did not draw.
fn suppressed(state: &Loop) -> bool {
    // The picker runs as its own world; when it is the active one, the bundle
    // whose warp is published is not what is on screen.
    if state.inner.worlds.active_id() == PICKER_WORLD {
        return true;
    }
    // The lock screen replaces the world's content wholesale.
    if matches!(
        state.inner.status,
        compositor_orchestration_core_state_base::state::Status::Locked { .. }
    ) {
        return true;
    }
    // The overview draws its grid over a FROZEN capture, so even the world's own
    // bundle is not producing what the pointer is over.
    state.inner.overview().visible
}
