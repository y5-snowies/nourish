use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{ImportAll, ImportMem, Texture};
use smithay::desktop::Window;
use smithay::utils::{Physical, Point, Rectangle, Size};
use compositor_y5_camera_transform_translate::transform::Transform;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_y5_window_interface_draw::bound::CalculateBoundResult;

pub fn scene<R>(
    state: &mut Loop,
    renderer: &mut R,
    size: Size<i32, Physical>,
    window: &Window,
    context: &compositor_y5_canvas_draw_context::context::Context,
    bound: &CalculateBoundResult,
) -> Vec<SolidColorRenderElement>
where
    R: smithay::backend::renderer::Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    // let header_logical_height = 30.0;
    let mut elements = Vec::new();

    // Choose color based on focus
    // let is_active = window.toplevel().map(|t| t.current_state().activated).unwrap_or(false);
    let is_active = state.inner.select().get(window.clone());
    let is_primary = state.inner.select().primary(window.clone());
    let (color, bw_logical) = if is_active {
        if is_primary {
            ([0.0, 0.0, 1.0, 1.0], 12.0)
        } else {
            ([0.0, 0.5, 1.0, 1.0], 6.0)
        }
    } else {
        ([0.2, 0.2, 0.2, 0.0], 3.0)
    };

    // let ctx = state.viewport_context();
    // let bbox = state
    //     .state
    //     .space
    //     .state
    //     .element_bbox(window)
    //     .unwrap_or_default();
    // let t: Transform = (bbox, ctx).into();
    // let render_at: Point<i32, Physical> = t.into();
    // let render_with: Rectangle<i32, Physical> = t.into();

    // It probably shouldn't scale the border with zoom? check now
    // let bw_screen = scale(&state.inner.camera_mut().transform, bw_logical);

    // let top_rect = smithay::utils::Rectangle::from_loc_and_size(
    //     (render_with.loc.x, render_with.loc.y - 1.0),
    //     (render_with.size.w, bw_screen),
    // );

    let ctx = state.viewport_context();

    // Frame the compositor-decided **slot** (`bound.Screen`, already projected from the slot
    // rect in `bound::calculate`), NOT `element_geometry`. The slot is the region the window
    // content is letterboxed into and the rect other interfaces key off, so the border must
    // match it exactly regardless of how the client misbehaves.
    let render_with: Rectangle<i32, Physical> = Rectangle::new(
        Point::from((bound.Screen.Left.round() as i32, bound.Screen.Top.round() as i32)),
        Size::from((bound.Screen.Width.round() as i32, bound.Screen.Height.round() as i32)),
    );

    let bw_screen = bw_logical * ctx.scale;

    let mut top_rect = render_with.clone();
    top_rect.loc.y -= 1;
    top_rect.size.h = bw_screen.round() as i32;

    let mut bottom_rect = render_with.clone();
    bottom_rect.loc.y = bottom_rect.loc.y + bottom_rect.size.h;
    bottom_rect.size.h = bw_screen.round() as i32;

    let mut left_rect = render_with.clone();
    left_rect.loc.x -= 1;
    left_rect.size.w = bw_screen.round() as i32;

    let mut right_rect = render_with.clone();
    right_rect.loc.x += right_rect.size.w;
    right_rect.size.w = bw_screen.round() as i32;

    // let mut top_rect = render_with.clone();
    // top_rect.loc.y -= 1;
    // top_rect.size.h = bw_screen.round() as i32;

    // let mut bottom_rect = render_with.clone();
    // bottom_rect.loc.y = bottom_rect.loc.y + bottom_rect.size.h;
    // bottom_rect.size.h = bw_screen.round() as i32;

    // let mut left_rect = render_with.clone();
    // left_rect.loc.x -= 1;
    // left_rect.size.w = bw_screen.round() as i32;

    // let mut right_rect = render_with.clone();
    // right_rect.loc.x += right_rect.size.w;
    // right_rect.size.w = bw_screen.round() as i32;

    // let bottom_rect = smithay::utils::Rectangle::from_loc_and_size(
    //     (bound.Screen.Left, bound.Screen.Top),
    //     (bound.Screen.Width, bw_screen),
    // );

    // let left_rect = smithay::utils::Rectangle::from_loc_and_size(
    //     (bound.Screen.Left - 1.0, bound.Screen.Top),
    //     (bw_screen, bound.Screen.Height),
    // );
    // let right_rect = smithay::utils::Rectangle::from_loc_and_size(
    //     (bound.Screen.Right, bound.Screen.Top),
    //     (bw_screen, bound.Screen.Height),
    // );

    // When drawing a split/floating viewport pane, clip the borders to the pane's
    // physical rect so they don't bleed past it (full-output render → no clip).
    let pane = state.inner.render_target.map(|rt| {
        Rectangle::new(
            Point::from(((rt.origin_logical.0 * ctx.scale).round() as i32, (rt.origin_logical.1 * ctx.scale).round() as i32)),
            Size::from((rt.size_physical.0.round() as i32, rt.size_physical.1.round() as i32)),
        )
    });

    // 3. Create and push elements
    for rect in [top_rect, left_rect, right_rect, bottom_rect] {
        let rect = match pane {
            Some(p) => match rect.intersection(p) {
                Some(clipped) => clipped,
                None => continue,
            },
            None => rect,
        };
        elements.push(SolidColorRenderElement::new(
            Id::new(),
            rect,
            CommitCounter::default(),
            color,
            Kind::Unspecified,
        ));
    }

    // // Create a logical bounding box for the trigger zone (sitting right on top of the window)
    let trigger_zone_world: Rectangle<f64, Physical> = smithay::utils::Rectangle::from_loc_and_size(
        (bound.World.Left, bound.World.Top - bw_screen),
        (bound.Box.size.w as f64, bw_screen),
    );

    //
    // // Check if the logical cursor is inside this trigger zone
    if trigger_zone_world.contains(Point::new(
        context.cursor.position.x,
        context.cursor.position.y,
    )) {
        // 3. --- RENDER THE HEADER ---
        // Calculate physical placement, just like we did for windows
        // let world_offset_x = bound.World.Left as f64 - state.inner.camera_mut().position.x;
        // let world_offset_y = (bound.World.Top as f64 - header_logical_height) - state.inner.camera_mut().position.y;
        // let world_offset_x = bound.World.Left as f64 - state.inner.camera_mut().position.x;
        // let world_offset_y = (bound.World.Top as f64 - header_logical_height) - state.inner.camera_mut().position.y;
        //
        // let scaled_x = world_offset_x * state.inner.camera_mut().zoom;
        // let scaled_y = world_offset_y * state.inner.camera_mut().zoom;

        // let screen_x = scaled_x + (size.w as f64 / 2.0);
        // let screen_y = scaled_y + (size.h as f64 / 2.0);

        // Scale the physical dimensions of the header
        // let physical_width =
        //     bw_screen as i32 + ((bound.Box.size.w) as f64 * state.inner.camera_mut().transform.zoom()) as i32;
        // let physical_height = (header_logical_height * state.inner.camera_mut().transform.zoom()) as i32;
        //
        // let screen_x = bound.Screen.Left;
        // let screen_y = bound.Screen.Top - physical_height as f64;
        //
        // let header_rect = smithay::utils::Rectangle::from_loc_and_size(
        //     (screen_x as i32, screen_y as i32),
        //     (physical_width, physical_height),
        // );
        //
        // let header_element = SolidColorRenderElement::new(
        //     Id::new(),
        //     header_rect,
        //     CommitCounter::default(),
        //     [0.15, 0.15, 0.15, 0.9], // RGBA color
        //     Kind::Unspecified,
        // );
        //
        // // Add it to the elements list so it renders!
        // elements.push(header_element);
    }
    return elements;
}
