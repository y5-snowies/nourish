use compositor_model_debug_instance_record::abort;
use compositor_orchestration_core_state_base::Loop;
use compositor_remote_message_client_base::bind::selection::{FitAspect, FitAspectResponse};
use compositor_support_action_camera_fit_aspect::aspect;
use compositor_y5_window_lifecycle_interface::interface::TransformUpdate;
use smithay::utils::{Logical, Point, Size};

pub fn fit_aspect(request: FitAspect, state: &mut Loop) -> FitAspectResponse {
    if state.inner.select().Selection.len() != 1 {
        return FitAspectResponse {};
    }

    let first = state.inner.select().Selection.first().unwrap().clone();

    // Scale and reposition.
    let window = first.as_ref().clone();
    let window = &window;

    let window_geometry = state.inner.space_state().state.element_geometry(window);
    if window_geometry.is_none() {
        return FitAspectResponse {};
    }
    let window_geometry = window_geometry.unwrap();
    let window_size = window_geometry.size;
    let window_location = window_geometry.loc;

    let output = state
        .inner.space_state()
        .state
        .outputs()
        .next()
        .unwrap_or_else(|| abort!("at least one output"));
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
            scale_to_perceived: request.perceived,
            maximize: request.max,
        },
    );

    if !request.vertical {
        element_size.h = window_size.h as f32;
        element_position.y = window_location.y as f32;
    }
    if !request.horizontal {
        element_size.w = window_size.w as f32;
        element_position.x = window_location.x as f32
    }

    let element_position = Point::new(
        element_position.x.ceil() as i32,
        element_position.y.ceil() as i32,
    );

    let new_size = Size::new(element_size.w.ceil() as i32, element_size.h.ceil() as i32);

    compositor_y5_window_lifecycle_interface::interface::reform(state, window.clone(), TransformUpdate {
        position: Some(element_position),
        size: Some(new_size)
    });

    FitAspectResponse {}
}
