use std::time::Instant;

use smithay::{
    desktop::Window,
    utils::{Logical, Point, Rectangle, Size},
};
use compositor_support_action_camera_find_base::find::Direction;
use compositor_support_action_camera_find_origin::distance_sq_to_viewport_center;
use compositor_support_action_camera_find_window::cmp_f64;
use compositor_support_action_camera_fit_aspect::aspect;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_navigator_travel_machine::view::view;
use compositor_y5_navigator_travel_state::state::{Target, Travel};
use compositor_y5_window_lifecycle_interface::interface::TransformUpdate;

pub fn move_direction(s: &mut Loop, direction: Direction, alternative: bool) {
    let result = compositor_y5_navigator_travel_machine::view_directional::view_directional(
        s,
        direction,
        alternative,
    );

    let zoom = result.zoom.and_then(|target| {
        return Some(Target {
            start: None,
            target: result.zoom.unwrap(),
        });
    });

    let position = result.position.and_then(|target| {
        return Some(Target {
            start: None,
            target: result.position.unwrap(),
        });
    });

    let travel = Travel {
        position: position,
        duration: None,
        zoom: zoom,
        time_start: None,
    };

    compositor_y5_navigator_state_base::state::request(
        s.inner.focus_channels(),
        compositor_y5_navigator_state_base::state::NavRequest::Set(
            compositor_y5_navigator_state_base::state::State::Travel(travel),
        ),
    );
}

pub fn fit(state: &mut Loop, zoom_1: bool, fit_1: bool) {
    let focused = focused(state);
    let selected = state.inner.select().Selection.clone();

    let windows: Vec<&Window> = if !selected.is_empty() {
        selected.iter().map(|w| w.as_ref()).collect()
    } else if let Some(ref focused) = focused {
        vec![focused]
    } else {
        let result = vec![];
        result
    };

    if windows.is_empty() {
        return;
    }
    if fit_1 && windows.len() != 1 {
        return;
    }

    let travel = if fit_1 {
        let window = windows.first().unwrap().clone();
        let window_geometry = state.inner.space_state().state.element_geometry(window);
        if window_geometry.is_none() {
            return;
        }
        let window_geometry = window_geometry.unwrap();
        let window_size = window_geometry.size;
        let window_location = window_geometry.loc;

        // The ACTIVE output (the one whose camera is being fitted), not the
        // primary/first — otherwise on multi-monitor the fit is computed against
        // the wrong screen size. Matches `camera()`/`size_ctx_all()`.
        let output = state.inner.current_output();
        let output_geom_i32 = state
            .inner.space_state()
            .state
            .output_geometry(output)
            .unwrap_or_else(|| abort!("output has geometry"));
        let screen_size: Size<f64, Logical> = output_geom_i32.size.to_f64();

        let total_size = aspect::Size {
            w: screen_size.w as f32,
            h: screen_size.h as f32,
        };

        let perceived_size = aspect::Size {
            w: screen_size.w as f32 / *state.inner.camera_mut().transform.zoom() as f32,
            h: screen_size.h as f32 / *state.inner.camera_mut().transform.zoom() as f32,
        };

        let element_size = aspect::Size {
            h: window_size.h as f32,
            w: window_size.w as f32,
        };

        let element_position = aspect::Point {
            x: window_location.x as f32,
            y: window_location.y as f32,
        };

        let (mut element_size, mut element_position) = aspect::fit_aspect_ratio(
            total_size,
            perceived_size,
            element_size,
            aspect::Origin::TopLeft,
            element_position,
            aspect::Flags {
                scale_to_perceived: false,
                maximize: true,
            },
        );

        let element_position = Point::new(
            element_position.x.ceil() as i32,
            element_position.y.ceil() as i32,
        );

        let new_size = Size::new(element_size.w.ceil() as i32, element_size.h.ceil() as i32);

        let transform_update = TransformUpdate {
            position: Some(element_position),
            size: Some(new_size),
        };
        compositor_y5_window_lifecycle_interface::interface::reform(
            state,
            window.clone(),
            transform_update,
        );

        Travel {
            duration: None,
            position: Some(Target {
                start: None,
                target: (
                    element_position.x as f64 + (element_size.w / 2.0) as f64,
                    element_position.y as f64 + (element_size.h / 2.0) as f64,
                ),
            }),
            zoom: Some(Target {
                start: None,
                target: 1.0,
            }),
            time_start: None,
        }
    } else {
        let result = compositor_y5_navigator_travel_machine::view::view(state, windows, false);
        let zoom = result.zoom.and_then(|target| {
            let target = if (zoom_1) { 1.0 } else { target };
            return Some(Target {
                start: None,
                target,
            });
        });

        let position = result.position.and_then(|target| {
            return Some(Target {
                start: None,
                target: result.position.unwrap(),
            });
        });

        Travel {
            duration: None,
            position: position,
            zoom: zoom,
            time_start: None,
        }
    };
    // get the view optimal to fit into the screen

    compositor_y5_navigator_state_base::state::request(
        state.inner.focus_channels(),
        compositor_y5_navigator_state_base::state::NavRequest::Set(
            compositor_y5_navigator_state_base::state::State::Travel(travel),
        ),
    );
}

/// Travel the camera to fit a single specific window (e.g. the overview cell the
/// user clicked). Unlike `fit_window`, the target window is explicit rather than
/// derived from the selection/focus.
pub fn fit_to_window(state: &mut Loop, window: &Window) {
    let result = view(state, vec![window], false);
    travel(state, result.position, result.zoom);
}

/// Four-finger pinch IN: frame the focused window — or, if nothing is focused or
/// selected, the most-centered visible window — like Super+Alt+F with a centered
/// fallback. Eases there via the navigator (one-shot, like the swipe handler).
pub fn fit_window(state: &mut Loop) {
    let selected = state.inner.select().Selection.clone();
    let windows: Vec<Window> = if !selected.is_empty() {
        selected.iter().map(|w| w.as_ref().clone()).collect()
    } else if let Some(window) = focused(state) {
        vec![window]
    } else if let Some(window) = most_centered(state) {
        vec![window]
    } else {
        return;
    };
    let refs: Vec<&Window> = windows.iter().collect();
    let result = view(state, refs, false);
    travel(state, result.position, result.zoom);
}

/// Four-finger pinch OUT: zoom out to an overview framing every (toplevel) window.
/// Zoom is capped at 1.0 so the view never zooms IN past the viewport — the most
/// zoomed-out floor is the screen itself (the user's "minimum zoom out").
pub fn fit_all(state: &mut Loop) {
    let windows: Vec<Window> = state
        .inner.space_state().state
        .elements()
        .filter(|w| w.toplevel().is_some())
        .cloned()
        .collect();
    if windows.is_empty() {
        return;
    }
    let refs: Vec<&Window> = windows.iter().collect();
    let result = view(state, refs, true);
    let zoom = result.zoom.map(|z| z.min(1.0));
    travel(state, result.position, zoom);
}

/// Issue a one-shot navigator travel to an optional target position/zoom.
fn travel(state: &mut Loop, position: Option<(f64, f64)>, zoom: Option<f64>) {
    let travel = Travel {
        duration: None,
        time_start: None,
        position: position.map(|target| Target { start: None, target }),
        zoom: zoom.map(|target| Target { start: None, target }),
    };
    compositor_y5_navigator_state_base::state::request(
        state.inner.focus_channels(),
        compositor_y5_navigator_state_base::state::NavRequest::Set(
            compositor_y5_navigator_state_base::state::State::Travel(travel),
        ),
    );
}

/// The most-centered visible window: smallest squared distance from its geometry
/// center to the output center (the action.camera.find metric, used directly).
fn most_centered(state: &mut Loop) -> Option<Window> {
    let output_rects: Vec<Rectangle<f64, Logical>> = {
        let outputs: Vec<_> = state.inner.space_state().state.outputs().cloned().collect();
        outputs
            .iter()
            .filter_map(|o| state.inner.space_state().state.output_geometry(o).map(|r| r.to_f64()))
            .collect()
    };
    if output_rects.is_empty() {
        return None;
    }
    let windows: Vec<Window> = state
        .inner.space_state().state
        .elements()
        .filter(|w| w.toplevel().is_some())
        .cloned()
        .collect();
    windows
        .into_iter()
        .filter_map(|w| {
            let rect = state.inner.space_state().state.element_geometry(&w)?.to_f64();
            Some((w, distance_sq_to_viewport_center(&rect, &output_rects)))
        })
        .min_by(|a, b| cmp_f64(a.1, b.1))
        .map(|(w, _)| w)
}

fn focused(state: &mut Loop) -> Option<Window> {
    let focused: Option<&Window> = {
        state
            .state
            .seat
            .seat
            .get_keyboard()
            .and_then(|kb| kb.current_focus())
            .and_then(|focus_surface| {
                state.inner.space_state().state.elements().find(|w| {
                    w.toplevel()
                        .and_then(|t| Some(t.wl_surface()))
                        .map(|s| s == &focus_surface)
                        .unwrap_or(false)
                })
            })
            .map_or(None, |w| Some(w))
    };
    return focused.cloned();
}

/// Sets the navigator into lock mode - not accepting further state.
pub fn lock(state: &mut Loop) {
    let view_result = {
        let windows: Vec<Window> = state.inner.space_state().state.elements().cloned().collect();

        // 2. Create the vector of references pointing to your local copy, NOT to `state`
        let windows: Vec<&Window> = windows.iter().collect();
        let view_result = { view(state, windows, true) };

        // use view with all windows selected
        view_result
    };

    const MIN_ZOOM: f64 = 0.1;
    const MAX_ZOOM: f64 = 0.75;
    // Set a minimum zoom of 0.25 with maximum zoom of 0.75
    let pending_travel = compositor_y5_navigator_travel_state::state::Travel {
        duration: Some(compositor_y5_lock_state_transition::transition::PERIOD_NAVIGATOR_DURATION),
        time_start: Some(Instant::now()),
        position: view_result.position.map(|a| Target {
            start: None,
            target: a,
        }),
        zoom: view_result.zoom.map(|mut a| {
            if a < MIN_ZOOM {
                a = MIN_ZOOM;
            } else if a > MAX_ZOOM {
                a = MAX_ZOOM
            }

            Target {
                start: None,
                target: a,
            }
        }),
    };

    let set_transform = state.inner.camera().transform.clone();
    // Apply directly on the focused (spawn-target) world's navigator instead of
    // announcing on its channel. Locking immediately switches the ACTIVE world to
    // LOCK_WORLD while the spawn-target world keeps owning this navigator, so the
    // spawn-target world stops being dispatched — a queued NAV_REQUEST would not be
    // drained until that world is reactivated at unlock, landing a stale Lock AFTER
    // unlock and wedging the machine. navigator_mut() is the transitional direct path.
    state.inner.navigator_mut().lock(compositor_y5_navigator_lock_state::state::NavigatorLock {
        set_transform,
        pending_travel: Some(pending_travel),
        transition_start: Instant::now(),
    });
}
/// Sets the navigator into lock mode - not accepting further state.

pub fn unlock(state: &mut Loop) {
    let view_result = {
        let compositor_y5_navigator_state_base::state::State::Lock(LockState) =
            state.inner.navigator().state()
        else {
            return;
        };

        LockState.set_transform.clone()
    };

    let pending_travel = compositor_y5_navigator_travel_state::state::Travel {
        duration: Some(compositor_y5_lock_state_transition::transition::PERIOD_NAVIGATOR_DURATION),
        time_start: Some(Instant::now()),

        position: Some(Target {
            start: None,
            target: (view_result.position.x, view_result.position.y),
        }),
        zoom: Some(Target {
            start: None,
            target: view_result.zoom,
        }),
    };

    // Mirror lock(): drive the focused world's navigator directly rather than via
    // the channel. unlock() runs the frame the spawn-target world is reactivated,
    // before its next dispatch, so a queued NAV_REQUEST::Unlock would not be applied
    // in time and the read above would still observe Lock. unlock() leaves Lock so
    // the following set() (ignored while Lock) lands the travel-home transition.
    let navigator = state.inner.navigator_mut();
    navigator.unlock();
    navigator.set(compositor_y5_navigator_state_base::state::State::Travel(pending_travel));
}
