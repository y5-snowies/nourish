//! The pointer slot's behaviour, which is mostly about what it does NOT do.
//!
//! ONE test function: the slot is a set of process globals shared by every test
//! thread in this binary, so two tests writing concurrently would read each
//! other's pointer — the same reason `window.set`'s tests are single-function.

use compositor_orchestration_seat_pointer_publish::publish as p;

const NEVER: f32 = -1.0;

#[test]
fn the_slot_tracks_buttons_and_stamps_only_the_edge_it_saw() {
    // Untouched, both moments read as NEVER rather than as time zero. A pass
    // asking "has there ever been a click" must be able to get "no" — zero would
    // answer "yes, at start-up", which is right by luck for a decay and wrong for
    // anything that branches on it.
    let fresh = p::packed(NEVER);
    assert_eq!(fresh[1][0], NEVER, "a press that never happened is not time zero");
    assert_eq!(fresh[1][1], NEVER);

    p::set_position(0.25, 0.75);
    let at = p::packed(NEVER)[0];
    assert_eq!((at[0], at[1]), (0.25, 0.75), "position is screen UV, unscaled");

    // Buttons accumulate and clear INDEPENDENTLY: holding left and clicking right
    // must not release left, which is what a plain boolean would have done.
    p::note_button(0x110, true, 10.0); // BTN_LEFT
    p::note_button(0x111, true, 11.0); // BTN_RIGHT
    assert_eq!(p::packed(NEVER)[0][2] as u32, p::LEFT | p::RIGHT);
    p::note_button(0x111, false, 12.0);
    assert_eq!(p::packed(NEVER)[0][2] as u32, p::LEFT, "releasing one must not release the other");

    // …and the two moments are the two edges, not one "last event".
    let m = p::packed(NEVER)[1];
    assert_eq!(m[0], 11.0, "last PRESS, not last event");
    assert_eq!(m[1], 12.0, "last RELEASE");

    // A button this build does not report is ignored ENTIRELY — it must not fall
    // through to LEFT (a bundle keying on "held" would fire on a thumb button)
    // and must not stamp a moment.
    p::note_button(0x116, true, 99.0); // BTN_SIDE, unreported
    assert_eq!(p::packed(NEVER)[0][2] as u32, p::LEFT, "an unreported button changed the set");
    assert_eq!(p::packed(NEVER)[1][0], 11.0, "an unreported button stamped a moment");
    assert!(p::bit_of(0x116).is_none());

    p::note_button(0x110, false, 13.0);
    assert_eq!(p::packed(NEVER)[0][2] as u32, 0);

    // Every reported bit is distinct, and `ANY` is their union — the cheapest way
    // for a new button to be wrong is to repeat a shift.
    let each = [p::LEFT, p::RIGHT, p::MIDDLE];
    assert_eq!(each.iter().fold(0, |a, b| a | b), p::ANY);
    assert_eq!(each.iter().map(|b| b.count_ones()).sum::<u32>(), p::ANY.count_ones());
}
