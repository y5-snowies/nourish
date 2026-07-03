//! Edge-crossing lookup for the cursor-teleport layout (orthographic projection).
use compositor_orchestration_driver_output_base::base::{Edge, Placement, TeleportLayout};

fn sq(id: u64, key: &str, x: f32, y: f32, size: f32) -> Placement {
    Placement { id, key: key.into(), x, y, w: size, h: size }
}

// Two equal squares side by side: A(id1) at x=0, B(id2) at x=100, both 100×100.
fn side_by_side() -> TeleportLayout {
    TeleportLayout::new(vec![sq(1, "A", 0.0, 0.0, 100.0), sq(2, "B", 100.0, 0.0, 100.0)], false)
}

#[test]
fn cross_right_into_left_neighbor() {
    let l = side_by_side();
    let n = l.neighbor(1, Edge::Right, 0.25).expect("B to the right of A");
    assert_eq!(n.id, 2);
    assert_eq!(n.key, "B");
    assert_eq!(n.entry_edge, Edge::Left);
    // Same vertical fraction preserved (equal heights).
    assert!((n.entry_frac - 0.25).abs() < 1e-4, "entry_frac={}", n.entry_frac);
}

#[test]
fn cross_left_back_into_right_neighbor() {
    let l = side_by_side();
    let n = l.neighbor(2, Edge::Left, 0.75).expect("A to the left of B");
    assert_eq!(n.id, 1);
    assert_eq!(n.entry_edge, Edge::Right);
    assert!((n.entry_frac - 0.75).abs() < 1e-4);
}

#[test]
fn no_neighbor_on_free_edge() {
    let l = side_by_side();
    assert!(l.neighbor(1, Edge::Left, 0.5).is_none(), "A's left edge is free");
    assert!(l.neighbor(1, Edge::Top, 0.5).is_none());
    assert!(l.neighbor(2, Edge::Right, 0.5).is_none());
}

#[test]
fn differing_sizes_preserve_proportion() {
    // Big A (0..200) beside small B (200..300, vertically offset 40..140).
    let l = TeleportLayout::new(vec![sq(1, "A", 0.0, 0.0, 200.0), sq(2, "B", 200.0, 40.0, 100.0)], false);
    // Exit A's right at frac 0.5 → abstract y = 100, within B's [40,140].
    let n = l.neighbor(1, Edge::Right, 0.5).expect("B covers y=100");
    assert_eq!(n.id, 2);
    // Entry frac into B = (100 - 40) / 100 = 0.6.
    assert!((n.entry_frac - 0.6).abs() < 1e-4, "entry_frac={}", n.entry_frac);
}

#[test]
fn crossing_outside_neighbor_span_is_none() {
    // B only covers y in [40,140]; exiting A's right at the very top (y≈0) misses.
    let l = TeleportLayout::new(vec![sq(1, "A", 0.0, 0.0, 200.0), sq(2, "B", 200.0, 40.0, 100.0)], false);
    assert!(l.neighbor(1, Edge::Right, 0.0).is_none());
}

#[test]
fn duplicate_key_is_a_distinct_zone() {
    // Same monitor "A" placed twice; B in the middle. Crossing B's right lands on
    // the second A placement (id 3), not the first.
    let l = TeleportLayout::new(
        vec![sq(1, "A", 0.0, 0.0, 100.0), sq(2, "B", 100.0, 0.0, 100.0), sq(3, "A", 200.0, 0.0, 100.0)],
        false,
    );
    let n = l.neighbor(2, Edge::Right, 0.5).expect("second A to the right of B");
    assert_eq!(n.id, 3);
    assert_eq!(n.key, "A");
}

#[test]
fn vertical_stack_crosses_bottom_to_top() {
    let l = TeleportLayout::new(vec![sq(1, "A", 0.0, 0.0, 100.0), sq(2, "B", 0.0, 100.0, 100.0)], false);
    let n = l.neighbor(1, Edge::Bottom, 0.3).expect("B below A");
    assert_eq!(n.id, 2);
    assert_eq!(n.entry_edge, Edge::Top);
    assert!((n.entry_frac - 0.3).abs() < 1e-4);
}

// ── Orthographic projection: gapped (non-touching) monitors still cross ──

#[test]
fn gapped_monitors_cross_by_projection() {
    // A(0..100) and B(300..400) with a big GAP between them; not snapped/touching.
    let l = TeleportLayout::new(vec![sq(1, "A", 0.0, 0.0, 100.0), sq(2, "B", 300.0, 0.0, 100.0)], false);
    let n = l.neighbor(1, Edge::Right, 0.5).expect("orthographic ray reaches B across the gap");
    assert_eq!(n.id, 2);
    assert_eq!(n.entry_edge, Edge::Left);
}

#[test]
fn nearest_in_direction_wins() {
    // Two monitors to the right; the NEARER one (B at 150) is chosen over C (at 400).
    let l = TeleportLayout::new(
        vec![sq(1, "A", 0.0, 0.0, 100.0), sq(2, "B", 150.0, 0.0, 100.0), sq(3, "C", 400.0, 0.0, 100.0)],
        false,
    );
    let n = l.neighbor(1, Edge::Right, 0.5).expect("nearest right neighbour");
    assert_eq!(n.id, 2, "should pick the nearer monitor");
}

// ── Cyclic wrap-around ──

#[test]
fn cyclic_wraps_off_the_right_edge() {
    // Row A|B; exiting B's RIGHT (free edge) wraps to the LEFTMOST monitor A.
    let mut l = side_by_side();
    l.cyclic = true;
    let n = l.neighbor(2, Edge::Right, 0.5).expect("wrap to leftmost");
    assert_eq!(n.id, 1);
    assert_eq!(n.key, "A");
    assert_eq!(n.entry_edge, Edge::Left);
}

#[test]
fn cyclic_off_does_not_wrap() {
    let l = side_by_side(); // cyclic = false
    assert!(l.neighbor(2, Edge::Right, 0.5).is_none(), "no wrap when cyclic is off");
}

#[test]
fn cyclic_wraps_off_the_left_edge_to_rightmost() {
    let mut l = side_by_side();
    l.cyclic = true;
    // Exiting A's LEFT (free edge) wraps to the RIGHTMOST monitor B.
    let n = l.neighbor(1, Edge::Left, 0.5).expect("wrap to rightmost");
    assert_eq!(n.id, 2);
    assert_eq!(n.entry_edge, Edge::Right);
}
