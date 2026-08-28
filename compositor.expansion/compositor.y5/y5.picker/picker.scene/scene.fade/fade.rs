//! The picker's entry transition: one full-screen black overlay, drawn on both
//! sides of the world switch. It ramps to opaque over the OUTGOING world
//! (`leaving`, on the main scene) and clears again over the picker (`overlay`,
//! on the picker scene) — so the two halves meet on a black frame rather than
//! cutting. In place of the lock screen's morph.

use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::utils::{Physical, Point, Rectangle, Size};
use compositor_orchestration_draw_scene_identity::identity::SolidBank;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};
use compositor_y5_picker_three_constant::{FADE_OUT_SECS, FADE_SECS};

thread_local! {
    /// Slot 0 is the outgoing-world half, slot 1 the picker-side half. Two slots
    /// rather than one because the halves are drawn on different scenes and can
    /// both be live across a switch.
    ///
    /// This is the case that makes `SolidBank` pair an id with a commit counter
    /// instead of just caching the id: the rect is the full output every frame and
    /// only the alpha moves, so a frozen `CommitCounter::default()` would tell the
    /// damage tracker nothing changed and the ramp would stick on its first frame.
    static FADE: SolidBank = SolidBank::default();
}

fn black(slot: usize, size: Size<i32, Physical>, alpha: f32) -> SolidColorRenderElement {
    FADE.with(|bank| bank.solid(slot, Rectangle::new(Point::from((0, 0)), size), [0.0, 0.0, 0.0, alpha]))
}

/// First half — drawn on the MAIN scene while an open is pending: alpha ramps
/// 0 → 1 over `FADE_OUT_SECS`, darkening the world the picker is replacing.
/// `None` once nothing is pending. The switch itself waits for this to finish
/// (`picker.interface/interface.capture`), so it never cuts mid-ramp.
pub fn leaving(state: &mut Loop, size: Size<i32, Physical>) -> Option<SolidColorRenderElement> {
    let secs = state
        .inner
        .worlds
        .get_mut(PICKER_WORLD)
        .storage_mut()
        .get_mut(&PICKER_MUT)
        .fade_out
        .map(|t| t.elapsed().as_secs_f64())?;
    Some(black(0, size, (secs / FADE_OUT_SECS as f64).clamp(0.0, 1.0) as f32))
}

/// Second half — drawn on the PICKER scene: alpha ramps 1 → 0 over `FADE_SECS`,
/// or `None` once the fade has cleared / the picker isn't active.
pub fn overlay(state: &mut Loop, size: Size<i32, Physical>) -> Option<SolidColorRenderElement> {
    let secs = state
        .inner
        .worlds
        .get_mut(PICKER_WORLD)
        .storage_mut()
        .get_mut(&PICKER_MUT)
        .active
        .as_ref()
        .map(|a| a.time.elapsed().as_secs_f64())?;
    let p = (secs / FADE_SECS as f64).clamp(0.0, 1.0) as f32;
    (p < 1.0).then(|| black(1, size, 1.0 - p))
}
