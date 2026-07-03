use smithay::backend::renderer::{ImportAll, ImportMem, Texture};
use smithay::desktop::Window;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};
use compositor_y5_camera_transform_translate::slot;
use compositor_y5_camera_transform_translate::transform::Transform;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;

#[derive(Debug)]
pub struct Bound {
    // pub Top: Rectangle<f64, Logical>,
    // pub Left: Rectangle<f64, Logical>,
    // pub Right: Rectangle<f64, Logical>,
    // pub Bottom: Rectangle<f64, Logical>,
    pub Top: f64,
    pub Left: f64,
    pub Right: f64,
    pub Width: f64,
    pub Height: f64,
    pub Bottom: f64,
}

pub struct CalculateBoundResult {
    pub Screen: Bound,

    // CHECK: World is wrong here
    pub World: Bound,
    pub Box: Rectangle<i32, Logical>,
}

pub fn calculate<R>(
    state: &mut Loop,
    renderer: &mut R,
    size: Size<i32, Physical>,
    window: &Window,
    context: &compositor_y5_canvas_draw_context::context::Context,
) -> (CalculateBoundResult)
where
    R: smithay::backend::renderer::Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    let ctx = state.viewport_context();

    // Recognized bounds = the compositor-decided **slot** (the size the window is enforced at,
    // and the rect content is letterboxed into), so decoration borders frame the slot rather
    // than the client's (possibly misbehaving) committed geometry. Mirrors the render/input
    // slot logic. Falls back to element_geometry when no size is decided (0x0 defer).
    let cfg = compositor_developer_environment_config_base::base::get();
    let elem_loc = state
        .inner.space_state()
        .state
        .element_location(window)
        .unwrap_or_default();
    let slot_size = if cfg.window_client_size_fallback {
        window
            .toplevel()
            .and_then(|t| t.with_pending_state(|s| s.size))
            .filter(|s| s.w > 0 && s.h > 0)
            .or_else(|| Some(window.geometry().size))
    } else {
        slot::expected_size(window)
    };
    let geom = match slot_size.filter(|s| s.w > 0 && s.h > 0) {
        Some(size) => smithay::utils::Rectangle::new(elem_loc, size),
        None => state
            .inner.space_state()
            .state
            .element_geometry(window)
            .unwrap_or_default(),
    };

    // World bounds: directly from the geometry, no projection needed.
    let world = Bound {
        Top: geom.loc.y as f64,
        Left: geom.loc.x as f64,
        Right: (geom.loc.x + geom.size.w) as f64,
        Bottom: (geom.loc.y + geom.size.h) as f64,
        Width: geom.size.w as f64,
        Height: geom.size.h as f64,
    };

    // Screen bounds: project the slot by its **corners** (each rounded once), identical to the
    // content crop in `window.draw.frame::scene`, so the border tracks the content exactly and an
    // edge at a fixed world coordinate (e.g. the right edge during a resize-from-left) stays put
    // instead of wobbling ±1px — which happens if you project the loc and add a separately-rounded
    // scaled width.
    let tl_t: Transform = (geom.loc, ctx).into();
    let tl: Point<i32, Physical> = tl_t.into();
    let br_t: Transform = (geom.loc + Point::from((geom.size.w, geom.size.h)), ctx).into();
    let br: Point<i32, Physical> = br_t.into();

    let screen = Bound {
        Top: tl.y as f64,
        Left: tl.x as f64,
        Right: br.x as f64,
        Bottom: br.y as f64,
        Width: (br.x - tl.x) as f64,
        Height: (br.y - tl.y) as f64,
    };

    return CalculateBoundResult {
        Box: geom,
        World: world,
        Screen: screen,
    };
}

// pub fn calculate<R>(
//     state: &mut Loop,
//     renderer: &mut R,
//     size: Size<i32, Physical>,
//     window: &Window,
//     context: &compositor_y5_canvas_draw_context::context::Context,
// ) -> (CalculateBoundResult)
// where
//     R: smithay::backend::renderer::Renderer + ImportAll + ImportMem,
//     R::TextureId: Texture + Clone + Send + 'static,
// {
//     let loc = state
//         .state
//         .space
//         .state
//         .element_location(window)
//         .unwrap_or_default();
//     let window_bbox = state
//         .state
//         .space
//         .state
//         .element_bbox(window)
//         .unwrap_or_default();

//     let win_w = window_bbox.size.w as f64;
//     let win_h = window_bbox.size.h as f64;

//     let win_top_left = global_to_canvas(
//         &state.inner.camera_mut().transform,
//         Size::new(size.w as f64, size.h as f64),
//         Point::new(loc.x as f64, loc.y as f64),
//         state.inner.space_state().default_scale(),
//     );

//     let win_bot_right = global_to_canvas(
//         &state.inner.camera_mut().transform,
//         Size::new(size.w as f64, size.h as f64),
//         Point::new(loc.x as f64 + win_w, loc.y as f64 + win_h),
//         state.inner.space_state().default_scale(),
//     );

//     // let world_width = scale(&state.inner.camera_mut().transform, win_w);
//     // let world_height = scale(&state.inner.camera_mut().transform, win_h);
//     // let world_offset_x = loc.x as f64;
//     // let world_offset_y = loc.y as f64;

//     let world: Transform = (window_bbox, state.viewport_context()).into();
//     // let world_transform = world_to_screen(&state.camera, Size::new(size.w as f64, size.h as f64), Point::new(
//     //     world_offset_x,
//     //     world_offset_y,
//     // ));

//     let world: Rectangle<f64, Logical> = world.into();
//     let world = Bound {
//         Top: world.loc.y,
//         Left: world.loc.x,
//         Right: world.loc.x + world.size.w,
//         Bottom: world.loc.y + world.size.h,
//         Width: world.size.w,
//         Height: world.size.h,
//         // Top: world_transform.y,
//         // Left: world_transform.x,
//         // Right: world_transform.x + world_width,
//         // Bottom: world_transform.y + world_height,
//     };
//     // let world_offset_x = loc.x as f64 - state.camera.position.x;
//     // let world_offset_y = loc.y as f64 - state.camera.position.y;
//     //
//     // let scaled_x = world_offset_x * state.camera.zoom;
//     // let scaled_y = world_offset_y * state.camera.zoom;
//     //
//     // let screen_x = scaled_x + (size.w as f64 / 2.0);
//     // let screen_y = scaled_y + (size.h as f64 / 2.0);

//     // let physical_loc = smithay::utils::Point::from((screen_x as i32, screen_y as i32));

//     return CalculateBoundResult {
//         Box: window_bbox,
//         World: world,
//         Screen: Bound {
//             // Top: screen_rect.y() as f64,
//             // Left: screen_rect.x() as f64,
//             // Right: screen_rect.right() as f64,
//             // Bottom: screen_rect.bottom() as f64,
//             // Width: screen_rect.w() as f64,
//             // Height: screen_rect.h() as f64,
//             Top: win_top_left.y,
//             Left: win_top_left.x,
//             Right: win_bot_right.x,
//             Bottom: win_bot_right.y,
//             Width: win_bot_right.x - win_top_left.x,
//             Height: win_bot_right.y - win_top_left.y,
//         },
//     };
// }
