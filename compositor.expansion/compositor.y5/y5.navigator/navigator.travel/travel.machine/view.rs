use std::collections::HashMap;
use smithay::desktop::Window;
use smithay::utils::{Logical, Point, Rectangle, Size, SERIAL_COUNTER};
use compositor_support_action_camera_find_base::find::{Direction, WindowEntry, WindowId};
use compositor_support_action_camera_fit_element::element::{compute_placement, CameraPlacementFlags, PlacementResult};
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_window_interface_record::window::LoopWindow;

pub struct Results {
    pub position: Option<(f64, f64)>,
    pub zoom: Option<f64>,
}

// Issue 1:
// A modifier is needed to determine whether to Fit, with Fit, movement should occur only for remaining points. ( eg. it may move only on X instead of trying to center )
// WIth fit- it is an exact fit, it wont go to center

pub fn view(state: &mut Loop, elements: Vec<&Window>, fit_absolute: bool) -> Results{
    // The ACTIVE output (the one the camera being moved belongs to), not the
    // primary/first — otherwise on multi-monitor the fit is computed against the
    // wrong screen size. Matches the output `camera()`/`size_ctx_all()` resolve.
    let output = state.inner.current_output();
    let output_geom_i32 = state.inner.space_state().state
        .output_geometry(output)
        .unwrap_or_else(|| abort!("output has geometry"));
    let screen_size: Size<f64, Logical> = output_geom_i32.size.to_f64();

    // ---- 2. Outputs vec for finder, in WORLD coords ------------------------
    // World == Smithay space. Camera (position, zoom) maps world → screen at
    // render time via world_to_space. The viewport in world coords is the
    // inverse: a rect of size screen/zoom, centered on camera.position.
    // let viewport_world = Rectangle::new(
    //     Point::from((
    //         state.camera.position().x - screen_size.w / (2.0 * state.camera.zoom()),
    //         state.camera.position().y - screen_size.h / (2.0 * state.camera.zoom()),
    //     )),
    //     Size::from((
    //         screen_size.w / state.camera.zoom(),
    //         screen_size.h / state.camera.zoom(),
    //     )),
    // );

    let mut id_map: HashMap<WindowId, smithay::desktop::Window> =
        HashMap::with_capacity(elements.len());
    let mut windows: Vec<WindowEntry> = Vec::with_capacity(elements.len());

    if elements.is_empty(){
        return Results{
            zoom: None,
            position: None,
        }
    }

    for w in &elements {
        let rect_space = state.inner.space_state().state
            .element_geometry(w)
            .unwrap_or_else(|| abort!("element has geometry"))
            .to_f64();

        let id = w.uuid();
        if id.is_none(){
        }
            continue;
        let id = id.unwrap();

        // window_id(w);
        id_map.insert(id, w.clone().clone());
        windows.push(WindowEntry { id, rect: rect_space });
    }



    let bbox: Rectangle<f64, Logical> = elements
        .iter()
        .filter_map(|b| {
            state.inner.space_state().state
                .element_bbox(b)
                .map(|rect| rect.to_f64())
        })
        .reduce(|acc, rect| acc.merge(rect)).unwrap();

    let mut set =  CameraPlacementFlags::PAN_CENTER
        | CameraPlacementFlags::ZOOM_IN_TO_FIT
        | CameraPlacementFlags::ZOOM_FIT_HORIZONTAL
        | CameraPlacementFlags::ZOOM_FIT_VERTICAL;

    if fit_absolute {
        set = set | CameraPlacementFlags::ZOOM_OUT_TO_FIT
    };
    
    let placement = compute_placement(
        set,
        bbox,
        PlacementResult {
            position: *state.inner.camera_mut().transform.position(),
            zoom:     *state.inner.camera_mut().transform.zoom(),
        },
        screen_size,
        Direction::Up, // <-- No OP for specified center and non-dominance flags.
    );

    trace!("Results are in: {:?}", placement);
    return Results {
        position: Some((placement.position.x, placement.position.y)),
        zoom: Some(placement.zoom),
    };

}
