//! Where the pointer is, what it is holding, and when it last changed.
//!
//! This lives in the SEAT because that is what it is: one pointer, per seat,
//! written by the paths that already compute it. It used to sit in
//! `kernel.graphic/graphic.bridge/bridge.window` — not because the kernel owned
//! it, but because the kernel is the one layer the renderer, the worker and the
//! seat could all reach without declaring a dependency on the pipeline. Choosing
//! a crate's home by who needs to reach it is the same mistake as choosing a
//! global for the same reason.
//!
//! Nothing in the pipeline writes here. `pipeline.world::PipelineSystem` READS
//! this once per world per frame and captures it into that world's own state, so
//! what reaches a shader is a value some world observed on a frame it was drawn,
//! not a slot the renderer fetched behind the caller's back.
//!
//! # Why this is a slot and not a lane
//!
//! Everything in `window.descriptor` is per-entry: it rides `ElementMeta` from the
//! scene, through the draw plan, into an array indexed by drawable. The pointer is
//! ONE value for the whole frame, and threading a frame-wide constant through a
//! per-element channel would mean carrying 32 identical bytes on every element and
//! reconciling them at the far end. So it goes the way the other frame-wide facts
//! already go — `set::band_suppressed`, `warp::true_screen` — as a published slot
//! the renderer reads once.
//!
//! # Where the values come from
//!
//! The position is published by the pointer's own motion path, which already
//! computes the physical point and the screen size for the warp, so it is stored
//! as screen UV and needs no extent downstream. The buttons are published by the
//! button handler.
//!
//! Neither needs edge DETECTION, unlike the per-window moments: a button event IS
//! the transition, so the moment is stamped where it happens and there is no
//! previous state to compare against. That is why there are seven derived window
//! moments and only two here.
//!
//! # Gating
//!
//! Not gated on the `pointer_state` requirement. Both writers are event paths —
//! a pointer motion, a click — not per-frame work, and the whole cost is two
//! atomic stores on input that already does far more. What the requirement gates
//! is the upload, in `renderer.graph`.

use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

/// Buttons, as a bit set in the same idiom as `window.descriptor`'s flags: an
/// integer carried in a float lane, tested with `(u32(lane) & BIT) != 0u`.
///
/// Named buttons rather than a single "down" boolean because the lane costs the
/// same either way, and a bundle that wants to distinguish a drag from a
/// right-click otherwise has no way to.
pub const LEFT: u32 = 1 << 0;
pub const RIGHT: u32 = 1 << 1;
pub const MIDDLE: u32 = 1 << 2;

/// Every button this version reports. `buttons != 0` is "something is held",
/// which is the common question.
pub const ANY: u32 = LEFT | RIGHT | MIDDLE;

/// Linux input codes, which is what the backend hands us.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

/// The bit a Linux button code maps to, or `None` for one we do not report —
/// side/extra buttons, tablet barrel buttons. Unreported buttons must not fall
/// through to `LEFT`: a bundle keying an effect on "held" would fire on a
/// thumb button.
pub const fn bit_of(code: u32) -> Option<u32> {
    match code {
        BTN_LEFT => Some(LEFT),
        BTN_RIGHT => Some(RIGHT),
        BTN_MIDDLE => Some(MIDDLE),
        _ => None,
    }
}

// f32 bits, so the whole slot is lock-free and a reader never blocks the input
// thread. Relaxed throughout: the four values are independent, and a frame that
// catches a position from just before a click is indistinguishable from one
// dispatched a microsecond earlier.
static X: AtomicU32 = AtomicU32::new(0);
static Y: AtomicU32 = AtomicU32::new(0);
static BUTTONS: AtomicU32 = AtomicU32::new(0);
static DOWN_AT: AtomicU32 = AtomicU32::new(0);
static UP_AT: AtomicU32 = AtomicU32::new(0);

/// Publish the pointer position in SCREEN UV.
///
/// UV rather than physical because the motion path has the screen size in hand
/// and the consumers do not all have the same extent — the worker renders per
/// pane. Normalising once at the source is also what the warp chain does, for the
/// same reason: it survives a resolution change unscaled.
pub fn set_position(u: f64, v: f64) {
    X.store((u as f32).to_bits(), Relaxed);
    Y.store((v as f32).to_bits(), Relaxed);
}

/// Record a button transition. `now` is on the shared clock (`window.clock`).
///
/// A code this build does not report is ignored entirely — it does not clear the
/// other buttons and does not stamp a moment.
pub fn note_button(code: u32, pressed: bool, now: f32) {
    let Some(bit) = bit_of(code) else { return };
    let held = BUTTONS.load(Relaxed);
    if pressed {
        BUTTONS.store(held | bit, Relaxed);
        DOWN_AT.store(now.to_bits(), Relaxed);
    } else {
        BUTTONS.store(held & !bit, Relaxed);
        UP_AT.store(now.to_bits(), Relaxed);
    }
}

/// The `Pointer` UBO's two vec4s, in shader order.
///
/// ```text
/// at.x     = screen U            at.y = screen V
/// at.z     = held buttons        at.w = reserved
/// moment.x = last pressed        moment.y = last released
/// moment.z = reserved            moment.w = reserved
/// ```
///
/// The moments are absolute, like the window ones, so a pass reads
/// `t - pointer.moment.x` for "how long since the press".
pub fn packed(never: f32) -> [[f32; 4]; 2] {
    let f = |a: &AtomicU32| f32::from_bits(a.load(Relaxed));
    let stamp = |a: &AtomicU32| match a.load(Relaxed) {
        // Zero is the untouched state, not a moment at time zero. A pass would
        // otherwise read "pressed at start-up" as a very old event, which is
        // right by luck for a decay and wrong for anything that asks whether it
        // happened at all.
        0 => never,
        bits => f32::from_bits(bits),
    };
    [
        [f(&X), f(&Y), BUTTONS.load(Relaxed) as f32, 0.0],
        [stamp(&DOWN_AT), stamp(&UP_AT), 0.0, 0.0],
    ]
}

/// The last TRUE physical cursor position, before correction — and `None`
/// whenever no correction is in effect.
///
/// Per-SEAT, not per-world: it describes where the hand is, and that is the same
/// fact whichever world is focused. It sat in `kernel.graphic/bridge.window/warp`
/// for reachability, which is the same misplacement the pointer above had; it is
/// produced by `pointer.input` and consumed by `pointer.draw` and the canvas
/// cursor, all seat and cursor code, so nothing outside this layer needs it.
///
/// Written on EVERY motion event — `Some` warped, `None` plain — so one read
/// answers both "is a warp in effect" and "where is the hand". Writing it only on
/// the warped path left it `Some` forever after any warp had run, so a bundle
/// switch or the picker left every reader correcting for nothing.
///
/// Consumers must not re-derive the gate: two cursors that disagree about whether
/// a warp applies is a cursor that renders away from where it clicks.
static TRUE_SCREEN: std::sync::RwLock<Option<(f64, f64)>> = std::sync::RwLock::new(None);

pub fn set_true_screen(at: Option<(f64, f64)>) {
    if let Ok(mut s) = TRUE_SCREEN.write() {
        *s = at;
    }
}

/// Where the hand is, while a warp is in effect — for the consumers the shader
/// never displaced: the cursor sprite (drawn in the screen band) and the canvas
/// solid box (drawn after the pipeline's output pass). `None` = no warp, use the
/// ordinary path.
pub fn true_screen() -> Option<(f64, f64)> {
    *TRUE_SCREEN.read().ok()?
}
