// dragons — shared ground truth: noise, and where every dragon is right now.
//
// The flight functions live here rather than in the band pass because BOTH the
// silhouette and the fire read them, and a dragon whose body and whose breath
// disagreed about where its head is would be very obvious. One definition, two
// readers.
//
// Everything is a pure function of (index, time). No state, no buffers: a
// dragon's whole life is arithmetic on its index, which is what lets any pixel
// ask about any dragon without the engine having to remember anything.
#define_import_path dragons::common

fn d_hash11(x: f32) -> f32 {
    return fract(sin(x * 91.3458) * 47453.5453);
}

fn d_hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

/// Value noise, smoothstep-interpolated.
fn d_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = d_hash21(i);
    let b = d_hash21(i + vec2<f32>(1.0, 0.0));
    let c = d_hash21(i + vec2<f32>(0.0, 1.0));
    let d = d_hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn d_fbm(p: vec2<f32>, octaves: i32) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < octaves; i = i + 1) {
        v = v + amp * d_noise(q);
        q = q * 2.03;
        amp = amp * 0.5;
    }
    return v;
}

/// Ridged noise — the profile a mountain silhouette wants, because a ridge line
/// is where a height field creases, not where it peaks.
fn d_ridge(p: vec2<f32>, octaves: i32) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < octaves; i = i + 1) {
        v = v + amp * (1.0 - abs(d_noise(q) * 2.0 - 1.0));
        q = q * 2.11;
        amp = amp * 0.5;
    }
    return v;
}

/// The per-dragon constant. Everything else keys off this.
fn dragon_seed(i: u32) -> f32 {
    return d_hash11(f32(i) * 3.71 + 1.37);
}

// ------------------------------------------------------------------ fire field
//
// Fire is not drawn and forgotten: it is DEPOSITED into a storage buffer that
// outlives the frame, so what it touches goes on burning after the jet has
// swept past. That buffer is the environment's state — a coarse grid over the
// screen holding how hot each patch of the world is right now — and everything
// downstream reads it: the landscape scorches, ANY window it crosses heats and
// tears (not just the one being aimed at), and the dragons themselves are lit
// by fire they are flying through.
//
// A `persist` target could hold a picture like this, but not build it: a
// fragment shader writes only the pixel it is shading, whereas the fire has to
// write where it LANDS and spread from there. Scatter is the whole reason this
// is a storage buffer.
//
// The grid is fixed rather than tied to any pass's resolution, because two
// passes at different scales have to agree on which cell a point falls in.
const GRID_W: f32 = 384.0;
const GRID_H: f32 = 216.0;
const GRID_CELLS: u32 = 82944u;      // 384 * 216
const HEAT_DECAY: f32 = 0.55;        // e-folds per second — fire that lingers

/// Which cell a screen UV falls in.
fn heat_cell(uv: vec2<f32>) -> u32 {
    let c = clamp(floor(uv * vec2<f32>(GRID_W, GRID_H)),
                  vec2<f32>(0.0), vec2<f32>(GRID_W - 1.0, GRID_H - 1.0));
    return u32(c.y) * u32(GRID_W) + u32(c.x);
}

/// Pack a deposit: TIME in the high half, heat in the low half.
///
/// The ordering is deliberate, because the cells are written with `atomicMax`
/// and the packing decides what "max" means. Time-major makes the NEWEST
/// deposit win, which is what a live field wants. Heat-major would make each
/// cell hold its historical peak for ever, so a patch once hit hard could never
/// register a fresh small fire again.
fn heat_pack(h: f32, t: f32) -> u32 {
    let hq = u32(clamp(h, 0.0, 1.0) * 65535.0);
    let tq = u32(t * 32.0) & 0xFFFFu;
    return (tq << 16u) | hq;
}

/// Current heat of a packed cell: what was deposited, decayed by how long ago.
///
/// Decaying on READ rather than writing decayed values back means no pass has
/// to own the clock, and the result cannot drift with frame rate. The u32
/// subtraction wraps correctly, so the 2048-second time field wrapping is
/// harmless.
fn heat_now(packed: u32, t: f32) -> f32 {
    let hq = f32(packed & 0xFFFFu) / 65535.0;
    if (hq <= 0.0) {
        return 0.0;
    }
    let tq = packed >> 16u;
    let now = u32(t * 32.0) & 0xFFFFu;
    let age = f32((now - tq) & 0xFFFFu) / 32.0;
    return hq * exp(-age * HEAT_DECAY);
}

// ---------------------------------------------------------------- attack runs
//
// A dragon does not wander. Its life is a sequence of RUNS, and one run is a
// plan: enter from off-screen, curve in on a chosen window, roll, flare and
// breathe on it, then break away off the far side. The next run picks a new
// aim and a new entry.
//
// The whole plan is a function of (dragon index, run number), so every pixel
// derives the same plan independently with nothing stored between frames — and
// the run number is just `floor(time / duration)`, which is what makes a dragon
// deliberately SPAWN rather than fade in wherever it happened to be.

fn d_bez3(a: vec2<f32>, b: vec2<f32>, c: vec2<f32>, u: f32) -> vec2<f32> {
    let m = 1.0 - u;
    return m * m * a + 2.0 * m * u * b + u * u * c;
}

/// Cubic Hermite: position at `s` given endpoints and their TANGENTS.
///
/// Tangents rather than control points, because the tangent is the thing that
/// has to match where two pieces of a path meet. Two quadratics joined at a
/// shared point agree on where they are and can still disagree on how fast they
/// are going through it, which reads as the animal changing its mind halfway —
/// two paths lerped together rather than one line flown.
fn d_hermite(p0: vec2<f32>, m0: vec2<f32>, p1: vec2<f32>, m1: vec2<f32>,
             s: f32) -> vec2<f32> {
    let s2 = s * s;
    let s3 = s2 * s;
    return (2.0 * s3 - 3.0 * s2 + 1.0) * p0
         + (s3 - 2.0 * s2 + s) * m0
         + (-2.0 * s3 + 3.0 * s2) * p1
         + (s3 - s2) * m1;
}

/// How this run is flown. 0 = STRAFE (a fast pass, breathing across the flare),
/// 1 = HOVER (stand off, hang in the air, sustained burn), 2 = HIGH (cross well
/// above the target and pour fire straight down onto it).
///
/// Chosen per (dragon, run), so the same animal hunts differently each time out
/// and several dragons on screen are never doing the same thing.
fn dragon_kind(i: u32, run: f32) -> u32 {
    let h = d_hash11(run * 4.19 + dragon_seed(i) * 13.7);
    if (h > 0.72) { return 1u; }
    if (h > 0.42) { return 2u; }
    return 0u;
}

/// Which way this run crosses the screen: -1 right-to-left, +1 left-to-right.
///
/// Constant for the whole run, and the ONLY thing the sprite's facing may key
/// off. Mirroring on the instantaneous heading instead looks fine until a
/// dragon noses down onto a target — then the horizontal component of its
/// heading sits near zero, and the animal flips back and forth between facings
/// on nothing.
fn dragon_side(i: u32, run: f32) -> f32 {
    return select(-1.0, 1.0, d_hash11(run * 3.71 + dragon_seed(i) * 11.0) > 0.5);
}

/// The kind this run can actually be flown as, given where the target sits.
///
/// A window whose top edge is already near the top of the screen has no "above"
/// to attack from — a dragon sent there ends up level with the glass or inside
/// it, breathing sideways at nothing. So that run is flown as a raking pass
/// instead. Everything that depends on the kind must ask THIS, not
/// `dragon_kind`, or the path and the fire will disagree about the plan.
fn dragon_kind_at(i: u32, run: f32, lo: vec2<f32>) -> u32 {
    let k = dragon_kind(i, run);
    if (k == 2u && lo.y < 0.16) {
        return 0u;
    }
    return k;
}

/// The phases at which fire starts and stops for a run of this kind:
/// (rise from, rise to, fall from, fall to).
fn dragon_fire_window(kind: u32) -> vec4<f32> {
    if (kind == 1u) { return vec4<f32>(0.34, 0.42, 0.76, 0.62); }   // sustained
    if (kind == 2u) { return vec4<f32>(0.40, 0.47, 0.68, 0.58); }   // a poured sheet
    return vec4<f32>(0.36, 0.44, 0.66, 0.56);                       // a raking pass
}

/// x = run number, y = raw phase 0..1 through that run.
fn dragon_run(i: u32, t: f32, speed: f32) -> vec2<f32> {
    let s = dragon_seed(i);
    // Per-dragon run length, so a flight of them never moves in lockstep.
    // ~6-9s per run at default speed, which crosses the screen in about three
    // and a half seconds — a stoop, not a drift.
    let dur = mix(6.0, 9.5, d_hash11(f32(i) * 5.13 + 2.9)) / max(speed, 0.08);
    let c = t / dur + s * 7.0;
    return vec2<f32>(floor(c), fract(c));
}

/// Fast strides: the phase is remapped so the dragon surges and eases rather
/// than tracking its path at a constant crawl. The derivative of this is the
/// speed, and it swings roughly 0.45x to 1.55x across the run.
fn dragon_stride(u: f32, kind: u32) -> f32 {
    // Amplitude is bounded by the requirement that the remap stay monotonic:
    // 0.042 * 2pi * 3 = 0.79 < 1, so the speed dips to ~0.2x but never reverses.
    let ripple = 0.042 * sin(6.2831853 * 3.0 * u);
    if (kind == 1u) {
        // HOVER: ease hard toward the middle of the plan and hang there. The
        // path is unchanged — it is the RATE along it that stalls, which is why
        // the dragon appears to stop over its target and hold station.
        let x = 2.0 * u - 1.0;
        let held = sign(x) * pow(abs(x), 2.6);
        return clamp(0.5 + 0.5 * held - ripple * 0.35, 0.0, 1.0);
    }
    return clamp(u - ripple, 0.0, 1.0);
}

/// Where this run means to end up, from the target window's actual EDGES.
///
/// Anchoring on the window's centre plus a fixed offset is not good enough: a
/// tall window, or one already near the top of the screen, leaves a "from
/// above" run level with the glass or below it. Standing off from the edge that
/// matters — the side for a raking pass, the TOP for a run that pours fire
/// down — is right whatever the window's size or position.
///
/// Fixed for the whole run, so nothing re-aims halfway through.
fn dragon_attack(i: u32, run: f32, lo: vec2<f32>, hi: vec2<f32>, ar: f32) -> vec2<f32> {
    let s = dragon_seed(i);
    let side = select(-1.0, 1.0, d_hash11(run * 3.71 + s * 11.0) > 0.5);
    let kind = dragon_kind_at(i, run, lo);
    let mid = (lo + hi) * 0.5;
    let half = (hi - lo) * 0.5;

    var a = vec2<f32>(mid.x + side * (half.x + 0.13), mid.y);      // raking pass
    if (kind == 1u) {
        a = vec2<f32>(mid.x + side * (half.x + 0.26), mid.y - 0.03);  // stand off
    } else if (kind == 2u) {
        a = vec2<f32>(mid.x + side * 0.06, lo.y - 0.30);              // above the TOP
    }
    // Keep it on screen; a target hard against an edge must not push the
    // dragon out of frame, where it would breathe from somewhere unseen.
    return vec2<f32>(clamp(a.x, 0.06 * ar, 0.94 * ar), clamp(a.y, 0.055, 0.90));
}

/// The planned path, in ASPECT SPACE (x already multiplied by aspect, so the
/// sprite is never stretched). Two quadratics sharing a tangent at the attack
/// point: run in, flare, break away.
fn dragon_path(i: u32, u: f32, lo: vec2<f32>, hi: vec2<f32>, run: f32, ar: f32) -> vec2<f32> {
    let s = dragon_seed(i);
    let h1 = d_hash11(run * 3.71 + s * 11.0);
    let h2 = d_hash11(run * 8.13 + s * 4.31);
    let h3 = d_hash11(run * 2.37 + s * 6.17);
    // Which side it comes in from, and where it crosses the frame edge.
    let side = select(-1.0, 1.0, h1 > 0.5);
    // The SHAPE of the run is what makes one attack look different from
    // another — not the angle the head is cranked to. A raking pass comes in
    // level and climbs away; a pour-down enters high and keeps descending
    // through the target, so its flight line is already pointing down when the
    // fire comes out and the jet goes down with it.
    let kind = dragon_kind_at(i, run, lo);
    var enter_y = mix(0.06, 0.58, h2);
    var leave_y = mix(0.04, 0.28, h3);          // climb away
    if (kind == 2u) {
        enter_y = mix(0.02, 0.16, h2);          // in from high up
        leave_y = mix(0.52, 0.86, h3);          // and still going down
    }
    let enter = vec2<f32>(ar * 0.5 - side * ar * 0.92, enter_y);
    let leave = vec2<f32>(ar * 0.5 + side * ar * 0.92, leave_y);
    // One definition of where the run ends up, shared with the pass that aims
    // the fire — so the animal and its breath cannot disagree about the plan.
    let attack = dragon_attack(i, run, lo, hi, ar);
    // Roughly a third of runs come UP from below instead of stooping from
    // above. Without this every approach has the same shape, and a flight of
    // dragons all diving identically reads as a loop rather than as hunting.
    let h4 = d_hash11(run * 5.17 + s * 9.43);
    let arc = select(-0.42 - 0.20 * h2, 0.52 + 0.20 * h2, h4 > 0.66);

    // ONE line, flown through the target — not two curves meeting on it.
    //
    // The tangent AT the attack point runs along the whole span of the run, so
    // the dragon carries its speed and direction straight through the strike.
    // Both halves are Hermite pieces sharing that exact tangent and the same
    // parameter rate, which makes the join invisible: same position, same
    // direction, same speed.
    let span = leave - enter;
    let m_at = span * 0.62;
    let m_en = (attack - enter) * 1.10 + vec2<f32>(0.0, arc);
    let m_lv = (leave - attack) * 1.10;
    if (u < 0.5) {
        return d_hermite(enter, m_en, attack, m_at, u * 2.0);
    }
    return d_hermite(attack, m_at, leave, m_lv, (u - 0.5) * 2.0);
}

/// Bank angle, from how hard the path is turning here.
///
/// This replaces an earlier full barrel roll. A barrel roll cannot be faked by
/// squashing a side-on sprite: at 90 degrees the whole animal — head, tail and
/// all — collapses to a sliver and then snaps back mirrored, which reads as a
/// glitch rather than as a manoeuvre. A BANK is what a side-on camera actually
/// shows of a turn, and it stays well short of edge-on, so the sprite never
/// degenerates.
///
/// `turn` is the signed curvature of the plan at this instant, so the dragon
/// leans into its own turn rather than rolling on a timer that knows nothing
/// about where the path is going.
fn dragon_bank(back: vec2<f32>, here: vec2<f32>, ahead: vec2<f32>) -> f32 {
    let a = here - back;
    let b = ahead - here;
    let la = length(a);
    let lb = length(b);
    if (la < 1e-6 || lb < 1e-6) {
        return 0.0;
    }
    // The turn is measured as an ANGLE between the two steps, not as a raw
    // cross product: the cross scales with step length, so it reports a tight
    // turn taken slowly as no turn at all.
    let an = a / la;
    let bn = b / lb;
    let turn = atan2(an.x * bn.y - an.y * bn.x, dot(an, bn));
    return clamp(turn * 26.0, -0.95, 0.95);
}

/// 0 = behind every window, 1 = in front of every one.
///
/// Planned, not wandering: each run starts at one side of the window plane and
/// ends at the other, and the crossing happens across the flare — so the pass
/// through the desktop is the same moment as the attack.
fn dragon_depth(i: u32, u: f32, run: f32) -> f32 {
    let a = d_hash11(run * 6.11 + f32(i) * 2.71);
    let b = d_hash11(run * 9.73 + f32(i) * 5.33);
    let from_d = mix(0.06, 0.94, a);
    let to_d = mix(0.06, 0.94, b);
    // NOTE: `u` here must be the STRIDE-remapped phase — position along the
    // plan, not elapsed time. A hover run reaches its target early in raw time,
    // so keying the crossing to raw time puts it right back over the glass.
    //
    // The crossing happens on the APPROACH (0.10..0.32 of the way along), before
    // the dragon is over its target — not across the flare.
    //
    // Changing layer while inside the window's own area puts the front/behind
    // boundary in the middle of the glass, where it corresponds to no visible
    // edge, and the animal appears to be sliced by nothing. Crossing on the way
    // in means it arrives already committed to one side, and the only edge that
    // ever cuts it is the window's own.
    return clamp(mix(from_d, to_d, smoothstep(0.10, 0.32, u)), 0.0, 1.0);
}

/// Size on screen. Tied to depth, because that is what perspective IS: the near
/// dragon is bigger, and the far one is smaller AND hazier (the pass fades it).
fn dragon_scale(i: u32, depth: f32) -> f32 {
    let s2 = d_hash11(f32(i) * 9.17 + 5.71);
    return mix(0.62, 1.10, s2) * mix(0.70, 1.18, depth);
}
