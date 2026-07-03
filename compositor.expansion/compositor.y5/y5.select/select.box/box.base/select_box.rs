use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{ImportAll, ImportMem, Texture};
use smithay::desktop::Window;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};
use compositor_y5_camera_transform_translate::transform::Transform;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::export::{ActiveOption, CanvasGrab, CanvasSelect};
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_y5_surface_interface_base::hit::{
    SurfaceHit, surfaces_inside_filtered, surfaces_overlap_filtered,
};
use compositor_y5_window_interface_draw::visible::DrawWindow;
use compositor_y5_window_interface_record::window::LoopWindow;

pub fn select_box<R>(
    state: &mut Loop,
    renderer: &mut R,
    size: Size<i32, Physical>,
    context: &compositor_y5_canvas_draw_context::context::Context,
) -> Vec<SolidColorRenderElement>
where
    R: smithay::backend::renderer::Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    // Render select box
    let active = {
        if let CanvasGrab::Active(ActiveOption::SelectBox {
            current_cursor,
            start_cursor,
            start_selection,
        }) = &state.inner.canvas_mut().Grab
        {
            Some((
                start_cursor.clone(),
                current_cursor.clone(),
                start_selection.clone(),
            ))
            // Translate to world
        } else {
            None
        }
    };

    if active.is_none() {
        return vec![];
    }

    let mut elements = Vec::new();
    let (start_cursor, current_cursor, start_selection) = active.unwrap();

    {
        let size = size.to_f64();
        let ctx = state.viewport_context();

        let start_xform: Transform = (start_cursor, ctx).into();
        let start_cursor: Point<f64, Physical> = start_xform.into();

        let current_xform: Transform = (current_cursor, ctx).into();
        let current_cursor: Point<f64, Physical> = current_xform.into();

        // let start_cursor = global_to_canvas(
        //     &state.inner.camera_mut().transform,
        //     Size::new(size.w, size.h),
        //     start_cursor.clone(),
        //     state.inner.space_state().default_scale()
        // );
        // let current_cursor = global_to_canvas(
        //     &state.inner.camera_mut().transform,
        //     Size::new(size.w, size.h),
        //     current_cursor.clone(),
        //     state.inner.space_state().default_scale()
        // );

        let fill_color = [137.0 / 255.0, 250.0 / 255.0, 222.0 / 255.0, 0.5]; // RGBA: Semi-transparent Blue
        let border_color = [0.0, 0.0, 0.8, 1.0]; // RGBA: Solid Darker Blue
        let border_thickness = 2;

        // It probably shouldn't scale the border with zoom? check now
        // let bw_screen = scale(&state.camera, border_thickness);
        let bw_screen = border_thickness;

        let x = f64::min(start_cursor.x, current_cursor.x);
        let y = f64::min(start_cursor.y, current_cursor.y);
        let width = (start_cursor.x - current_cursor.x).abs();
        let height = (start_cursor.y - current_cursor.y).abs();

        let box_geometry =
            Rectangle::from_loc_and_size((x as i32, y as i32), (width as i32, height as i32));

        // Clip the selection box to the viewport pane being drawn (no leak past it);
        // full-output render (no render target) → unchanged.
        let pane = state.inner.render_target.map(|rt| {
            Rectangle::<i32, Physical>::from_loc_and_size(
                ((rt.origin_logical.0 * ctx.scale).round() as i32, (rt.origin_logical.1 * ctx.scale).round() as i32),
                (rt.size_physical.0.round() as i32, rt.size_physical.1.round() as i32),
            )
        });
        let mut push_solid = |rect: Rectangle<i32, Physical>, color: [f32; 4]| {
            let rect = match pane {
                Some(p) => match rect.intersection(p) {
                    Some(clipped) => clipped,
                    None => return,
                },
                None => rect,
            };
            elements.push(SolidColorRenderElement::new(Id::new(), rect, CommitCounter::default(), color, Kind::Unspecified));
        };

        push_solid(box_geometry, fill_color);

        // 2. Draw the borders (Top, Bottom, Left, Right)
        let top_border =
            Rectangle::from_loc_and_size(box_geometry.loc, (box_geometry.size.w, border_thickness));
        let bottom_border = Rectangle::from_loc_and_size(
            box_geometry.loc + Point::new(0, box_geometry.size.h - border_thickness),
            (box_geometry.size.w, border_thickness),
        );
        let left_border =
            Rectangle::from_loc_and_size(box_geometry.loc, (border_thickness, box_geometry.size.h));
        let right_border = Rectangle::from_loc_and_size(
            box_geometry.loc + Point::new(box_geometry.size.w - border_thickness, 0),
            (border_thickness, box_geometry.size.h),
        );

        for border in [top_border, bottom_border, left_border, right_border] {
            push_solid(border, border_color);
        }
    }

    {
        // Interactively, find out which windows are intersecting within the selectbox using all_surface_under
        let x = f64::min(start_cursor.x, current_cursor.x);
        let y = f64::min(start_cursor.y, current_cursor.y);
        let width = (start_cursor.x - current_cursor.x).abs();
        let height = (start_cursor.y - current_cursor.y).abs();

        let box_geometry: Rectangle<i32, Logical> =
            Rectangle::from_loc_and_size((x as i32, y as i32), (width as i32, height as i32));

        enum Mode {
            Intersect,
            Contain,
        }

        let mode = Mode::Contain;
        // Convert box geom to world
        let SelectAll = match mode {
            Mode::Intersect => surfaces_overlap_filtered(
                state,
                Rectangle::new(
                    Point::new(box_geometry.loc.to_f64().x, box_geometry.loc.to_f64().y),
                    Size::new(box_geometry.size.to_f64().w, box_geometry.size.to_f64().h),
                ),
                &|hit| {
                    let Some(window) = hit.window() else {
                        return false;
                    };

                    window.visible(state)
                },
            ),
            Mode::Contain => surfaces_inside_filtered(
                state,
                Rectangle::new(
                    Point::new(box_geometry.loc.to_f64().x, box_geometry.loc.to_f64().y),
                    Size::new(box_geometry.size.to_f64().w, box_geometry.size.to_f64().h),
                ),
                &|hit| {
                    let Some(window) = hit.window() else {
                        return false;
                    };

                    window.visible(state)
                },
            ),
        };

        let mut selection: CanvasSelect = state.inner.select().clone();
        let mut selection_set = false;

        // Collect first: the loop body mutates the canvas selection slot, which
        // also lives in `inner.worlds` — so the space borrow must end here.
        let windows: Vec<smithay::desktop::Window> =
            state.inner.space_state().state.elements().cloned().collect();
        for window in &windows {
            let window_uuid = window.uuid();

            if window_uuid.is_none() {
                continue;
            }
            let window_uuid = window_uuid.unwrap();

            let surface_hit = SelectAll.iter().find_map(|hit| {
                // Find a matching hit
                if let SurfaceHit::Window {
                    window: hit_window, ..
                } = hit
                    && let Some(uuid) = hit_window.uuid()
                    && uuid == window_uuid
                {
                    return Some((hit_window, uuid));
                };

                None
            });

            let selected_initial = start_selection.contains(&window_uuid);

            let selected_current = selection.Selection.iter().any(|w| {
                if let Some(itr) = w.uuid()
                    && itr == window_uuid
                {
                    return true;
                };

                return false;
            });

            // Always increase selection, otherwise, consider selected initial to prevent endless toggles
            let selected_updated = surface_hit.is_some();

            enum Mode {
                Increase,
                Append,
            }

            let mode = Mode::Append;

            match mode {
                Mode::Increase => {
                    // Regular mode
                    if !selected_current && selected_updated {
                        selection = state.inner.select().append(window.clone());
                        selection_set = true;
                    }
                }
                Mode::Append => {
                    let selected_updated = !selected_initial && selected_updated
                        || (selected_initial && !selected_updated);
                    if selected_updated != selected_current {
                        selection = state.inner.select()
                            
                            .exact(window.clone(), selected_updated);
                        selection_set = true;
                    }
                }
            };
        }

        if selection_set {
            compositor_y5_select_interface_base::select(state, selection);
        }
    }

    return elements;
}
