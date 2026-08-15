//! Screen-extent policy: what a push past the output's physical bounds does.
//!
//! The pointer paths ask here what reaching an extent MEANS. Two modes, selected by
//! `preference.input_edge_pan` (Settings → Input → Mouse & Touchpad):
//!
//! - OFF (historical): the extent belongs to the monitor next door — cross when
//!   the teleport layout places one across that edge, else pin the cursor.
//! - ON: the extent belongs to the CAMERA — the push pans the canvas, and the only
//!   way to reach another monitor is Super held with no canvas grab in progress.
//!
//! Two pointer models, two mechanics for the same policy:
//!
//! - RELATIVE (mouse): the accumulator's overflow past the bounds IS a pan step, so
//!   pushing into an edge drives the canvas 1:1 for as long as the push lasts
//!   ([`pan`]). On top of that — unless `input_edge_pan_continuous` is off — parking
//!   against the extent SUSTAINS the pan off the frame clock ([`sustain`]), the way
//!   an RTS edge scroll does. The sustained speed is not a constant: it is seeded
//!   from how hard the pointer drove into the edge over its first moments there (see
//!   [`EdgeSeed`]), so arriving fast keeps moving fast and creeping in crawls. A
//!   canvas PAN grab is left out of it entirely — the push alone already carries
//!   the drag past the edge at mouse speed, and a sustain on top only fights it.
//! - ABSOLUTE (winit): the host clamps the position to the window and stops
//!   reporting once the pointer is pressed against the edge, so there is no
//!   overflow to integrate — it has to be SIMULATED. The pointer arms a hold as
//!   soon as it enters the [`EDGE_BAND`] near an edge ([`hold`]), and the per-frame
//!   [`tick`] autoscrolls at a rate that ramps with how deep into the band it sits:
//!   brushing the band creeps, pinned against the wall runs at full speed. A winit
//!   drag holds the host's implicit grab and does report out-of-window positions,
//!   so pushing further out keeps accelerating (up to [`EDGE_MAX_RAMP`]).
//!
//! DIRECT devices (touch, pen) are excluded from the absolute half: a finger has no
//! cursor to press against an edge, y5 already gives touch its own 2-finger pan, and
//! a band wide enough to be reachable would collide with the left-edge swipe that
//! opens the touch menu.
//!
//! Both mechanics are scaled by `preference.input_edge_pan_speed` ON TOP of the
//! pointer speed, so the edge pan keeps the feel of the pointer driving it.
//!
//! Neither runs while an OVERLAY owns the screen — the overview, the world picker,
//! the lock. The extent belongs to the camera only while the camera's world is the
//! thing being drawn; see [`pans`].

use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_seat_pointer_state::state::{EdgeHold, EdgeSeed};
use compositor_support_system_input_event_base::base::InputEvent;
use smithay::utils::{Physical, Point};
use std::time::{Duration, Instant};

/// How far from an edge an ABSOLUTE pointer starts pulling the canvas, in physical
/// px. Wide enough to enter deliberately before the host pins the cursor to the
/// window edge; the speed ramps from nothing at the inner boundary to full at the
/// edge, so brushing past it barely moves.
const EDGE_BAND: f64 = 24.0;
/// Canvas travel at the edge itself (ramp 1.0), in physical px/sec, BEFORE the
/// pointer speed and the edge-pan multiplier.
const EDGE_BASE_SPEED: f64 = 900.0;
/// Ceiling on the ramp. Only a winit drag can exceed 1.0 — its out-of-window
/// positions keep growing — so this is what stops a fling off-screen from launching
/// the canvas into the void.
const EDGE_MAX_RAMP: f64 = 3.0;

/// How long the ARRIVAL at an extent is sampled before the continuous pan's speed
/// is fixed. Long enough to catch the first handful of motion events at any polling
/// rate, short enough that the pan is under way before it reads as lag.
const EDGE_SEED_WINDOW: Duration = Duration::from_millis(120);
/// Bounds on the seeded sustained speed, physical px/sec before the edge-pan
/// multiplier. The floor stops a feather-light arrival from parking the canvas
/// dead; the ceiling stops a slam from launching it out of the world.
const EDGE_SUSTAIN_MIN: f64 = 150.0;
const EDGE_SUSTAIN_MAX: f64 = 3600.0;
/// Ceiling on a single sample's interval when measuring the arrival. A pointer that
/// pauses mid-window and resumes must not have the gap counted as time spent
/// pushing, or the average collapses.
const EDGE_SAMPLE_MAX_DT: f64 = 0.032;

thread_local! {
    /// When the previous relative motion event landed, for the true inter-event
    /// interval the arrival measurement divides by. Kept here rather than on the
    /// hold because the interval that matters most — the one carrying the pointer
    /// INTO the edge — spans the moment before any hold exists.
    static LAST_MOTION: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
    /// Set while [`tick`] replays the parked position through the absolute-motion
    /// path, so [`hold`] leaves the armed pan alone for that call. Main-thread only,
    /// like every other rim input path.
    static REPLAYING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// What a push past the screen extent does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Extent {
    /// Cross the cursor to the monitor placed across that edge.
    Teleport,
    /// Pan the camera by the push; the cursor stays pinned to the edge. See
    /// [`pan`] for which way the view travels (a pan grab continues its drag,
    /// everything else moves toward the pushed edge).
    Pan,
    /// Pin the cursor to the edge and nothing else.
    Clamp,
}

/// Resolve the policy for the CURRENT input state. Asked only once the pointer has
/// actually reached an extent.
///
/// With edge-pan on: a select box pins (panning under a rubber band would stretch
/// it over content the user can't see); Super held with NO canvas grab is the one
/// gesture that still crosses monitors; everything else — no grab at all, or a
/// move / scale / hand grab — pans. A press-drag pan counts as a grab
/// (`position_updating`) even though it arms no [`CanvasGrab`], so pushing out of
/// the screen mid-pan keeps panning instead of teleporting away from the drag.
pub fn resolve(state: &Loop) -> Extent {
    if !pans(state) {
        return Extent::Teleport;
    }
    let canvas = state.inner.canvas();
    if canvas.select_box() {
        return Extent::Clamp;
    }
    let grabbed = canvas.active_grab() || canvas.position_updating;
    if !grabbed && super_held(state) {
        return Extent::Teleport;
    }
    Extent::Pan
}

/// What to do when a resolved [`Extent::Teleport`] found nothing across that edge
/// (no layout, no placement abutting it, or the suppression lock is held). With
/// edge-pan on the push still belongs to the camera, so it pans rather than
/// leaving Super+edge as a dead zone on a single monitor.
pub fn fallback(state: &Loop) -> Extent {
    if pans(state) { Extent::Pan } else { Extent::Clamp }
}

/// Whether a push past an extent belongs to the CAMERA at all.
///
/// Off with the preference off — and off whenever an OVERLAY owns the screen: the
/// overview, the world picker, the lock. None of them is the focused world's
/// canvas, so driving into an edge to reach the overview's own menu bar (which
/// sits ON the top extent), or the far side of the picker's globe, would scroll a
/// world the user cannot see move, behind a frozen backdrop. Pin the cursor
/// instead, which is what the extent did before edge pan existed.
///
/// Both callers go through this, so Super+edge with an overlay up falls back to a
/// clamp rather than looping back into a pan.
///
/// The same predicate the pointer warp uses to decide the live bundle is not what
/// is on screen (`desktop.suppressed`) — one answer to one question.
fn pans(state: &Loop) -> bool {
    state.inner.preference.input_edge_pan
        && !compositor_orchestration_desktop_suppressed_base::base::suppressed(state)
}

/// Pan the camera by the cursor accumulator's outward overflow (physical screen
/// px, right/down positive), re-weighted by the edge-pan speed. Returns whether a
/// pan was actually emitted — the caller must re-derive its pane context when so,
/// because the camera moved.
///
/// The overflow already carries the pointer speed (the accumulator is advanced by
/// `delta × cursor_sensitivity`), so scaling it here is exactly "a multiplier on top
/// of the pointer speed".
///
/// Routed on the world input bus rather than written straight to the camera slot:
/// the camera system OWNS that slot and announces `CAMERA_MOVED` from it (the
/// background parallax rides on that), and the bus flushes its buffer
/// synchronously, so the new position is readable the moment this returns.
pub fn pan(state: &mut Loop, mx: f64, my: f64, pw: f64, ph: f64, dt: f64) -> bool {
    let g = gain(state);
    // A continuous pan is already delivering travel for this same interval, so the
    // push may only contribute what it EXCEEDS. Netting them is what keeps the speed
    // steady across all three transitions that otherwise lurch:
    //   - the arrival, where the push would land on top of the speed it just seeded;
    //   - the hand stopping, where a push counted separately would vanish and drop
    //     the canvas to the sustain alone;
    //   - the measurement freezing, where the push would suddenly become additive.
    // A HARDER shove still pulls ahead — that is exactly the excess — and with no
    // hold at all (continuous off) the push applies whole, as it always did.
    let (sx, sy) = state
        .inner
        .pointer()
        .edge_hold
        .map_or((0.0, 0.0), |h| (h.vx.abs() * dt, h.vy.abs() * dt));
    let dx = net(overflow(mx, pw) * g, sx);
    let dy = net(overflow(my, ph) * g, sy);
    if dx == 0.0 && dy == 0.0 {
        return false;
    }
    let (dx, dy) = travel(state, dx, dy);
    compositor_orchestration_input_drive_base::drive::route(state, InputEvent::PointerEdgePush { dx, dy });
    true
}

/// Sign the camera travel for a push of `(dx, dy)` physical px.
///
/// Normally the VIEW travels toward the edge being pushed (push into the right
/// edge → reveal what lies further right). That is what carrying a window out of
/// the screen needs: the world point under the pinned cursor moves the way the user
/// is pushing, so the dragged window travels with it.
///
/// A canvas PAN grab is the exception — it is already dragging the world along with
/// the cursor, so travelling toward the edge would REVERSE the content's on-screen
/// motion the instant the cursor pins. Inverted, the drag just keeps going: pan
/// indefinitely without lifting the mouse. The camera's own pan step contributes
/// nothing more once pinned (its screen delta is zero), so the two add up to
/// exactly the motion the user made.
fn travel(state: &Loop, dx: f64, dy: f64) -> (f64, f64) {
    if state.inner.canvas().position_updating { (-dx, -dy) } else { (dx, dy) }
}

/// ABSOLUTE pointer (winit): arm or drop the simulated edge pan for the reported
/// position, and return the position to actually use — pinned to the extent while
/// armed, since a winit drag reports positions well outside the window and those
/// belong to the camera, not to the cursor. `n` is the same position normalized to
/// the output, replayed by [`tick`] while the pointer sits still.
///
/// A no-op returning `pos` unchanged whenever the pointer is a DIRECT device, isn't
/// in the band, or the extent isn't a pan (edge-pan off, or a select box).
pub fn hold(state: &mut Loop, pos: Point<f64, Physical>, n: (f64, f64), pw: f64, ph: f64) -> Point<f64, Physical> {
    // A replay is [`tick`] re-stating where the pointer already is, not news about
    // where it went — the tick owns the hold across that call, so leave it be.
    // Without this the replay of a RELATIVE hold would land inside the band and
    // re-arm it as an absolute one.
    if REPLAYING.with(|r| r.get()) {
        return pos;
    }
    // Touch/pen reach this same path as emulated pointer events — see the module
    // docs for why they sit this out.
    if state.inner.touch.modality.is_direct() {
        release(state);
        return pos;
    }
    let (x, y) = (ramp(pos.x, pw), ramp(pos.y, ph));
    if x.is_none() && y.is_none() {
        release(state);
        return pos;
    }
    let mut policy = resolve(state);
    if policy == Extent::Teleport {
        // No absolute teleport path exists — the layout is crossed by the relative
        // accumulator only — so Super in the band lands here.
        policy = fallback(state);
    }
    if policy != Extent::Pan {
        release(state);
        return pos;
    }
    // Speed follows the pointer speed, then the user's edge-pan multiplier — the
    // same two factors the relative push carries.
    // Full-ramp speed. Unlike the relative half this DOES fold in the pointer
    // speed: there is no accumulator here to have scaled it already.
    let full = EDGE_BASE_SPEED * state.inner.preference.cursor_sensitivity * gain(state);
    let speed = |r: Option<(f64, f64)>| r.map_or(0.0, |(dir, t)| dir * t * full);
    // Keep the existing clock across events so a pointer sliding ALONG the band
    // doesn't restart its step and stall the scroll.
    let last = state.inner.pointer().edge_hold.map_or_else(Instant::now, |h| h.last);
    // No arrival seeding: an absolute pointer's speed comes from how deep into the
    // band it is, which is already the "how hard" measure the seed provides.
    state.inner.pointer_mut().edge_hold = Some(EdgeHold {
        dir_x: x.map_or(0.0, |(d, _)| d),
        dir_y: y.map_or(0.0, |(d, _)| d),
        vx: speed(x),
        vy: speed(y),
        seed: None,
        nx: n.0,
        ny: n.1,
        absolute: true,
        last,
    });
    Point::from((pos.x.clamp(0.0, pw), pos.y.clamp(0.0, ph)))
}

/// RELATIVE pointer: arm or drop the CONTINUOUS (RTS-style) edge pan for a cursor
/// that has just been pinned to an extent. `pinned` is the post-clamp accumulator.
///
/// This is what makes the pan keep running without being pushed again — the mouse
/// stops sending events the moment it stops moving, so the frame clock takes over.
/// The push itself is NOT folded in here: [`pan`] already applied it for this event,
/// so pushing into the edge rides on top of the sustained travel and only speeds
/// things up for as long as it lasts, exactly as an RTS edge scroll does.
///
/// Dropped when the cursor leaves the extent, when the extent is not a pan, when
/// the preference is off, or while a canvas PAN grab is driving — all of which
/// leave the push-only behaviour, i.e. mouse speed and nothing else.
///
/// `push` is this event's accumulator advance in physical px (`delta ×
/// cursor_sensitivity`), which is what the arrival is measured from — see
/// [`EdgeSeed`]. Only the component driving INTO the extent counts, so sliding fast
/// along an edge doesn't inflate the speed of the axis pressed against it.
/// Returns the interval since the previous motion event (seconds, capped), which
/// the caller feeds to [`pan`] so the push can be netted against the travel this
/// hold is already delivering.
pub fn sustain(
    state: &mut Loop,
    pinned: Point<f64, Physical>,
    pw: f64,
    ph: f64,
    panning: bool,
    push: (f64, f64),
) -> f64 {
    // The true interval since the previous motion event, banked on EVERY event so
    // the one that carried the pointer into the edge has an interval to be measured
    // against. Capped, so a pause never reads as time spent pushing.
    let now = Instant::now();
    let dt = LAST_MOTION
        .replace(Some(now))
        .map_or(0.0, |prev| now.duration_since(prev).as_secs_f64())
        .min(EDGE_SAMPLE_MAX_DT);

    // A canvas PAN grab gets the push and NOTHING else — no sustained speed of its
    // own, ever. It is already dragging the world along with the cursor and the
    // push is inverted into that drag ([`travel`]), so the two already come to
    // exactly the motion the hand made: pushing past the edge just keeps the drag
    // going, 1:1, at mouse speed. A sustained speed on top is a SECOND source
    // driving the same axis, and [`pan`] nets the push against it — so as the
    // measurement rises and falls the canvas alternates between running on the
    // push and running on the sustain instead of simply following the mouse.
    if !panning
        || !state.inner.preference.input_edge_pan_continuous
        || state.inner.canvas().position_updating
    {
        release(state);
        return dt;
    }
    let dir = |v: f64, max: f64| if v <= 0.0 { -1.0 } else if v >= max { 1.0 } else { 0.0 };
    let (dx, dy) = (dir(pinned.x, pw), dir(pinned.y, ph));
    if dx == 0.0 && dy == 0.0 {
        release(state);
        return dt;
    }
    // Continue the hold only if it is the same relative one against the same edges;
    // anything else (a fresh park, sliding into a corner, an absolute hold) starts
    // over, so the arrival that matters is the one just made.
    let mut hold = match state.inner.pointer().edge_hold {
        Some(h) if !h.absolute && h.dir_x == dx && h.dir_y == dy => h,
        _ => EdgeHold {
            dir_x: dx,
            dir_y: dy,
            vx: 0.0,
            vy: 0.0,
            seed: Some(EdgeSeed {
                since: now,
                push_x: 0.0,
                push_y: 0.0,
                active_x: 0.0,
                active_y: 0.0,
            }),
            absolute: false,
            nx: 0.0,
            ny: 0.0,
            last: now,
        },
    };
    hold.nx = pinned.x / pw.max(1.0);
    hold.ny = pinned.y / ph.max(1.0);
    absorb(&mut hold, push, dt, gain(state), now);
    state.inner.pointer_mut().edge_hold = Some(hold);
    dt
}

/// Fold one event's drive into the arrival measurement and re-derive the sustained
/// speed from it.
///
/// The speed is LIVE from the very first sample rather than published when the
/// window closes. Waiting would leave the canvas dead between the push ending and
/// the sustain starting — the user shoves, lets go, and the pan stalls for the rest
/// of the window before picking up again.
///
/// Only the component driving INTO the extent counts, per axis, so sliding fast
/// along an edge never inflates the speed of the axis pressed against it. The window
/// then FIXES the result, so a later nudge cannot drag the average back down — by
/// then a push is meant to add on top, not re-measure.
fn absorb(hold: &mut EdgeHold, push: (f64, f64), dt: f64, gain: f64, now: Instant) {
    let (dir_x, dir_y) = (hold.dir_x, hold.dir_y);
    let Some(seed) = hold.seed.as_mut() else { return };
    let px = (push.0 * dir_x).max(0.0);
    let py = (push.1 * dir_y).max(0.0);
    if px > 0.0 {
        seed.push_x += px;
        seed.active_x += dt;
    }
    if py > 0.0 {
        seed.push_y += py;
        seed.active_y += dt;
    }
    let rate = |d: f64, t: f64| {
        if t > 0.0 { (d / t).clamp(EDGE_SUSTAIN_MIN, EDGE_SUSTAIN_MAX) * gain } else { 0.0 }
    };
    hold.vx = dir_x * rate(seed.push_x, seed.active_x);
    hold.vy = dir_y * rate(seed.push_y, seed.active_y);
    let done = now.duration_since(seed.since) >= EDGE_SEED_WINDOW;
    if done {
        hold.seed = None;
    }
}

/// Fix the arrival measurement once its window has run out. Called from [`tick`] as
/// well as on motion: a pointer that slams into the edge and stops sends nothing
/// more, and its measurement still has to stop accepting samples.
fn ripen(hold: &mut EdgeHold, now: Instant) {
    if hold.seed.is_some_and(|s| now.duration_since(s.since) >= EDGE_SEED_WINDOW) {
        hold.seed = None;
    }
}

/// The user's edge-pan multiplier. NOT multiplied by the pointer speed here: a
/// seeded speed is measured off the accumulator, which the sensitivity already
/// scaled, so folding it in again would square it.
fn gain(state: &Loop) -> f64 {
    state.inner.preference.input_edge_pan_speed
}

/// Drop any armed edge pan. Called when the pointer leaves the band/extent, when the
/// extent stops being a pan, and when the host cursor leaves (or unfocuses) the winit
/// window, where no further motion would ever arrive to stop it.
pub fn release(state: &mut Loop) {
    if state.inner.pointer().edge_hold.is_some() {
        state.inner.pointer_mut().edge_hold = None;
    }
}

/// Drop only an ABSOLUTE-armed edge pan, leaving a relative pointer's continuous one
/// running. Two callers, same reasoning — what fed the absolute pan is gone:
/// - button-up, because the host's implicit drag grab is what was reporting the
///   positions past the extent;
/// - a relative motion event, because a relative pointer is now driving.
///
/// A relative continuous pan must survive both: the cursor is still parked at the
/// edge, and an RTS edge scroll neither stops because a button was let go nor
/// because the mouse moved a hair.
pub fn release_absolute(state: &mut Loop) {
    if state.inner.pointer().edge_hold.is_some_and(|h| h.absolute) {
        state.inner.pointer_mut().edge_hold = None;
    }
}

/// Per-frame step for an armed absolute edge pan. Pans by this frame's slice of the
/// held speed, then REPLAYS the parked position through the normal absolute-motion
/// path: the pointer sends nothing while it sits still, so without the replay the
/// world point under the cursor (and with it any move/scale grab, and the client
/// under the pointer) would never follow the camera.
pub fn tick(state: &mut Loop) {
    // `hooks` runs once per output per frame; only step during the pass for the
    // output the cursor is on, or the pan would run N times per frame on a
    // multi-monitor setup. Unset `render_output` = off the render loop, so proceed.
    if state.inner.render_output.is_some() && state.inner.render_output != state.inner.cursor_output {
        return;
    }
    let Some(mut hold) = state.inner.pointer().edge_hold else { return };
    // An overlay can open while the pointer is parked in the band — Super+Tab does
    // exactly that — and this clock would go on scrolling the world behind it. The
    // motion paths re-ask `resolve` on every event; a still pointer sends none, so
    // this is the only place that notices.
    if compositor_orchestration_desktop_suppressed_base::base::suppressed(state) {
        release(state);
        return;
    }
    let now = Instant::now();
    // Fix the arrival window here too: a pointer that slammed into the edge and
    // stopped sends no further motion, and this is the only clock still running.
    ripen(&mut hold, now);
    // Clamped so a stalled frame can't fling the camera across the world.
    let dt = (now - hold.last).as_secs_f64().clamp(0.0, 0.1);
    state.inner.pointer_mut().edge_hold = Some(EdgeHold { last: now, ..hold });
    if dt <= 0.0 {
        return;
    }
    let (dx, dy) = travel(state, hold.vx * dt, hold.vy * dt);
    compositor_orchestration_input_drive_base::drive::route(state, InputEvent::PointerEdgePush { dx, dy });
    REPLAYING.with(|r| r.set(true));
    crate::touch::emulate::move_to(state, hold.nx, hold.ny, now_msec());
    REPLAYING.with(|r| r.set(false));
    // This hook only runs per drawn frame, and a still pointer sends nothing that
    // would ask for one — keep the loop turning for as long as the scroll lasts.
    state.schedule_redraw();
}

/// How deep into one axis' edge band the pointer sits: `Some((direction, ramp))`
/// with ramp `0..EDGE_MAX_RAMP` — `1.0` at the edge itself, above that only for a
/// winit drag reporting outside the window — or `None` when it is not in the band
/// (or exactly on its inner boundary, where the speed would be zero anyway).
fn ramp(v: f64, max: f64) -> Option<(f64, f64)> {
    let depth = if v <= EDGE_BAND {
        EDGE_BAND - v
    } else if v >= max - EDGE_BAND {
        v - (max - EDGE_BAND)
    } else {
        return None;
    };
    let t = (depth / EDGE_BAND).min(EDGE_MAX_RAMP);
    if t <= 0.0 {
        return None;
    }
    Some((if v <= EDGE_BAND { -1.0 } else { 1.0 }, t))
}

fn now_msec() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u32)
        .unwrap_or(0)
}

/// `push` reduced toward zero by `sustained` (an unsigned distance), keeping its
/// sign. `0.0` once the sustained travel already covers it.
fn net(push: f64, sustained: f64) -> f64 {
    let excess = push.abs() - sustained;
    if excess <= 0.0 { 0.0 } else { excess.copysign(push) }
}

/// How far past `0..=max` the accumulator went, signed (negative = past the low
/// edge), `0.0` while inside.
fn overflow(v: f64, max: f64) -> f64 {
    if v < 0.0 {
        v
    } else if v > max {
        v - max
    } else {
        0.0
    }
}

/// Super (logo) currently held. Read straight off the keyboard's modifier state —
/// the canvas tool flags can't stand in for it: `finger_pan` is Super-held-ALONE
/// and is cleared by any other key in the combo, whereas crossing monitors must
/// work regardless of what else is down.
fn super_held(state: &Loop) -> bool {
    state
        .state
        .seat
        .seat
        .get_keyboard()
        .map(|k| k.modifier_state().logo)
        .unwrap_or(false)
}
