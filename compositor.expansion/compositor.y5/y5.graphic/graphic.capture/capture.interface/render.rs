//! Per-element (offscreen) capture: render the captured windows DIRECTLY into
//! the capture dmabuf with the composing renderer, positioned capture-local —
//! so windows that are panned off-screen are still captured, and the capture
//! never includes the compositor's own chrome.
//!
//! This is the alternative to blitting the on-screen framebuffer region. It is
//! used for `Windows` / `WorldRegion` targets (which can be off-screen);
//! `ScreenRegion` / `FullScreen` keep the screen blit (they are screen-space by
//! definition).
//!
//! Backends call [`window_render_job`] (which reads the capture session) and,
//! for the matching registry entry, [`draw_windows_into`] with their composing
//! renderer (GLES or Vulkan — both `Bind<Dmabuf>`), instead of the blit.

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::{AsRenderElements, Element, RenderElement};
use smithay::backend::renderer::{Bind, Color32F, Frame, ImportAll, ImportMem, Renderer, Texture};
use smithay::desktop::Window;
use smithay::utils::{Physical, Point, Rectangle, Scale, Size, Transform};

use compositor_background_two_draw_element::element::ParallaxBackground;
use compositor_orchestration_draw_dispatch_frame::frame::SceneDispatch;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_y5_graphic_capture_registry::EntryId;
use compositor_y5_graphic_capture_session::session::{CapturePhase, CaptureTarget};
use compositor_y5_window_interface_record::window::LoopWindow;

/// The per-frame "render these windows into this capture entry" job.
pub struct WindowRenderJob {
    /// Registry entry to render into.
    pub entry_id: EntryId,
    /// Capture buffer size (physical pixels).
    pub size: Size<i32, Physical>,
    /// Render scale (output scale; camera zoom is intentionally NOT applied).
    pub scale: f64,
    /// Each captured window + its top-left position in capture-local physical
    /// pixels.
    pub windows: Vec<(Window, Point<i32, Physical>)>,
    /// When true, emit a transparent backdrop (windows only); when false, the
    /// parallax + iced backdrop is drawn behind the windows.
    pub no_background: bool,
    /// y5-world top-left of the capture region (for projecting the backdrop into
    /// capture-local space).
    pub origin: Point<i32, smithay::utils::Logical>,
    /// Parallax `zoom` for the backdrop — `render scale / output scale` (1.0 for
    /// world-region native capture; the fit ratio for window capture).
    pub backdrop_zoom: f32,
}

/// Clone the live parallax instance and re-aim it at the capture region (so the
/// backdrop shows the same slice of the infinite background, at native scale).
/// `None` when no-background is set or there is no background instance.
pub fn capture_backdrop(state: &Loop, job: &WindowRenderJob) -> Option<ParallaxBackground> {
    if job.no_background {
        return None;
    }
    let target = state.inner.worlds.spawn_target();
    let mut bg = state.inner.worlds.get(target).storage().get(&compositor_background_two_storage_base::base::BG_TWO).instance.clone()?;
    bg.pan = (job.origin.x as f32, job.origin.y as f32);
    bg.zoom = job.backdrop_zoom;
    bg.output_size = (job.size.w as f32, job.size.h as f32);
    Some(bg)
}

type LogicalRect = Rectangle<i32, smithay::utils::Logical>;

fn rects_overlap(a: LogicalRect, b: LogicalRect) -> bool {
    a.loc.x < b.loc.x + b.size.w
        && b.loc.x < a.loc.x + a.size.w
        && a.loc.y < b.loc.y + b.size.h
        && b.loc.y < a.loc.y + a.size.h
}

fn union_logical(a: LogicalRect, b: LogicalRect) -> LogicalRect {
    let x0 = a.loc.x.min(b.loc.x);
    let y0 = a.loc.y.min(b.loc.y);
    let x1 = (a.loc.x + a.size.w).max(b.loc.x + b.size.w);
    let y1 = (a.loc.y + a.size.h).max(b.loc.y + b.size.h);
    Rectangle::new(Point::from((x0, y0)), Size::from((x1 - x0, y1 - y0)))
}

/// The live union bbox (y5-world) of the captured (`force_set`) windows.
fn live_windows_bbox(state: &Loop) -> Option<LogicalRect> {
    let mut acc: Option<LogicalRect> = None;
    for w in state.inner.space_state().state.elements() {
        let in_set = w
            .uuid()
            .map(|id| state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE).force_set.contains(&id))
            .unwrap_or(false);
        if !in_set {
            continue;
        }
        if let Some(g) = state.inner.space_state().state.element_bbox(w) {
            acc = Some(match acc {
                None => g,
                Some(a) => union_logical(a, g),
            });
        }
    }
    acc
}

/// Build the render job for the active capture, or `None` if the current
/// target is screen-space (uses the blit path) or there's nothing to render.
pub fn window_render_job(state: &Loop) -> Option<WindowRenderJob> {
    let CapturePhase::Active(a) = &state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE).phase else {
        return None;
    };
    let entry_id = a.capture.entry_id();
    let scale = state.size_ctx_all().scale;

    let size = a.capture.size()?;
    if size.w < 1 || size.h < 1 {
        return None;
    }

    match &a.target {
        // WorldRegion: a FIXED y5-world rectangle captured at NATIVE scale
        // (camera-zoom-independent). The entry is sized `world × scale`, so
        // `region_world` is the true world rect; windows overlapping it are
        // drawn at their world offset and off-region parts clip.
        CaptureTarget::WorldRegion(r) => {
            let rect = *r;
            if rect.size.w < 1 || rect.size.h < 1 {
                return None;
            }
            // Fit the world rect into the (possibly capped) entry, letterbox-
            // centered. For an un-capped entry `fit == output scale`, so this is
            // the native, zoom-independent capture; a capped entry scales down.
            let fit = (size.w as f64 / rect.size.w as f64).min(size.h as f64 / rect.size.h as f64);
            let off_x = ((size.w as f64 - rect.size.w as f64 * fit) / 2.0).round() as i32;
            let off_y = ((size.h as f64 - rect.size.h as f64 * fit) / 2.0).round() as i32;
            let mut windows = Vec::new();
            for w in state.inner.space_state().state.elements() {
                let Some(bbox) = state.inner.space_state().state.element_bbox(w) else {
                    continue;
                };
                if !rects_overlap(bbox, rect) {
                    continue;
                }
                let loc = Point::from((
                    off_x + ((bbox.loc.x - rect.loc.x) as f64 * fit).round() as i32,
                    off_y + ((bbox.loc.y - rect.loc.y) as f64 * fit).round() as i32,
                ));
                windows.push((w.clone(), loc));
            }
            Some(WindowRenderJob {
                entry_id,
                size,
                scale: fit,
                windows,
                no_background: a.no_background,
                origin: rect.loc,
                backdrop_zoom: (fit / scale) as f32,
            })
        }
        // Windows: track the LIVE union bbox of the captured windows and FIT it
        // into the fixed entry (letterbox-centered). The encoder resolution
        // stays constant; the content scales as windows move/resize — so the
        // bbox "grows" without resizing the capture buffer.
        CaptureTarget::Windows(_) => {
            let bbox = live_windows_bbox(state)?;
            if bbox.size.w < 1 || bbox.size.h < 1 {
                return None;
            }
            // Fit world bbox → physical entry, preserving aspect.
            let fit = (size.w as f64 / bbox.size.w as f64).min(size.h as f64 / bbox.size.h as f64);
            let off_x = ((size.w as f64 - bbox.size.w as f64 * fit) / 2.0).round() as i32;
            let off_y = ((size.h as f64 - bbox.size.h as f64 * fit) / 2.0).round() as i32;
            let mut windows = Vec::new();
            for w in state.inner.space_state().state.elements() {
                let in_set = w
                    .uuid()
                    .map(|id| state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE).force_set.contains(&id))
                    .unwrap_or(false);
                if !in_set {
                    continue;
                }
                let Some(wb) = state.inner.space_state().state.element_bbox(w) else {
                    continue;
                };
                let loc = Point::from((
                    off_x + ((wb.loc.x - bbox.loc.x) as f64 * fit).round() as i32,
                    off_y + ((wb.loc.y - bbox.loc.y) as f64 * fit).round() as i32,
                ));
                windows.push((w.clone(), loc));
            }
            Some(WindowRenderJob {
                entry_id,
                size,
                scale: fit,
                windows,
                no_background: a.no_background,
                origin: bbox.loc,
                backdrop_zoom: (fit / scale) as f32,
            })
        }
        // Screen/FullScreen use the blit path.
        _ => None,
    }
}

/// Render the job's windows into `dmabuf` with `renderer` (the renderer holding
/// the windows' imported buffers). Clears to transparent first; NO backdrop.
/// Used by the native multi-GPU GLES tap (`MultiRenderer`, which isn't
/// `SceneDispatch`). Best-effort — errors are logged, not propagated.
pub fn draw_windows_into<R>(
    renderer: &mut R,
    dmabuf: &mut Dmabuf,
    size: Size<i32, Physical>,
    windows: &[(Window, Point<i32, Physical>)],
    scale: f64,
) where
    R: Renderer + ImportAll + ImportMem + Bind<Dmabuf>,
    R::TextureId: Texture + Clone + 'static,
{
    let scale_s = Scale::from(scale);
    let built = build_window_elements(renderer, windows, scale_s);
    let full = Rectangle::from_loc_and_size((0, 0), size);
    let mut fb = match renderer.bind(dmabuf) {
        Ok(fb) => fb,
        Err(e) => {
            warn!("capture render: bind entry dmabuf failed: {e:?}");
            return;
        }
    };
    let mut frame = match renderer.render(&mut fb, size, Transform::Normal) {
        Ok(f) => f,
        Err(e) => {
            warn!("capture render: begin frame failed: {e:?}");
            return;
        }
    };
    let _ = frame.clear(Color32F::new(0.0, 0.0, 0.0, 0.0), &[full]);
    draw_window_layer::<R>(&mut frame, &built, scale_s, full);
    finish_capture::<R>(frame);
}

/// Like [`draw_windows_into`] but draws the parallax backdrop behind the windows
/// (no-background OFF). Requires `R: SceneDispatch` (Vulkan + plain GLES).
pub fn draw_windows_into_bg<R>(
    renderer: &mut R,
    dmabuf: &mut Dmabuf,
    size: Size<i32, Physical>,
    windows: &[(Window, Point<i32, Physical>)],
    scale: f64,
    backdrop: Option<ParallaxBackground>,
) where
    R: Renderer + ImportAll + ImportMem + Bind<Dmabuf> + SceneDispatch,
    R::TextureId: Texture + Clone + 'static,
{
    let scale_s = Scale::from(scale);
    let built = build_window_elements(renderer, windows, scale_s);
    let full = Rectangle::from_loc_and_size((0, 0), size);
    let mut fb = match renderer.bind(dmabuf) {
        Ok(fb) => fb,
        Err(e) => {
            warn!("capture render: bind entry dmabuf failed: {e:?}");
            return;
        }
    };
    let mut frame = match renderer.render(&mut fb, size, Transform::Normal) {
        Ok(f) => f,
        Err(e) => {
            warn!("capture render: begin frame failed: {e:?}");
            return;
        }
    };
    let _ = frame.clear(Color32F::new(0.0, 0.0, 0.0, 0.0), &[full]);
    // Backdrop: the parallax slice for this region, behind the windows
    // (re-aimed at the capture region by `capture_backdrop`).
    if let Some(bg) = &backdrop {
        let _ = RenderElement::<R>::draw(bg, &mut frame, Element::src(bg), full, &[full], &[], None);
    }
    draw_window_layer::<R>(&mut frame, &built, scale_s, full);
    finish_capture::<R>(frame);
}

/// Build the per-window render elements (each call needs `&mut renderer`; the
/// elements hold buffer handles, not a renderer borrow, so they outlive this).
fn build_window_elements<R>(
    renderer: &mut R,
    windows: &[(Window, Point<i32, Physical>)],
    scale_s: Scale<f64>,
) -> Vec<Vec<WaylandSurfaceRenderElement<R>>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + 'static,
{
    let mut built = Vec::with_capacity(windows.len());
    for (w, loc) in windows {
        built.push(w.render_elements(renderer, *loc, scale_s, 1.0));
    }
    built
}

/// Draw the windows back-to-front. `built` is in `Space::elements()` order
/// (bottom-to-top), so the topmost window composites last; within each window,
/// `render_elements` is front-to-back, so it is reversed.
fn draw_window_layer<R>(
    frame: &mut <R as smithay::backend::renderer::RendererSuper>::Frame<'_, '_>,
    built: &[Vec<WaylandSurfaceRenderElement<R>>],
    scale_s: Scale<f64>,
    full: Rectangle<i32, Physical>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + 'static,
{
    for els in built.iter() {
        for el in els.iter().rev() {
            let _ = el.draw(
                frame,
                el.src(),
                el.geometry(scale_s),
                &[full],
                el.opaque_regions(scale_s).iter().as_slice(),
                None,
            );
        }
    }
}

/// Block until the GPU render lands before returning. The capture dmabuf is read
/// by a DIFFERENT API (wgpu/gles) on a different device; without this wait the
/// readback races and sees an uninitialized (white) buffer. Mirrors the winit
/// Vulkan present (`sp.wait()`) and the native capture blit (`device_wait_idle`).
fn finish_capture<R>(frame: <R as smithay::backend::renderer::RendererSuper>::Frame<'_, '_>)
where
    R: Renderer,
{
    match frame.finish() {
        Ok(sync) => {
            let _ = sync.wait();
        }
        Err(e) => warn!("capture render: finish failed: {e:?}"),
    }
}
