//! The pane key's whole job is to keep buffer sets apart. Each test below is a
//! bug that was actually shipped, or one the current structure exists to make
//! unrepresentable.

use compositor_background_two_worker_key::key::{PaneKey, Region};
use uuid::Uuid;

fn out_a() -> std::sync::Arc<str> {
    std::sync::Arc::from("Dell U2720Q ABC123")
}
fn out_b() -> std::sync::Arc<str> {
    std::sync::Arc::from("LG 27GP950 XYZ789")
}

fn world_a() -> Uuid {
    Uuid::from_u128(0x59350000_0000_4000_8000_000000000001)
}
fn world_b() -> Uuid {
    Uuid::from_u128(0x59350000_0000_4000_8000_0000000000aa)
}

/// THE bug this key change exists for: two worlds on the same output+region
/// shared one `Pane`, so a `persist` bundle in one sampled the other's
/// accumulated pixels.
#[test]
fn worlds_do_not_share_a_pane() {
    let a = PaneKey::viewport(&out_a(), world_a(), 0);
    let b = PaneKey::viewport(&out_a(), world_b(), 0);
    assert_ne!(a, b);
}

/// The bug the output field was added for: regions restart at 0 on every
/// monitor, so root panes collided at one size and one camera.
#[test]
fn outputs_do_not_share_a_pane() {
    let a = PaneKey::viewport(&out_a(), world_a(), 0);
    let b = PaneKey::viewport(&out_b(), world_a(), 0);
    assert_ne!(a, b);
}

#[test]
fn regions_do_not_share_a_pane() {
    assert_ne!(
        PaneKey::viewport(&out_a(), world_a(), 0),
        PaneKey::viewport(&out_a(), world_a(), 1)
    );
}

/// The picker, the lock screen and the overview each draw a full-output
/// backdrop, and all three are region 0 of the same world on the same output.
#[test]
fn overlays_are_distinct_from_each_other_and_from_region_zero() {
    let w = world_a();
    let keys = [
        PaneKey::overlay(&out_a(), w, "picker"),
        PaneKey::overlay(&out_a(), w, "lock"),
        PaneKey::overlay(&out_a(), w, "overview"),
        PaneKey::viewport(&out_a(), w, 0),
    ];
    for (i, a) in keys.iter().enumerate() {
        for b in &keys[i + 1..] {
            assert_ne!(a, b, "{a} collided with {b}");
        }
    }
}

/// The regression guard for the leak this replaced. The namespace used to be
/// baked into the output string (`"{output}::lock"`), so the unplug sweep —
/// which compares against the PLAIN output key the kernel publishes — never
/// matched an overlay pane, and every one of them leaked a fullscreen dmabuf
/// per slot until the 30-second backstop.
#[test]
fn an_overlay_still_retires_with_its_output() {
    let overlay = PaneKey::overlay(&out_a(), world_a(), "lock");
    assert_eq!(&*overlay.output, &*out_a());
}

/// An out-of-range region index must not alias region 0 — that would silently
/// re-share the pane this key exists to separate.
#[test]
fn an_absurd_region_index_saturates_rather_than_wrapping() {
    let huge = PaneKey::viewport(&out_a(), world_a(), usize::MAX);
    assert_eq!(huge.region, Region::Viewport(u16::MAX));
    assert_ne!(huge, PaneKey::viewport(&out_a(), world_a(), 0));
}

/// A world switch must be able to name exactly its own panes, across every
/// output and both region kinds.
#[test]
fn a_world_owns_its_panes_across_outputs_and_kinds() {
    let (w, other) = (world_a(), world_b());
    let mine = [
        PaneKey::viewport(&out_a(), w, 0),
        PaneKey::viewport(&out_b(), w, 3),
        PaneKey::overlay(&out_a(), w, "overview"),
    ];
    assert!(mine.iter().all(|k| k.world == w));
    assert!(!mine.iter().any(|k| k.world == other));
}
