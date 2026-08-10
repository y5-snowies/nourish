//! The engine facts a world states about itself, and the world-switch bug they
//! exist to make impossible.
//!
//! `requires` / `owns` / `whole_frame` tell the engine to do three things it
//! otherwise would not: collect the world set, leave the world band to the
//! shader, and redraw whole rather than by damage. They used to be published once
//! per SELECTION, from `load_multipass` — which runs only when a world builds its
//! `ParallaxBackground`. A world returning to the screen with an already-compiled
//! bundle therefore never re-published, and the engine went on following whichever
//! world had built last.
//!
//! On a desktop that is: open the picker (it builds its own parallax and states
//! ITS answers), come back, and the session world is drawn while the engine still
//! believes the picker. Stale `owns` leaves nobody drawing the window band; stale
//! `requires` at zero means the world set is never collected, so a bundle that
//! composites windows itself is handed an empty one. Windows do not come back.
//!
//! These tests pin the replacement: the facts live in the WORLD's own storage, are
//! a pure function of the bundle handed in, and `None` is neutral — so a per-frame
//! publish from the drawn world cannot leave another world's answers behind, and
//! two worlds cannot see each other's at all.

use compositor_pipeline_abi_worldset_base::base::Own;
use compositor_pipeline_world_system_base::base::{PipelineState, PIPELINE, facts, publish_facts};
use compositor_support_system_storage_slot_base::base::Storage;

const WORKER: compositor_pipeline_build_place_base::place::Env =
    compositor_pipeline_build_place_base::place::Env { worker: true };

fn world() -> Storage {
    let mut s = Storage::default();
    s.insert(&PIPELINE, PipelineState::default());
    s
}

#[test]
fn a_world_states_its_own_facts_and_takes_them_back() {
    let mut w = world();
    // ---- A world with no bundle states the neutral answers. -----------------
    // Dirty them first, as this world's previous bundle would have.
    publish_facts(&mut w, None);
    assert_eq!(facts(&w).requires, 0, "a world with no bundle must ask the engine for nothing");
    assert!(!facts(&w).whole_frame, "no bundle cannot mean whole-frame redraw");
    assert_eq!(
        facts(&w).owns_band(), Own::None,
        "STALE OWNERSHIP IS THE BUG: left owned, the engine leaves the band to a shader that \
         is not running and no windows are drawn at all",
    );

    // ---- A world WITH a bundle states that bundle's answers. -----------------
    let Some(root) = examples() else { return };
    // `tb-window-chroma` owns the band and requires the world set — the shape that
    // breaks visibly when it is inherited by a world that is not running it.
    let dir = root.join("tb-window-chroma");
    if !dir.join("pipeline.json").is_file() {
        return;
    }
    let cp = compositor_pipeline_build_pipeline_base::pipeline::load_pipeline(&dir, WORKER)
        .expect("shipped bundle loads");

    publish_facts(&mut w, Some(&cp));
    assert_eq!(facts(&w).requires, cp.requires.bits(), "this world's requirements, not the last one's");
    assert_eq!(facts(&w).owns_band(), cp.owns.into(), "this world's ownership");
    assert_eq!(facts(&w).whole_frame, cp.composes_whole_frame());

    // A SECOND world is untouched by the first — the property the global could not
    // have. This is the world-switch bug, expressed as a test rather than a hope.
    let other = world();
    assert_eq!(facts(&other).requires, 0, "one world's bundle must not reach another's slot");
    assert_eq!(facts(&other).owns_band(), Own::None);

    // …and this world dropping its bundle takes it all down again, with no hook
    // and nothing to invalidate.
    publish_facts(&mut w, None);
    assert_eq!(facts(&w).requires, 0);
    assert_eq!(facts(&w).owns_band(), Own::None);
}

fn examples() -> Option<std::path::PathBuf> {
    let d = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../document/shader-examples");
    d.is_dir().then_some(d)
}
