//! Viewport tree mutations driven by shortcuts: split the active slot, detach it
//! to a floating pane. All operate on the focused world's `Viewports`.
use compositor_y5_camera_state_base::state::Camera;
use compositor_y5_viewport_state_base::state::{Axis, Slot, SlotId, Viewport, Viewports};
use smithay::utils::{Physical, Rectangle};

/// Active slot, across the root tree and every floating pane.
fn active_slot_mut(viewports: &mut Viewports) -> Option<&mut Slot> {
    let id = viewports.active;
    if viewports.root.find(id).is_some() {
        return viewports.root.find_mut(id);
    }
    viewports.floating.iter_mut().find_map(|v| v.find_mut(id))
}

/// Split the active leaf slot into two panes along `axis`; the new pane becomes
/// active (both start from the split slot's camera). Works in root or floating.
pub fn split(viewports: &mut Viewports, axis: Axis) {
    let (a, b) = (viewports.next_id, viewports.next_id + 1);
    let Some(slot) = active_slot_mut(viewports) else { return };
    if slot.content.is_some() {
        return;
    }
    let transform = slot.camera.transform.clone();
    let mk = |id| Slot {
        id,
        camera: Camera { transform: transform.clone(), ..Default::default() },
        content: None,
        weight: 1.0,
    };
    slot.content = Some(Box::new(Viewport::Slots { axis, slots: vec![mk(a), mk(b)] }));
    viewports.next_id += 2;
    viewports.active = b;
}

/// Detach the active tiled pane into a new floating pane at `rect`. No-op if the
/// active slot is the root's only leaf, or isn't a tiled pane.
pub fn detach(viewports: &mut Viewports, rect: Rectangle<i32, Physical>) {
    let id = viewports.active;
    if let Viewport::Slots { slots, .. } = &viewports.root {
        if slots.len() == 1 && slots[0].id == id && slots[0].content.is_none() {
            return;
        }
    }
    let Some(slot) = take_slot(&mut viewports.root, id) else { return };
    let inner = Viewport::Slots { axis: Axis::Vertical, slots: vec![slot] };
    viewports.floating.push(Viewport::Floating { rect, inner: Box::new(inner) });
    // `active` stays the detached slot id (now living in the floating pane).
}

/// Remove the active pane (a split leaf, or a floating pane when its last leaf
/// goes). No-op if the active slot is the root's only leaf.
pub fn remove_active(viewports: &mut Viewports) {
    let id = viewports.active;
    if let Viewport::Slots { slots, .. } = &viewports.root {
        if slots.len() == 1 && slots[0].id == id && slots[0].content.is_none() {
            return;
        }
    }
    if take_slot(&mut viewports.root, id).is_none() {
        // Not in the tiled root → a floating pane. Remove the leaf; drop the
        // floating pane if it became empty.
        for i in 0..viewports.floating.len() {
            if take_slot(&mut viewports.floating[i], id).is_some() {
                if is_empty(&viewports.floating[i]) {
                    viewports.floating.remove(i);
                }
                break;
            }
        }
    }
    let fallback = viewports.root.first_leaf().id;
    viewports.active = fallback;
    viewports.pointer = fallback;
}
fn is_empty(vp: &Viewport) -> bool {
    match vp {
        Viewport::Slots { slots, .. } => slots.is_empty(),
        Viewport::Floating { inner, .. } => is_empty(inner),
    }
}

/// Remove the slot with `id` from a `Slots` array within `vp`, returning it.
fn take_slot(vp: &mut Viewport, id: SlotId) -> Option<Slot> {
    match vp {
        Viewport::Slots { slots, .. } => {
            if let Some(pos) = slots.iter().position(|s| s.id == id) {
                return Some(slots.remove(pos));
            }
            for s in slots.iter_mut() {
                if let Some(taken) = s.content.as_mut().and_then(|inner| take_slot(inner, id)) {
                    return Some(taken);
                }
            }
            None
        }
        Viewport::Floating { inner, .. } => take_slot(inner, id),
    }
}
