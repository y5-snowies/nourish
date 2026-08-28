use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::{ImportAll, ImportMem, Texture};
use smithay::utils::{Physical, Point, Rectangle, Size};
use compositor_orchestration_draw_scene_identity::identity::SolidBank;
use compositor_y5_canvas_draw_context::context::Context;
use compositor_orchestration_core_state_base::{Loop, Transform};
use compositor_orchestration_core_state_base::state::CoordinateTrait;

thread_local! {
    /// One canvas cursor exists, so it gets one identity that outlives the frame.
    /// A fresh `Id::new()` per frame reads to smithay's damage tracker as the old
    /// box vanishing and a new one appearing — and, on the DRM path, changes the
    /// element id occupying a hardware plane every frame. See `scene.identity`.
    static CURSOR: SolidBank = SolidBank::default();
}

/// The canvas cursor: a translucent box in the world band, drawn on the pane the
/// physical cursor is over (the caller gates that).
pub fn scene<R>(
    state: &mut Loop,
    _renderer: &mut R,
    _size: Size<i32, Physical>,
    context: &Context,
) -> Vec<SolidColorRenderElement>
where
    R: smithay::backend::renderer::Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    /// Edge length in world-logical units. A point extracted as a `Rectangle` is
    /// zero-sized, so the rect is built with a size rather than from a position.
    const CURSOR_SIZE: f64 = 20.0;

    // Where the box goes, in world units — and which of the two cursor positions
    // is the right source depends on whether the SHADER moves this box.
    //
    // `context.cursor.position` is the warp-CORRECTED world point: the content the
    // hand is over, named in un-displaced space. Drawn as-is it lands at that
    // content's pre-warp screen position, which is where the content is not drawn.
    // The hand's own position (`true_screen`) lands under the hand.
    //
    // An after-content pass resamples the whole band, this box included, so it
    // maps the box's un-displaced position back under the hand — exactly what the
    // corrected point already is. Correcting it here too would displace it twice.
    // Without such a pass nothing moves the box, so it needs the hand directly:
    // the output pass of a `windows`/`world` bundle samples its own targets, and
    // this is a `DrawOp::Solid` that is not among them.
    //
    // Not a guess about band placement — the box is the LAST element in the
    // content band, which smithay draws first (`render_elements.iter().rev()`), so
    // it precedes every window in the op list and `split_at` (the index after the
    // last world op) always leaves it inside `content`.
    let post_processed = {
        let target = state.inner.worlds.spawn_target();
        state
            .inner
            .worlds
            .get(target)
            .storage()
            .try_get(&compositor_background_two_storage_base::base::BG_TWO)
            .and_then(|t| t.bundle())
            .is_some_and(|cp| !cp.after.is_empty())
    };
    let viewport = state.viewport_context();
    let hand = (!post_processed)
        .then(compositor_orchestration_seat_pointer_publish::publish::true_screen)
        .flatten();
    let at = match hand {
        Some((x, y)) => {
            let t: Transform = (Point::<f64, Physical>::from((x, y)), viewport).into();
            t.into_storage_point_f64()
        }
        None => context.cursor.position,
    };
    let rect: Transform = (
        (at.x - CURSOR_SIZE / 2.0, at.y - CURSOR_SIZE / 2.0, CURSOR_SIZE, CURSOR_SIZE),
        viewport,
    )
        .into();

    vec![CURSOR.with(|bank| {
        bank.solid(
            0,
            Rectangle::<i32, Physical>::from(rect),
            [137.0 / 255.0, 250.0 / 255.0, 222.0 / 255.0, 0.5],
        )
    })]
}
