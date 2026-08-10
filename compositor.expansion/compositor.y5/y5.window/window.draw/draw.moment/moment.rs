//! When each thing last happened to a window, derived in ONE place from what the
//! frame already knows.
//!
//! # Why derived rather than hooked
//!
//! Every one of these moments has a natural hook somewhere — the map path, the
//! focus path, the two ends of the resize grab, the cull. Seven hooks in six files
//! is seven chances for a path to be added later that forgets one, and the symptom
//! would be a timestamp that is stale rather than absent: a shader animating
//! against a moment that silently stopped updating.
//!
//! So this observes the values the draw already computes and records the EDGES.
//! One function, called once per window per frame, and a new moment is a field
//! here rather than a hook somewhere else.
//!
//! # On-screen is inferred from being drawn, not from the cull
//!
//! `entered`/`left` could be read off the frustum cull, but the canvas scene runs
//! once per PANE: a window on one pane and off another would leave and enter every
//! frame. Instead every observation stamps `last_seen`, and a gap in it is what
//! counts as having left — so as long as any pane draws the window, it is present.
//!
//! The consequence is that this is only ever called for windows that DRAW, which
//! is also why it costs nothing for a culled one.

use compositor_pipeline_abi_clock_base::base as clock;
use compositor_pipeline_abi_descriptor_base::base as d;
use smithay::desktop::Window;
use std::sync::Mutex;

/// How long a window must go undrawn before it counts as having left, rather than
/// as having been skipped for a frame.
///
/// Long enough to cover a pane that redraws out of step or a frame the compositor
/// simply did not composite; short enough that a window returning from a genuine
/// absence is not mistaken for one that never went. It is a debounce on a
/// heuristic, not a deadline anything waits on.
const ABSENT: f32 = 0.15;

/// Record that `window` has just been MAPPED.
///
/// The one hook, and the reason the rest of this file can be derived. `observe`
/// is skipped entirely while no bundle is loaded, so without a birth stamp a
/// bundle selected later would find no history and have to invent one — and the
/// only thing it could invent is "everything opened now".
pub fn born(window: &Window) {
    let now = clock::now();
    window.user_data().insert_if_missing_threadsafe(|| Slot::new(Moments::born(now)));
}

/// One window's history. Lives in the window's own user data, so it is created
/// with the window and dropped with it — nothing to evict, and no map keyed by an
/// id that can outlive its window.
#[derive(Debug)]
struct Moments {
    opened: f32,
    entered: f32,
    left: f32,
    focused: f32,
    topmost: f32,
    selected: f32,
    deselected: f32,
    resize_start: f32,
    resize_end: f32,
    move_start: f32,
    move_end: f32,
    last_seen: f32,
    /// The frame whose edges have already been folded in. See `observe`.
    last_frame: u64,
    /// Whether the previous frame was observed at all. False while no bundle is
    /// loaded — see `observe` — and the flag exists so RESUMING is not mistaken
    /// for the window re-entering: `last_seen` is arbitrarily old by then, and
    /// the absence test below would otherwise stamp `left`/`entered` on every
    /// window the instant a bundle is selected.
    observing: bool,
    was_focused: bool,
    was_topmost: bool,
    was_selected: bool,
    was_resizing: bool,
    was_moving: bool,
}

impl Moments {
    /// A window that appeared while nobody was watching.
    ///
    /// `opened`/`entered` are NEVER, not `now`: a shader reads `t - opened` as an
    /// age, and `NEVER` reads as "long ago", which is the truth. Stamping `now`
    /// would tell every window already on screen that it had just opened, and the
    /// whole desktop would play its open animation the instant a bundle loaded.
    fn unseen() -> Self {
        let mut m = Moments::born(clock::NEVER);
        m.last_seen = clock::NEVER;
        m
    }

    fn born(now: f32) -> Self {
        Moments {
            // A window is "opened" the first time it is drawn, which is the moment
            // it becomes something a shader could animate — earlier than that it
            // has no pixels and no entry in the array to carry the stamp.
            opened: now,
            entered: now,
            left: clock::NEVER,
            focused: clock::NEVER,
            topmost: clock::NEVER,
            selected: clock::NEVER,
            deselected: clock::NEVER,
            resize_start: clock::NEVER,
            resize_end: clock::NEVER,
            move_start: clock::NEVER,
            move_end: clock::NEVER,
            last_seen: now,
            last_frame: u64::MAX,
            observing: false,
            was_focused: false,
            was_topmost: false,
            was_selected: false,
            was_resizing: false,
            was_moving: false,
        }
    }

    fn row(&self) -> d::Times {
        [
            self.opened,
            self.entered,
            self.left,
            clock::NEVER,
            self.focused,
            self.topmost,
            self.selected,
            self.deselected,
            self.resize_start,
            self.resize_end,
            self.move_start,
            self.move_end,
        ]
    }
}

type Slot = Mutex<Moments>;

/// Record this frame's edges for `window` and return its timestamp row.
///
/// The booleans are the ones the draw already resolved for the descriptor flags,
/// passed in rather than re-derived: two answers to "is this focused" is how a
/// flag and a timestamp come to disagree about the same instant.
pub fn observe(
    window: &Window,
    focused: bool,
    topmost: bool,
    resizing: bool,
    moving: bool,
    selected: bool,
    active: bool,
    frame: u64,
) -> d::Times {
    // NOTHING for a desktop with no bundle, which is the overwhelmingly common
    // one. This ran per window PER PANE per frame — a clock read, a user-data
    // lookup, a mutex and a twelve-float row — for tables only a bundle reads.
    //
    // Gating it is only safe because `born` stamps the slot where the window is
    // MAPPED, so a bundle selected mid-session finds real `opened` times already
    // recorded rather than deciding every window opened just now.
    if !active {
        return d::no_times(clock::NEVER);
    }
    let now = clock::now();
    let data = window.user_data();
    // Only reached for a window that was mapped before this hook existed, or one
    // whose map event was missed: it appeared while nobody was watching.
    data.insert_if_missing_threadsafe(|| Slot::new(Moments::unseen()));
    let Some(slot) = data.get::<Slot>() else { return d::no_times(clock::NEVER) };
    let Ok(mut m) = slot.lock() else { return d::no_times(clock::NEVER) };

    // EDGES ARE FOLDED IN ONCE PER FRAME, not once per call.
    //
    // The canvas scene runs once per PANE, and the `topmost` it passes is
    // pane-local — it is latched by a `top_taken` flag that the scene resets on
    // every call. So a window that is front-most on one monitor and not on
    // another used to arrive here as `true` then `false` within a single frame,
    // and the rising-edge test below re-stamped `topmost` every frame forever.
    // That is precisely the "every age zero, every animation frozen at its first
    // frame" failure the rising-edge rule exists to prevent, and it was live on
    // any multi-output desktop.
    //
    // Recording only on the first call of a frame makes the stamp independent of
    // how many panes draw the window, and of the order they draw it in. Later
    // panes still get the row — they need it to fill their own uniforms — they
    // just do not get to restate history.
    if m.last_frame == frame {
        return m.row();
    }
    m.last_frame = frame;

    // Presence first: a window that has been away is re-entering, and the moment
    // it left is the last frame that saw it, not the frame that noticed.
    //
    // RESUMING is not re-entering. With no bundle loaded nothing observed, so
    // `last_seen` is arbitrarily old; treating that as an absence would stamp
    // `left` and `entered` on every window the frame a bundle is selected.
    if !m.observing {
        m.observing = true;
        m.last_seen = now;
    } else if now - m.last_seen > ABSENT {
        m.left = m.last_seen;
        m.entered = now;
    }
    m.last_seen = now;

    // Rising edges only. A moment is when something BECAME true; restamping it
    // every frame it stays true would make every age zero, and every animation
    // driven by one would sit frozen at its first frame.
    if focused && !m.was_focused {
        m.focused = now;
    }
    if topmost && !m.was_topmost {
        m.topmost = now;
    }
    if selected && !m.was_selected {
        m.selected = now;
    }
    if resizing && !m.was_resizing {
        m.resize_start = now;
    }
    if moving && !m.was_moving {
        m.move_start = now;
    }
    // …and the falling edges that are themselves events. A gesture has two ends
    // worth animating: the grab, and the settle after it.
    if !resizing && m.was_resizing {
        m.resize_end = now;
    }
    if !moving && m.was_moving {
        m.move_end = now;
    }
    // Leaving the selection is as animatable as joining it — a group highlight
    // has to fade out, and it cannot if only the rising edge is recorded.
    if !selected && m.was_selected {
        m.deselected = now;
    }
    m.was_focused = focused;
    m.was_topmost = topmost;
    m.was_selected = selected;
    m.was_resizing = resizing;
    m.was_moving = moving;
    m.row()
}
