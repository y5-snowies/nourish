//! Geometry for the viewport tree: turn a `Viewport` tree + an output rect into
//! the physical rect of every leaf slot (one render region per leaf) and the
//! separator bars between adjacent split slots.
use compositor_y5_viewport_state_base::state::{Axis, Slot, SlotId, Viewport, Viewports};
use smithay::utils::{Physical, Point, Rectangle, Size};

/// Width of the bar drawn between adjacent split slots (physical px).
pub const SEPARATOR: i32 = 8;

pub struct Region {
    pub slot: SlotId,
    pub rect: Rectangle<i32, Physical>,
}

/// A bar between two adjacent slots in a `Slots` array. `a`/`b` are those slots'
/// ids (each carries a `weight`); dragging the bar shifts weight between them.
/// `a_len`/`b_len` are their physical lengths along `axis` (for the drag math).
pub struct Separator {
    pub rect: Rectangle<i32, Physical>,
    pub axis: Axis,
    pub a: SlotId,
    pub b: SlotId,
    pub a_len: i32,
    pub b_len: i32,
}

#[derive(Default)]
pub struct Computed {
    /// Leaf regions, root-first then floating (floating drawn on top). Reverse-
    /// iterate for topmost-first hit testing.
    pub regions: Vec<Region>,
    pub separators: Vec<Separator>,
}

/// Regions + separators for `viewports` filling `bounds` (the whole output).
pub fn compute(viewports: &Viewports, bounds: Rectangle<i32, Physical>) -> Computed {
    let mut out = Computed::default();
    walk(&viewports.root, bounds, &mut out);
    // Floating panes overlay the root (appended last → on top; `slot_at` reverse-
    // iterates). Each `Floating` carries its own rect, so `bounds` is ignored.
    for floating in &viewports.floating {
        walk(floating, bounds, &mut out);
    }
    out
}

fn walk(vp: &Viewport, bounds: Rectangle<i32, Physical>, out: &mut Computed) {
    match vp {
        Viewport::Slots { axis, slots } => divide(*axis, slots, bounds, out),
        Viewport::Floating { rect, inner } => walk(inner, *rect, out),
    }
}

fn divide(axis: Axis, slots: &[Slot], bounds: Rectangle<i32, Physical>, out: &mut Computed) {
    let total = slots.iter().map(|s| s.weight.max(0.0)).sum::<f64>().max(f64::MIN_POSITIVE);
    // A *vertical* split places panes side by side (a vertical divider) → divide x.
    let horizontal = matches!(axis, Axis::Vertical);
    let span = if horizontal { bounds.size.w } else { bounds.size.h };
    let gaps = SEPARATOR * (slots.len() as i32 - 1).max(0);
    let usable = (span - gaps).max(0) as f64;

    let lens: Vec<i32> = slots.iter().map(|s| ((s.weight.max(0.0) / total) * usable).round() as i32).collect();
    let mut cursor = if horizontal { bounds.loc.x } else { bounds.loc.y };
    for (i, slot) in slots.iter().enumerate() {
        let len = lens[i];
        let rect = if horizontal {
            Rectangle::new(Point::from((cursor, bounds.loc.y)), Size::from((len, bounds.size.h)))
        } else {
            Rectangle::new(Point::from((bounds.loc.x, cursor)), Size::from((bounds.size.w, len)))
        };
        match &slot.content {
            Some(inner) => walk(inner, rect, out),
            None => out.regions.push(Region { slot: slot.id, rect }),
        }
        cursor += len;
        if i + 1 < slots.len() {
            let rect = if horizontal {
                Rectangle::new(Point::from((cursor, bounds.loc.y)), Size::from((SEPARATOR, bounds.size.h)))
            } else {
                Rectangle::new(Point::from((bounds.loc.x, cursor)), Size::from((bounds.size.w, SEPARATOR)))
            };
            out.separators.push(Separator { rect, axis, a: slot.id, b: slots[i + 1].id, a_len: lens[i], b_len: lens[i + 1] });
            cursor += SEPARATOR;
        }
    }
}

/// The leaf slot whose region contains `point` (topmost-first: floating, then
/// root). `None` on a separator / outside every leaf.
pub fn slot_at(computed: &Computed, point: Point<i32, Physical>) -> Option<(SlotId, Rectangle<i32, Physical>)> {
    computed.regions.iter().rev().find(|r| r.rect.contains(point)).map(|r| (r.slot, r.rect))
}

/// The separator bar under `point`, if any (for separator-drag hit testing).
pub fn separator_at(computed: &Computed, point: Point<i32, Physical>) -> Option<&Separator> {
    computed.separators.iter().find(|s| s.rect.contains(point))
}
