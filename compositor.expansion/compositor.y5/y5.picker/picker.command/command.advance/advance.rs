//! The picker sphere's per-frame orientation step, shared by BOTH renderers of
//! it: the full-screen picker (`picker.scene/scene.tick`) and the overview's
//! embedded World tab (`overview.draw/draw.world`). They had a copy each, so a
//! fix to one silently left the other behind — this is the single copy.

use std::time::Instant;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};
use compositor_y5_picker_three_constant::{APPROACH_RATE, SPIN_DECAY};
use compositor_y5_picker_three_orient::orient;

/// Longest frame the step will integrate, in seconds. A stalled or first frame
/// otherwise flings the globe by however long the compositor was busy.
const DT_MAX: f32 = 0.1;

/// Advance the sphere one frame — play out drag-release momentum, else glide
/// toward the selection target — and push the result to the bevy scene.
///
/// TIME-STEPPED, not frame-counted: the elapsed wall time since the last call
/// drives both, so the globe spins and settles at the same rate whatever the
/// refresh rate. `step` is refreshed even while dragging (where nothing is
/// integrated), so releasing a long drag doesn't hand the next frame a dt
/// covering the whole drag.
pub fn advance(state: &mut Loop) {
    let now = Instant::now();
    if let Some(a) =
        state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT).active.as_mut()
    {
        let dt = (now - a.step).as_secs_f32().clamp(0.0, DT_MAX);
        a.step = now;
        if a.drag.is_none() {
            if orient::spinning(a.spin) {
                let (o, s) = orient::momentum(a.orientation, a.spin, SPIN_DECAY, dt);
                (a.orientation, a.spin, a.target) = (o, s, o);
            } else {
                a.orientation = orient::approach(a.orientation, a.target, APPROACH_RATE, dt);
            }
        }
    }
    compositor_y5_picker_command_base::base::push_transform(state);
}
