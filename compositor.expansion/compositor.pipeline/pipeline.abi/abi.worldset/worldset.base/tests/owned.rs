//! The ownership filter, which is the whole ordering invariant in one function.
//!
//! `renderer.core/submit.rs` suppresses exactly what `owned()` selects. If these two ever
//! disagree the symptom is not a crash — it is world content drawn twice, or
//! world content drawn on top of windows it belongs under, which is the bug the
//! mode exists to fix. Pinning the selection here is what keeps a future edit to
//! one side from silently drifting from the other.

use compositor_pipeline_abi_worldset_base::base::{Kind, Own, Source, WorldSet};

/// A band in draw order: window, panel, window. The middle entry is the case
/// that matters — a placeholder BETWEEN two windows, which is exactly what
/// `pipeline` mode cannot express.
fn band() -> WorldSet {
    WorldSet {
        rects: vec![[0.0; 4]; 3],
        srcs: vec![[0.0; 4]; 3],
        kinds: vec![Kind::Window, Kind::Panel, Kind::Window],
        alphas: vec![1.0; 3],
        flags: vec![0; 3],
        times: Vec::new(),
        sources: vec![Source::Unavailable, Source::Unavailable, Source::Unavailable],
    }
}

#[test]
fn engine_and_windows_modes_select_only_client_windows() {
    for own in [Own::None, Own::Windows] {
        assert_eq!(band().owned(own), vec![0, 2], "{own:?}");
    }
}

/// And they select them in ORDER, closing the gap the panel left — so a bundle
/// written against the window-only set sees the same indices it always saw.
#[test]
fn world_mode_selects_the_whole_band_in_draw_order() {
    assert_eq!(band().owned(Own::World), vec![0, 1, 2]);
}

/// A SHM surface has no fd, so it cannot cross to the worker's device, and one of
/// them must send the whole bundle back inline rather than bind a partial array.
///
/// Only the negative and empty cases are reachable from a unit test: the
/// "available" arms of `Source` wrap a real `Dmabuf` / imported allocation, which
/// needs a device. What the positive case rests on instead is that
/// `all_importable` is written as "every index `owned()` selects" — so the
/// per-mode scoping is covered by the `owned()` tests above rather than
/// re-asserted here.
#[test]
fn an_unimportable_entry_refuses_the_worker() {
    let s = band();
    assert!(!s.all_importable(Own::Windows));
    assert!(!s.all_importable(Own::World));
}

/// An empty set is vacuously importable — a desktop with nothing open is a valid
/// answer, not a reason to refuse the worker.
#[test]
fn an_empty_band_does_not_refuse_the_worker() {
    assert!(WorldSet::default().all_importable(Own::World));
}
