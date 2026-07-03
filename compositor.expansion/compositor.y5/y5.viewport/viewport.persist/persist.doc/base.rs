//! Per-world persistence of the viewport tree (split/float layout + each pane's
//! camera), mirroring the placeholder document pattern. Rehydrated at world
//! build into the `VIEWPORTS` slot. Momentum/zone are transient (not persisted).
use compositor_support_system_persist_document_trait::base::Document;
use compositor_support_system_persist_document_trait::y5_document;
use compositor_y5_camera_state_base::state::Camera;
use compositor_y5_camera_transform_state::state::Transform;
use compositor_y5_viewport_state_base::state::{Axis, OutputViews, Slot, Viewport, Viewports, OUTPUT_VIEWS, OUTPUT_VIEWS_MUT};
use smithay::utils::{Point, Rectangle, Size};

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum AxisRec {
    Vertical,
    Horizontal,
}

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ViewportRec {
    Slots { axis: AxisRec, slots: Vec<SlotRec> },
    Floating { rect: (i32, i32, i32, i32), inner: Box<ViewportRec> },
}

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SlotRec {
    pub id: u64,
    pub pos: (f64, f64),
    pub zoom: f64,
    pub weight: f64,
    pub content: Option<Box<ViewportRec>>,
}

/// One world's whole viewport layout (one record per world).
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ViewportsRecord {
    pub root: ViewportRec,
    pub floating: Vec<ViewportRec>,
    pub active: u64,
    pub pointer: u64,
    pub next_id: u64,
}

fn vp_to(v: &Viewport) -> ViewportRec {
    match v {
        Viewport::Slots { axis, slots } => ViewportRec::Slots {
            axis: match axis {
                Axis::Vertical => AxisRec::Vertical,
                Axis::Horizontal => AxisRec::Horizontal,
            },
            slots: slots.iter().map(slot_to).collect(),
        },
        Viewport::Floating { rect, inner } => ViewportRec::Floating {
            rect: (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h),
            inner: Box::new(vp_to(inner)),
        },
    }
}

fn slot_to(s: &Slot) -> SlotRec {
    SlotRec {
        id: s.id,
        pos: (s.camera.transform.position.x, s.camera.transform.position.y),
        zoom: s.camera.transform.zoom,
        weight: s.weight,
        content: s.content.as_ref().map(|v| Box::new(vp_to(v))),
    }
}

fn vp_from(v: &ViewportRec) -> Viewport {
    match v {
        ViewportRec::Slots { axis, slots } => Viewport::Slots {
            axis: match axis {
                AxisRec::Vertical => Axis::Vertical,
                AxisRec::Horizontal => Axis::Horizontal,
            },
            slots: slots.iter().map(slot_from).collect(),
        },
        ViewportRec::Floating { rect, inner } => Viewport::Floating {
            rect: Rectangle::new(Point::from((rect.0, rect.1)), Size::from((rect.2, rect.3))),
            inner: Box::new(vp_from(inner)),
        },
    }
}

fn slot_from(s: &SlotRec) -> Slot {
    Slot {
        id: s.id,
        camera: Camera {
            transform: Transform { position: Point::from((s.pos.0, s.pos.1)), zoom: s.zoom },
            ..Default::default()
        },
        content: s.content.as_ref().map(|v| Box::new(vp_from(v))),
        weight: s.weight,
    }
}

fn record_of(vp: &Viewports) -> ViewportsRecord {
    ViewportsRecord {
        root: vp_to(&vp.root),
        floating: vp.floating.iter().map(vp_to).collect(),
        active: vp.active,
        pointer: vp.pointer,
        next_id: vp.next_id,
    }
}

fn apply_to(vp: &mut Viewports, rec: ViewportsRecord) {
    vp.root = vp_from(&rec.root);
    vp.floating = rec.floating.iter().map(vp_from).collect();
    vp.active = rec.active;
    vp.pointer = rec.pointer;
    vp.next_id = rec.next_id;
}

/// Row id ↔ output key. The bootstrap (sole/unknown-output) `""` key persists under
/// the legacy `"viewports"` row id, so a viewport layout saved before multi-monitor
/// still loads onto the single output. Each real monitor persists under its EDID key.
fn row_id(key: &str) -> String {
    if key.is_empty() { "viewports".to_string() } else { key.to_string() }
}
fn key_of(row_id: &str) -> String {
    if row_id == "viewports" { String::new() } else { row_id.to_string() }
}

pub struct ViewportsDoc;

impl Document for ViewportsDoc {
    type Slot = OutputViews;
    type Record = ViewportsRecord;
    const TABLE: &'static str = "world.viewport";
    const VERSION: u32 = 1;

    fn rows(s: &OutputViews) -> Vec<(String, Vec<(&'static str, String)>, ViewportsRecord)> {
        s.map.iter().map(|(key, vp)| (row_id(key), Vec::new(), record_of(vp))).collect()
    }

    fn apply(s: &mut OutputViews, id: &str, rec: ViewportsRecord) {
        apply_to(s.map.entry(key_of(id)).or_default(), rec);
    }
}

y5_document!(VIEWPORTS_DOC, ViewportsDoc, OUTPUT_VIEWS, OUTPUT_VIEWS_MUT);
