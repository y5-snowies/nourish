use smithay::backend::renderer::element::surface::render_elements_from_surface_tree;
use smithay::backend::renderer::element::{AsRenderElements, Kind};
use smithay::backend::renderer::{ImportAll, ImportMem, Texture};
use smithay::utils::{Physical, Point, Rectangle, Scale, Size};
use std::os::linux::raw::stat;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_core_state_base::{Loop, Transform};
use compositor_orchestration_seat_pointer_element::element::PointerRenderElement;

pub fn element<R>(
    state: &mut Loop,
    renderer: &mut R,
    size: Size<i32, Physical>,
) -> Vec<PointerRenderElement<R>>
where
    R: smithay::backend::renderer::Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    let mut elements: Vec<PointerRenderElement<R>> = Vec::new();

    let pointer = state.state.seat.seat.get_pointer().unwrap();
    let cursor_world = pointer.current_location();
    // let hotspot = state.inner.pointer_mut().element.get_current_hotspot();

    // Project the world pointer location through the pane the cursor is in (not
    // the full output), so the rendered cursor lands at the physical hardware
    // position even when a split pane's camera is panned/zoomed.
    let size_context = state.focus_pane_context();

    let hotspot_logical = state.inner.pointer_mut().element.get_current_hotspot();
    // hotspot is in surface logical buffer space; convert to physical:
    let hotspot_phys = Point::<i32, Physical>::from((
        (hotspot_logical.x as f64 * size_context.scale).round() as i32,
        (hotspot_logical.y as f64 * size_context.scale).round() as i32,
    ));

    // let render_at = cursor_phys - hotspot_phys;

    // Build Transform from world position. Hotspot is in *physical* pixels
    // per the smithay convention (hotspot is on the buffer, buffers are
    // physical-density).
    let cursor_rect_transform: Transform =
        ((cursor_world.x, cursor_world.y, 0.0, 0.0), size_context).into();

    let cursor_phys: Point<i32, Physical> = cursor_rect_transform.into();
    let render_at = cursor_phys - hotspot_phys; // subtract hotspot

    // A compositor-forced cursor (e.g. the canvas hand tool's grab icon) overrides
    // whatever the focused client set, so the affordance survives moving over windows.
    state.inner.pointer_mut().element.status = match state.state.seat.force_cursor {
        Some(icon) => smithay::input::pointer::CursorImageStatus::Named(icon),
        None => state.state.seat.pointer_status.clone(),
    };
    let pointer_elements: Vec<PointerRenderElement<_>> =
        state.inner.pointer_mut().element.render_elements(
            renderer,
            render_at,
            Scale::from(size_context.scale), // not .round() — exact fractional
            1.0,
        );

    if let Some(icon_surface) = &state.state.dnd.icon {
        let dnd_element = render_elements_from_surface_tree::<_, PointerRenderElement<R>>(
            renderer,
            icon_surface,
            render_at,
            Scale::from(size_context.scale),
            1.0,
            Kind::Unspecified,
        );
        
        elements.extend(dnd_element);
    }

    // let pointer_elements: Vec<PointerRenderElement<_>> = state.inner.pointer_mut().element.render_elements(
    //     renderer,
    //     cursor_rect.loc,
    //     Scale::from(size_ctx.scale.round()),
    //     1.0, // Alpha/Opacity
    // );

    // if pointer_elements.is_empty()
    //     println!("Cursor is empty");
    // }

    // POinter is in window space. SHould it be scaled? Yes. it is expected to be in render space which is different from world space. However it must remain unscaled because mouse is uniform across zoom.
    // also- pointer.current_location may already be in y5 world. in this case it should not be transformed at all.
    // It should be same as canvas ( which does transform )

    // Legacy which is transformed over panning and zooming.
    // let cursor_offset_x = cursor_logical.x - state.inner.camera_mut().transform.position().x;
    // let cursor_offset_y = cursor_logical.y - state.inner.camera_mut().transform.position().y;
    //
    // let cursor_scaled_x = cursor_offset_x * state.inner.camera_mut().transform.zoom();
    // let cursor_scaled_y = cursor_offset_y * state.inner.camera_mut().transform.zoom();
    //
    // let cursor_screen_x = cursor_scaled_x + (size.w as f64 / 2.0);
    // let cursor_screen_y = cursor_scaled_y + (size.h as f64 / 2.0);
    // let cursor_hotspot = state.inner.pointer_mut().element.get_current_hotspot();
    //
    // // 3. Subtract the hotspot (scaled by your camera zoom!) from the ACTUAL screen position
    // let scaled_hotspot_x = cursor_hotspot.x as f64;
    // let scaled_hotspot_y = cursor_hotspot.y as f64;
    // // let scaled_hotspot_x = cursor_hotspot.x as f64 * state.camera.zoom;
    // // let scaled_hotspot_y = cursor_hotspot.y as f64 * state.camera.zoom;
    //
    // let draw_x = cursor_screen_x - scaled_hotspot_x;
    // let draw_y = cursor_screen_y - scaled_hotspot_y;

    // let draw_location = cursor_logical;
    // let draw_location = smithay::utils::Point::<i32, smithay::utils::Physical>::from((
    //     draw_x.round() as i32,
    //     draw_y.round() as i32,
    // ));

    // 4. Generate the pointer elements
    // Note: We pass your `state.camera.zoom` as the scale factor so the cursor scales with the world
    // Legacy - Scaled version ( should also scale by output scale.
    // let pointer_elements: Vec<PointerRenderElement<_>> = state.inner.pointer_mut().element.render_elements(
    //     renderer,
    //     draw_location,
    //     smithay::utils::Scale::from(*state.inner.camera_mut().transform.zoom()),
    //     1.0, // Alpha/Opacity
    // );

    // CHECK:  use damage output in redraw and vblank
    // 5. Push the generated pointer elements into your master canvas list
    for el in pointer_elements {
        // Wrap it in the CanvasElement enum variant we just created
        elements.push(el);
    }

    return elements;
}
