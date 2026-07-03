//! The cursor-teleport layout canvas: a custom pan/zoom iced widget that draws each
//! placed monitor as a rectangle (with an inner box showing its true aspect ratio) and
//! lets you drag rectangles to move them and drag the bottom-right corner to resize
//! width and height independently (free aspect). The map is infinite (pan/positions
//! are unbounded in every direction).
//! Scroll-wheel zooms (around the cursor); dragging empty canvas pans. Content is
//! clipped to the canvas bounds so a placement dragged out of view never overflows
//! the panel. Moves/resizes emit `LayoutMove`/`LayoutResize` in LAYOUT coords
//! (applied UI-locally with snapping + no-overlap in `surface.view`); clicking a
//! square emits `LayoutSelect`. The layout is abstract/teleport-only — it never
//! affects a monitor's scale or resolution. The APPLY LAYOUT button (in `display.rs`)
//! persists the arrangement.
use compositor_configurator_settings_surface_message::message::SettingsMessage;
use compositor_developer_environment_preference_base::base::LayoutPlacement;
use compositor_orchestration_driver_output_base::base::DisplayInfo;
use compositor_support_iced_core_engine_base::Renderer;
use iced_core::renderer::Renderer as _;
use iced_core::widget::{tree, Tree, Widget};
use iced_core::{
    layout, mouse, renderer, Background, Border, Color, Element, Event, Layout, Length, Point,
    Rectangle, Shell, Size, Theme,
};

const CANVAS_H: f32 = 240.0;
const MARGIN: f32 = 10.0;
const HANDLE: f32 = 12.0;
const MIN_SIZE: f32 = 60.0;
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 6.0;

const BG: Color = Color::from_rgb(0.08, 0.08, 0.10);
const LINE: Color = Color::from_rgb(0.24, 0.24, 0.28);
const SQUARE: Color = Color::from_rgb(0.15, 0.16, 0.20);
const INNER: Color = Color::from_rgb(0.32, 0.34, 0.40);
const MUTED: Color = Color::from_rgb(0.40, 0.40, 0.46);
const ACCENT: Color = Color::from_rgb(0.24, 0.60, 1.0);

/// Transient view + drag state (per-widget, in the iced tree). `pan`/`zoom` map
/// LAYOUT coordinates to on-screen pixels; both persist across frames.
struct State {
    drag: Option<Drag>,
    /// Layout-space point shown at the canvas's top-left inset corner.
    pan: (f32, f32),
    /// Pixels-per-layout-unit (>0).
    zoom: f32,
}
impl Default for State {
    fn default() -> Self {
        Self { drag: None, pan: (0.0, 0.0), zoom: 1.0 }
    }
}

/// The mode of an in-progress drag.
#[derive(Clone, Copy)]
enum Drag {
    /// Moving a placement; `grab_*` is the cursor offset inside it (layout coords).
    Move { id: u64, grab_dx: f32, grab_dy: f32 },
    /// Resizing a placement from its bottom-right handle.
    Resize { id: u64 },
    /// Panning the view; `last_*` is the previous cursor position (canvas pixels).
    Pan { last_x: f32, last_y: f32 },
}

struct LayoutCanvas<'a> {
    placements: &'a [LayoutPlacement],
    displays: &'a [DisplayInfo],
    selected: Option<u64>,
}

impl<'a> LayoutCanvas<'a> {
    /// Whether a placement's monitor is shown on the map: it must be CONNECTED (in
    /// `displays`, which lists only connected monitors) AND ACTIVE (`enabled`). A
    /// placement for a disconnected or deactivated monitor stays in the stored layout
    /// but is not drawn or interactive here.
    fn shown(&self, identity: &str) -> bool {
        self.displays.iter().any(|d| d.edid_key == identity && d.enabled)
    }

    /// The (w, h) of the monitor's current/preferred/first mode, for the aspect box.
    fn aspect(&self, identity: &str) -> Option<(f32, f32)> {
        let d = self.displays.iter().find(|d| d.edid_key == identity)?;
        let m = d.current.or(d.preferred).or_else(|| d.available.first().copied())?;
        (m.width > 0 && m.height > 0).then(|| (m.width as f32, m.height as f32))
    }
}

/// Canvas-pixel offset (relative to the widget bounds' top-left) → layout coords.
fn to_layout(px: f32, py: f32, st: &State) -> (f32, f32) {
    ((px - MARGIN) / st.zoom + st.pan.0, (py - MARGIN) / st.zoom + st.pan.1)
}

/// The bottom-right resize-handle region of a placement, in layout coords (the
/// handle is a fixed pixel size on screen, so it scales as `HANDLE / zoom` here).
fn in_handle(lx: f32, ly: f32, p: &LayoutPlacement, zoom: f32) -> bool {
    let m = HANDLE / zoom;
    lx >= p.x + p.w - m && lx <= p.x + p.w && ly >= p.y + p.h - m && ly <= p.y + p.h
}
fn point_in(lx: f32, ly: f32, p: &LayoutPlacement) -> bool {
    lx >= p.x && lx <= p.x + p.w && ly >= p.y && ly <= p.y + p.h
}

/// Fit a box of aspect `aw:ah` centered inside `r` (minus `pad` on each side).
fn fit_aspect(r: Rectangle, aw: f32, ah: f32, pad: f32) -> Rectangle {
    let (fw, fh) = (r.width - 2.0 * pad, r.height - 2.0 * pad);
    let scale = (fw / aw).min(fh / ah);
    let (w, h) = (aw * scale, ah * scale);
    Rectangle { x: r.x + (r.width - w) / 2.0, y: r.y + (r.height - h) / 2.0, width: w, height: h }
}

impl<'a> Widget<SettingsMessage, Theme, Renderer> for LayoutCanvas<'a> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(CANVAS_H))
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(limits.resolve(Length::Fill, Length::Fixed(CANVAS_H), Size::ZERO))
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        // Canvas backdrop.
        renderer.fill_quad(
            renderer::Quad {
                bounds,
                border: Border { color: LINE, width: 1.0, radius: 6.0.into() },
                ..Default::default()
            },
            Background::Color(BG),
        );
        let z = st.zoom;
        // Layout → absolute screen pixel.
        let sx = |lx: f32| bounds.x + MARGIN + (lx - st.pan.0) * z;
        let sy = |ly: f32| bounds.y + MARGIN + (ly - st.pan.1) * z;
        // Clip everything to the canvas so a panned/zoomed square never overflows.
        renderer.with_layer(bounds, |renderer| {
            // Draw non-selected first, then the selected one on top.
            let order = self
                .placements
                .iter()
                .filter(|p| self.shown(&p.identity) && self.selected != Some(p.id))
                .chain(self.placements.iter().filter(|p| self.shown(&p.identity) && self.selected == Some(p.id)));
            for p in order {
                let selected = self.selected == Some(p.id);
                let r = Rectangle { x: sx(p.x), y: sy(p.y), width: p.w * z, height: p.h * z };
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: r,
                        border: Border {
                            color: if selected { ACCENT } else { MUTED },
                            width: if selected { 2.0 } else { 1.0 },
                            radius: 4.0.into(),
                        },
                        ..Default::default()
                    },
                    Background::Color(SQUARE),
                );
                if let Some((aw, ah)) = self.aspect(&p.identity) {
                    let inner = fit_aspect(r, aw, ah, 10.0);
                    renderer.fill_quad(
                        renderer::Quad { bounds: inner, ..Default::default() },
                        Background::Color(INNER),
                    );
                }
                if selected {
                    let h = Rectangle {
                        x: r.x + r.width - HANDLE,
                        y: r.y + r.height - HANDLE,
                        width: HANDLE,
                        height: HANDLE,
                    };
                    renderer.fill_quad(renderer::Quad { bounds: h, ..Default::default() }, Background::Color(ACCENT));
                }
            }
        });
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        shell: &mut Shell<'_, SettingsMessage>,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let state = tree.state.downcast_mut::<State>();
        match event {
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let Some(pos) = cursor.position_in(bounds) else { return };
                let dy = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 40.0,
                };
                let old = state.zoom;
                let nz = (old * (1.0 + dy * 0.15)).clamp(MIN_ZOOM, MAX_ZOOM);
                if (nz - old).abs() > f32::EPSILON {
                    // Keep the layout point under the cursor fixed across the zoom.
                    let (lx, ly) = to_layout(pos.x, pos.y, state);
                    state.zoom = nz;
                    state.pan.0 = lx - (pos.x - MARGIN) / nz;
                    state.pan.1 = ly - (pos.y - MARGIN) / nz;
                    shell.request_redraw();
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(pos) = cursor.position_in(bounds) else { return };
                let (lx, ly) = to_layout(pos.x, pos.y, state);
                // A resize handle on the selected square takes priority.
                if let Some(id) = self.selected {
                    if let Some(p) = self.placements.iter().find(|p| p.id == id) {
                        if in_handle(lx, ly, p, state.zoom) {
                            state.drag = Some(Drag::Resize { id });
                            shell.capture_event();
                            return;
                        }
                    }
                }
                // Otherwise select + begin moving the topmost SHOWN square under the cursor.
                if let Some(p) = self.placements.iter().rev().find(|p| self.shown(&p.identity) && point_in(lx, ly, p)) {
                    shell.publish(SettingsMessage::LayoutSelect(p.id));
                    state.drag = Some(Drag::Move { id: p.id, grab_dx: lx - p.x, grab_dy: ly - p.y });
                    shell.capture_event();
                    return;
                }
                // Empty canvas → pan the view.
                state.drag = Some(Drag::Pan { last_x: pos.x, last_y: pos.y });
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let Some(drag) = state.drag else { return };
                let Some(pos) = cursor.position_in(bounds) else { return };
                let (lx, ly) = to_layout(pos.x, pos.y, state);
                match drag {
                    Drag::Move { id, grab_dx, grab_dy } => {
                        // No lower bound — the map is infinite, placements may go negative.
                        shell.publish(SettingsMessage::LayoutMove(id, lx - grab_dx, ly - grab_dy));
                    }
                    Drag::Resize { id } => {
                        if let Some(p) = self.placements.iter().find(|p| p.id == id) {
                            // Width and height move INDEPENDENTLY (free aspect ratio).
                            let w = (lx - p.x).max(MIN_SIZE);
                            let h = (ly - p.y).max(MIN_SIZE);
                            shell.publish(SettingsMessage::LayoutResize(id, w, h));
                        }
                    }
                    Drag::Pan { last_x, last_y } => {
                        state.pan.0 -= (pos.x - last_x) / state.zoom;
                        state.pan.1 -= (pos.y - last_y) / state.zoom;
                        state.drag = Some(Drag::Pan { last_x: pos.x, last_y: pos.y });
                    }
                }
                shell.request_redraw();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.drag.take().is_some() {
                    shell.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let st = tree.state.downcast_ref::<State>();
        if let Some(pos) = cursor.position_in(layout.bounds()) {
            let (lx, ly) = to_layout(pos.x, pos.y, st);
            if self.placements.iter().any(|p| self.shown(&p.identity) && point_in(lx, ly, p)) {
                return mouse::Interaction::Pointer;
            }
        }
        mouse::Interaction::None
    }
}

/// Build the layout-canvas element for the Display tab.
pub fn layout_canvas<'a>(
    placements: &'a [LayoutPlacement],
    displays: &'a [DisplayInfo],
    selected: Option<u64>,
) -> Element<'a, SettingsMessage, Theme, Renderer> {
    Element::new(LayoutCanvas { placements, displays, selected })
}
