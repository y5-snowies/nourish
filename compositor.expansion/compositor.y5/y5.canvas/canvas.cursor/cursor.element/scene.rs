use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{ImportAll, ImportMem, Texture};
use smithay::desktop::Window;
use smithay::utils::{Physical, Rectangle, Size};
use compositor_y5_canvas_draw_context::context::Context;
use compositor_orchestration_core_state_base::{Loop, Transform};
use compositor_orchestration_core_state_base::state::CoordinateTrait;

pub fn scene<R>(
    state: &mut Loop,
    renderer: &mut R,
    size: Size<i32, Physical>,
    context: &Context,
) -> (Vec<SolidColorRenderElement>)
where
    R: smithay::backend::renderer::Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    // Use logical size.
    // let size = state.inner.space_state().default_logical().size;
    // let size_scaling = state.inner.space_state().default_scale();

    // Cursor world position → Transform, then extract as physical Rectangle.
    // A "point" extracted as Rectangle gives a zero-sized rect at the
    // position, which won't work — we need to construct with size.
    //

    let cursor_size = 20.0;
    // The 20-unit cursor: in world space, that's 20 logical units. Build
    // the Transform as a rect in world.
    let cursor_rect_transform: Transform = (
        (
            context.cursor.position.x - cursor_size / 2.0,
            context.cursor.position.y - cursor_size / 2.0,
            cursor_size,
            cursor_size,
        ),
        state.viewport_context(),
    )
        .into();

    let cursor_rect: Rectangle<i32, Physical> = cursor_rect_transform.into();

    let cursor_element = SolidColorRenderElement::new(
        Id::new(),
        cursor_rect,
        CommitCounter::default(),
        [137.0 / 255.0, 250.0 / 255.0, 222.0 / 255.0, 0.5],
        Kind::Unspecified,
    );
    //
    // let state = &mut state.inner;
    //
    let mut elements: Vec<SolidColorRenderElement> = Vec::new();
    //
    // let physical_cursor_size = (20.0 * state.camera.transform.zoom()) as i32;
    //
    // let cursor_offset_x = context.cursor.position.x - state.camera.transform.position().x;
    // let cursor_offset_y = context.cursor.position.y - state.camera.transform.position().y;
    //
    // let cursor_scaled_x = cursor_offset_x * state.camera.transform.zoom();
    // let cursor_scaled_y = cursor_offset_y * state.camera.transform.zoom();
    //
    // let cursor_screen_x = cursor_scaled_x + (size.w as f64 / 2.0);
    // let cursor_screen_y = cursor_scaled_y + (size.h as f64 / 2.0);
    //
    // let cursor_rect = smithay::utils::Rectangle::from_loc_and_size(
    //     ((cursor_screen_x)  as i32, (cursor_screen_y) as i32),
    //     (physical_cursor_size, physical_cursor_size),
    // );
    //
    // // Create a SolidColorRenderElement (Red: [1.0, 0.0, 0.0, 1.0])
    //
    // let cursor_element = SolidColorRenderElement::new(
    //     Id::new(),                                          // Generate a fresh, unique ID
    //     cursor_rect,                                        // Your calculated geometry
    //     CommitCounter::default(),                           // Standard commit state
    //     [137.0 / 255.0, 250.0 / 255.0, 222.0 / 255.0, 0.5],
    //     Kind::Unspecified, // Standard rendering kind for custom elements
    // );

    // Add it to the very end of the elements list so it renders ON TOP of the windows
    elements.push(cursor_element);

    return elements;
}
