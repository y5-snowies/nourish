use compositor_support_library_ease_velocity_base::velocity::velocity::{Solver, solve};
use compositor_support_system_trait_system_base::base::SystemCx;
use compositor_y5_viewport_state_base::state::OUTPUT_VIEWS;
use compositor_y5_navigator_state_base::state::{NavigatorOutput, State};
use compositor_y5_navigator_tick_warp::warp::warp_intent;
use compositor_y5_navigator_travel_state::state::Travel;
use std::time::{Duration, Instant};

pub static CONFIG: Solver = Solver {
    duration: Duration::from_millis(500),
    stiffness: 154.0,
    damping: 9.54,
    mass: 0.25,
    value_start: 0.0,
    value_target: 0.0,
};

/// One easing tick. Pure with respect to the world: reads camera + kernel,
/// returns (next navigator state, eased output, pointer-warp intent).
pub fn travel_tick(
    cx: &SystemCx,
    mut machine: Travel,
) -> (State, Option<NavigatorOutput>, Option<(f64, f64)>) {
    if machine.position.is_none() && machine.zoom.is_none() {
        return (State::Idle, None, None);
    }

    let camera = cx.storage.get(&OUTPUT_VIEWS).current_views().focus_camera();
    let transform_previous = camera.transform.clone();

    let time_start = *machine.time_start.get_or_insert_with(Instant::now);
    let duration = machine
        .duration
        .map(Duration::from_secs_f64)
        .unwrap_or(CONFIG.duration);
    let time_elapsed = Instant::now() - time_start;

    let mut output = NavigatorOutput::default();

    if let Some(ref mut position) = machine.position {
        let start = *position.start.get_or_insert_with(|| {
            (camera.transform.position().x, camera.transform.position().y)
        });
        let x = solve(&Solver { value_start: start.0, value_target: position.target.0, duration, ..CONFIG }, time_elapsed);
        let y = solve(&Solver { value_start: start.1, value_target: position.target.1, duration, ..CONFIG }, time_elapsed);
        output.position = Some((x.unwrap_or(position.target.0), y.unwrap_or(position.target.1)));
        if x.is_none() && y.is_none() {
            machine.position = None;
        }
    }

    if let Some(ref mut zoom) = machine.zoom {
        let start = *zoom.start.get_or_insert_with(|| *camera.transform.zoom());
        let z = solve(&Solver { value_start: start, value_target: zoom.target, duration, ..CONFIG }, time_elapsed);
        output.zoom = Some(z.unwrap_or(zoom.target));
        if z.is_none() {
            machine.zoom = None;
        }
    }

    let warp = warp_intent(cx, &transform_previous, &output);
    (State::Travel(machine), Some(output), warp)
}

