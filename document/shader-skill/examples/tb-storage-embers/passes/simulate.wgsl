// TB — `storage`, the effect: 4096 embers that live on the GPU.
//
// Glowing motes drift across the desktop, pull toward the cursor, and are pushed
// out of any window they touch — so they pour around the edges of your windows
// like sparks around a stone. Pass 2 of 3.
//
// WHY THIS NEEDS STORAGE AND A PERSISTENT TARGET CANNOT DO IT
// ----------------------------------------------------------
// Two separate reasons, and the second is the interesting one.
//
// 1. A particle has STATE that outlives the frame — position and velocity — and
//    it is not attached to a pixel. `parts[i]` is written by invocation `i`,
//    whichever pixel that invocation happens to be shading. A render target can
//    only ever write the pixel being shaded, so it cannot hold a particle at all.
//
// 2. Each ember then SPLATS its light into a grid at the cells its own POSITION
//    selects — `atomicAdd(&field.cell[c], ...)` where `c` is data, not geometry.
//    Many embers land in the same cell and add up; most cells get nothing. That
//    is a scatter in the strict sense: the destination is computed from a value,
//    and it is exactly what rendering cannot express.
//
// The alternative without scatter is to loop over all 4096 embers per pixel,
// which is two million pixels × 4096 — the grid turns that into a few atomic adds
// per ember and one bilinear read per pixel.
//
// THE SPLAT IS SPREAD, NOT A POINT — AND THAT IS WHY THE GRID IS INVISIBLE
// -----------------------------------------------------------------------
// The first version of this dropped each ember's whole contribution into the ONE
// cell containing it, and the grid was plainly visible: an ember's light jumped
// from cell to cell as it moved, and every mote was a hard-edged 12-pixel square.
//
// That was not a consequence of the field being a buffer, which is the obvious
// suspect and the wrong one. A nearest-cell splat quantises the light no matter
// what the field is stored in — the same code writing to a storage IMAGE would
// look identical, because hardware filtering happens on the READ and the damage
// was already done on the write.
//
// The fix is here, on the write: each ember distributes its light over a 3×3
// neighbourhood weighted by where it actually sits inside its cell, normalised so
// the total is exactly its brightness however it straddles the boundaries. Moving
// an ember by a tenth of a cell now changes the field by a tenth of a cell's
// worth, so nothing pops, and one ember is a soft round blob rather than a square.
//
// @prop pull    float default=0.55 min=0.0 max=2.0  step=0.01 label="Cursor pull"   group="Embers"
// @prop push    float default=1.0  min=0.0 max=3.0  step=0.01 label="Window push"   group="Embers"
// @prop drift   float default=0.35 min=0.0 max=2.0  step=0.01 label="Drift"         group="Embers"
// @prop damp    float default=0.94 min=0.80 max=1.0 step=0.005 label="Damping"      group="Embers"
// @prop size    float default=1.0  min=0.4  max=2.5 step=0.01 label="Mote size"     group="Embers"
//
// KNOWN LIMIT — one step per COMPOSITE, not per frame
// ---------------------------------------------------
// The step is a fixed 0.016 s and runs once every time this pass is recorded,
// which is once per OUTPUT. On a two-monitor desktop the embers move at twice the
// speed they do on one, and at a refresh rate other than 60 Hz proportionally
// wrong. Nothing in the push carries a frame delta, and the honest fix needs
// somewhere to keep the previous timestamp AND a read of it that does not race —
// the double-buffered storage this engine does not have yet. Written down rather
// than hidden behind a fudge factor that would be wrong in a different way.

const COUNT: u32 = 4096u;
const GRID_W: u32 = 320u;
const GRID_H: u32 = 180u;
const CELLS: u32 = 57600u;
/// Splat weight as fixed point — `atomicAdd` is integer. 4096 leaves a spread
/// weight of a hundredth still landing on ~40 counts rather than truncating to
/// nothing, and 4096 embers at full brightness in one cell reach ~34M, well
/// inside a `u32`.
const UNIT: f32 = 4096.0;

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

struct Windows {
    count: u32,
    _pad: vec3<u32>,
    rects: array<vec4<f32>, 256>,
    srcs: array<vec4<f32>, 256>,
    attrs: array<vec4<f32>, 256>,
};
@group(1) @binding(0) var<uniform> windows: Windows;

struct Pointer {
    at: vec4<f32>,
    moment: vec4<f32>,
};
@group(1) @binding(3) var<uniform> pointer: Pointer;

// `@group(2)` in the manifest's SORTED name order: `field` before `parts`.
struct Field { cell: array<atomic<u32>, 57600>, };
@group(2) @binding(0) var<storage, read_write> field: Field;

/// One ember: `xy` = position in screen UV, `zw` = velocity per second.
struct Parts { p: array<vec4<f32>, 4096>, };
@group(2) @binding(1) var<storage, read_write> parts: Parts;

fn hash(n: u32) -> f32 {
    var x = n * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return f32((x >> 22u) ^ x) / 4294967295.0;
}

/// Advance one ember and splat it. Split out because the invocation may own
/// several — see the strided loop in `fs_main`.
fn step_one(idx: u32, t: f32, p_pull: f32, p_push: f32, p_drift: f32, p_damp: f32, p_size: f32) {
    var e = parts.p[idx];
    // Fresh: the zeroed state. Scatter them, and give each a different phase so
    // the field does not pulse in unison.
    if (all(e == vec4<f32>(0.0))) {
        e = vec4<f32>(hash(idx), hash(idx + 9871u), 0.0, 0.0);
    }
    var pos = e.xy;
    var vel = e.zw;

    // A slow curl, so the field is alive with nothing else happening.
    let phase = hash(idx + 4242u) * 6.283;
    vel = vel + vec2<f32>(
        sin(pos.y * 9.0 + t * 0.6 + phase),
        cos(pos.x * 11.0 - t * 0.5 + phase),
    ) * p_drift * 0.02;

    // CURSOR — pull, falling off with distance so only the local field responds.
    let to_cursor = pointer.at.xy - pos;
    let d = max(length(to_cursor), 0.0001);
    vel = vel + (to_cursor / d) * p_pull * 0.06 * exp(-d * 4.0);
    // …and a shove outward while a button is held, so clicking scatters them.
    if (u32(pointer.at.z) != 0u) {
        vel = vel - (to_cursor / d) * p_pull * 0.20 * exp(-d * 6.0);
    }

    // WINDOWS — an ember inside a window is pushed out through its nearest edge.
    // Front to back; the first window that covers it owns it, which matches how
    // the desktop is stacked.
    let n = min(windows.count, 256u);
    for (var k = 0u; k < n; k = k + 1u) {
        let i = n - 1u - k;
        if (windows.attrs[i].x > 0.5) {
            continue;
        }
        let r = windows.rects[i];
        let hi = r.xy + r.zw;
        if (pos.x < r.x || pos.x > hi.x || pos.y < r.y || pos.y > hi.y) {
            continue;
        }
        let dl = pos.x - r.x;
        let dr = hi.x - pos.x;
        let dt = pos.y - r.y;
        let db = hi.y - pos.y;
        let m = min(min(dl, dr), min(dt, db));
        var out = vec2<f32>(-1.0, 0.0);
        if (m == dr) { out = vec2<f32>(1.0, 0.0); }
        else if (m == dt) { out = vec2<f32>(0.0, -1.0); }
        else if (m == db) { out = vec2<f32>(0.0, 1.0); }
        vel = vel + out * p_push * 0.10;
        break;
    }

    vel = vel * p_damp;
    pos = fract(pos + vel * 0.016 + vec2<f32>(1.0));
    parts.p[idx] = vec4<f32>(pos, vel);

    // An ember that leaves one edge reappears at the opposite one — `fract` wraps
    // it — and a BRIGHT mote doing that reads as a snap across the screen. Fading
    // it out as it approaches any edge and back in on the far side hides the
    // teleport without touching the physics.
    let edge = min(min(pos.x, 1.0 - pos.x), min(pos.y, 1.0 - pos.y));
    let visible = smoothstep(0.0, 0.04, edge);
    // Brighter the faster it moves, so a stirred field glows and a settled one dims.
    let heat = clamp(0.35 + length(vel) * 6.0, 0.0, 2.0) * visible;
    if (heat <= 0.0) {
        return;
    }

    // THE SCATTER, spread over 3×3. `g` is the ember's CONTINUOUS position in
    // cell space; the weights come from its distance to each cell centre, so they
    // change smoothly as it moves and the grid never shows through.
    let g = pos * vec2<f32>(f32(GRID_W), f32(GRID_H)) - vec2<f32>(0.5);
    let base = vec2<i32>(floor(g));
    let f = g - floor(g);
    let radius = max(p_size, 0.05);

    // Two passes over the neighbourhood: total the weights, then deposit shares of
    // `heat`. Normalising is what keeps an ember's total light constant however it
    // straddles a boundary — without it the field pulses at the cell frequency,
    // which is the grid becoming visible again by a subtler route.
    var total = 0.0;
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let o = vec2<f32>(f32(dx), f32(dy)) - f;
            total = total + max(1.0 - length(o) / (radius * 1.6), 0.0);
        }
    }
    if (total <= 0.0) {
        return;
    }
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let c = base + vec2<i32>(dx, dy);
            if (c.x < 0 || c.x >= i32(GRID_W) || c.y < 0 || c.y >= i32(GRID_H)) {
                continue;
            }
            let o = vec2<f32>(f32(dx), f32(dy)) - f;
            let w = max(1.0 - length(o) / (radius * 1.6), 0.0) / total;
            if (w <= 0.0) {
                continue;
            }
            atomicAdd(&field.cell[u32(c.y) * GRID_W + u32(c.x)], u32(heat * w * UNIT));
        }
    }
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    // `res` is this pass's own target (quarter scale) — the engine pushes each
    // intermediate the size of what it draws into.
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let px = vec2<u32>(frag.xy);
    let w = u32(res.x);
    let stride = max(w * u32(res.y), 1u);
    let start = px.y * w + px.x;

    let p_pull = pc.params[0].x;
    let p_push = pc.params[0].y;
    let p_drift = pc.params[0].z;
    let p_damp = pc.params[0].w;
    let p_size = pc.params[1].x;

    // STRIDED, so this works at any output size. One invocation per ember is the
    // common case and the loop runs once; on a small output where the target holds
    // fewer pixels than there are embers, each invocation takes several — still
    // one owner per index, so no invocation ever writes another's ember.
    for (var i = start; i < COUNT; i = i + stride) {
        step_one(i, t, p_pull, p_push, p_drift, p_damp, p_size);
    }
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}
