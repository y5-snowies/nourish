// dragons, pass 2 — the world band. This pass owns compositing: it draws the
// landscape, every window, every dragon and every jet of fire, in one ordered
// sweep.
//
// WHY OWNERSHIP
// -------------
// `windows: world` hands this pass the whole band instead of a finished
// desktop. That is what buys the effect: a dragon can be drawn BETWEEN two
// windows, or between a window and the background, because this pass decides
// the order rather than reading a picture in which the order is already
// resolved. It is also what lets fire BURN THROUGH a window — the hole shows
// what is behind, and only a pass that draws the band itself has that.
//
// THE DEPTH WEAVE, AND WHY IT IS SOFT
// -----------------------------------
// Each dragon carries a depth, 0 (behind everything) to 1 (in front). The band
// is an ORDERED list, so depth maps straight onto an index: depth d belongs just
// before window `floor(d * (count+1))`.
//
// That depth is evaluated PER PIXEL and shifted along the body, so the head sits
// slightly in front of the tail and an animal can straddle the plane of a
// window. But a hard per-pixel slot assignment CUTS the body along the line
// where the slot changes — and that line falls wherever it falls, including
// across the middle of a window's glass, where it matches no visible edge.
//
// So the boundary is soft: a pixel near a slot edge is drawn in BOTH adjacent
// slots with complementary weights, and the body fades from one side of the
// plane to the other instead of being cut. `dragon_depth` also does its
// crossing on the approach rather than over the target.
//
// @prop dcount  float default=5.0  min=1.0  max=8.0  step=1.0   label="Dragons"          group="Dragons"
// @prop dsize   float default=0.34 min=0.10 max=0.70 step=0.01  label="Dragon size"      group="Dragons"
// @prop dspeed  float default=1.00 min=0.20 max=3.00 step=0.05  label="Flight speed"     group="Dragons"
// @prop dweave  float default=0.55 min=0.00 max=1.00 step=0.01  label="Depth weave"      group="Dragons"
// `flap` is atlas FRAMES PER SECOND, and the sheet holds one full wingbeat in
// eight — so 8 is one beat a second, about right for something that size.
// @prop flap    float default=9.00 min=2.00 max=24.0 step=0.50  label="Wing beat"        group="Dragons"
// @prop roll    float default=1.00 min=0.00 max=1.00 step=0.05  label="Barrel roll"      group="Dragons"
// `frate` is the fraction of runs that are attack runs: 1.0 means every pass
// ends in fire, 0.0 means they only ever fly by.
// @prop frate   float default=0.80 min=0.00 max=1.00 step=0.05  label="Fire frequency"   group="Fire"
// @prop freach  float default=1.05 min=0.20 max=1.60 step=0.02  label="Flame reach"      group="Fire"
// @prop fbright float default=1.50 min=0.00 max=3.00 step=0.05  label="Flame brightness" group="Fire"
// @prop scorch  float default=1.00 min=0.00 max=2.50 step=0.05  label="Window heat"      group="Fire"
// @prop tear    float default=0.85 min=0.00 max=2.00 step=0.05  label="Burn-through"     group="Fire"

enable wgpu_binding_array;

#import dragons::common::d_hash11
#import dragons::common::d_fbm
#import dragons::common::heat_cell
#import dragons::common::heat_pack
#import dragons::common::dragon_seed
#import dragons::common::dragon_kind
#import dragons::common::dragon_kind_at
#import dragons::common::dragon_side
#import dragons::common::dragon_fire_window
#import dragons::common::dragon_run
#import dragons::common::dragon_stride
#import dragons::common::dragon_attack
#import dragons::common::dragon_path
#import dragons::common::dragon_bank
#import dragons::common::dragon_depth
#import dragons::common::dragon_scale

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 3>,
};
var<immediate> pc: Push;

// Sorted by binding NAME: breath, dragon, flame, heatmap, land.
@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var breath: texture_2d<f32>;
@group(0) @binding(2) var dragon: texture_2d<f32>;
@group(0) @binding(3) var flame: texture_2d<f32>;
@group(0) @binding(4) var heatmap: texture_2d<f32>;
@group(0) @binding(5) var land: texture_2d<f32>;

// The world's fire, surviving from frame to frame. This pass is the only writer:
// wherever it draws flame, it deposits into the cell under that pixel, and the
// `heat` pass publishes the decayed, spread field back as `heatmap`.
struct Field {
    cell: array<atomic<u32>, 82944>,
};
@group(2) @binding(0) var<storage, read_write> field: Field;

struct Windows {
    count: u32,
    _pad: vec3<u32>,
    rects: array<vec4<f32>, 256>,
    srcs: array<vec4<f32>, 256>,
    attrs: array<vec4<f32>, 256>,
};
@group(1) @binding(0) var<uniform> windows: Windows;
@group(1) @binding(1) var win_tex: binding_array<texture_2d<f32>>;

// The dragon atlas is two-dimensional: a column per wingbeat frame, a row per
// roll angle. The fire atlases are plain 4x4 strips.
const FLAPS: f32 = 8.0;
const DCOLS: f32 = 16.0;   // FLAPS x jaws-shut/jaws-open
const ROLLS: f32 = 12.0;
const COLS: f32 = 4.0;
const ROWS: f32 = 4.0;
const FRAMES: f32 = 16.0;
const MAXD: u32 = 8u;
const SMOKE: i32 = 3;
const TAU: f32 = 6.2831853;

/// UV of `local` (0..1 within one cell) in cell (col, row) of a cols x rows
/// atlas.
///
/// The half-texel inset matters: the sampler clamps at the edge of the whole
/// image, not per cell, so a sample at a cell border bleeds in the neighbouring
/// frame — on a wing that shows as a faint second wing.
fn atlas_cell(dims: vec2<f32>, local: vec2<f32>, col: f32, row: f32,
              cols: f32, rows: f32) -> vec2<f32> {
    let grid = vec2<f32>(cols, rows);
    let cell = vec2<f32>(clamp(col, 0.0, cols - 1.0), clamp(row, 0.0, rows - 1.0));
    let inset = 0.5 / dims * grid;
    let inner = clamp(local, inset, vec2<f32>(1.0) - inset);
    return (cell + inner) / grid;
}

/// One index into a 4x4 strip.
fn atlas_uv(dims: vec2<f32>, local: vec2<f32>, frame: f32) -> vec2<f32> {
    let f = clamp(frame, 0.0, COLS * ROWS - 1.0);
    return atlas_cell(dims, local, f % COLS, floor(f / COLS), COLS, ROWS);
}

/// Distance from `p` to segment AB — used to spill firelight onto everything
/// near the jet, not just onto what it hits.
fn seg_dist(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let denom = max(dot(ab, ab), 1e-6);
    let h = clamp(dot(p - a, ab) / denom, 0.0, 1.0);
    return length(p - a - ab * h);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    let ar = res.x / res.y;
    // Aspect space: x scaled so a sprite is not stretched and a circle is round.
    let p = vec2<f32>(uv.x * ar, uv.y);

    let dcount = pc.params[0].x;
    let dsize = pc.params[0].y;
    let dspeed = pc.params[0].z;
    let dweave = pc.params[0].w;
    let flap = pc.params[1].x;
    let rollamt = pc.params[1].y;
    let frate = pc.params[1].z;
    let freach = pc.params[1].w;
    let fbright = pc.params[2].x;
    let scorch = pc.params[2].y;
    let tearamt = pc.params[2].z;

    let n = min(windows.count, 256u);
    let dn = u32(clamp(dcount, 1.0, f32(MAXD)));
    // The world's fire as of last frame: what everything here is standing in.
    //
    // Modulated by a drifting noise so the field BURNS rather than sitting
    // there as a stain. The buffer holds how much fuel a patch has; this is the
    // flame on it, and it has to move or the whole thing reads as a decal.
    let amb_raw = textureSampleLevel(heatmap, samp, uv, 0.0).r;
    let live = 0.62 + 0.85 * d_fbm(vec2<f32>(uv.x * 20.0 + t * 1.3,
                                             uv.y * 20.0 - t * 2.4), 3);
    let amb = clamp(amb_raw * live, 0.0, 1.0);
    let ddims = vec2<f32>(textureDimensions(dragon));
    let bdims = vec2<f32>(textureDimensions(breath));
    let fdims = vec2<f32>(textureDimensions(flame));

    // ---- every dragon, resolved ONCE for this pixel ------------------------
    var d_rgba: array<vec4<f32>, 8>;
    var d_slot: array<u32, 8>;
    var d_frac: array<f32, 8>;
    var d_shadow: array<f32, 8>;
    var d_mouth: array<vec2<f32>, 8>;
    var d_anchor: array<vec2<f32>, 8>;   // the fixed point on the window it burns
    var d_dir: array<vec2<f32>, 8>;      // the nose direction; fire leaves along it
    var d_impact: array<vec2<f32>, 8>;   // where the jet lands, swept across the burst
    var d_fire: array<vec4<f32>, 8>;     // burst, jet growth, target, seed
    var d_heat: array<vec2<f32>, 8>;     // window heat (with afterglow), smoke age
    var d_ext: array<f32, 8>;

    for (var k = 0u; k < dn; k = k + 1u) {
        let seed = dragon_seed(k);

        // ---- the plan for this run -----------------------------------------
        let rp = dragon_run(k, t, dspeed);
        let run = rp.x;
        let u_raw = rp.y;
        // The window this run is aimed at, chosen once per run and NOT revisited
        // — the target must not drift while the fire is already out.
        var tgt = 0u;
        var lo = vec2<f32>(ar * 0.42, 0.30);
        var hi = vec2<f32>(ar * 0.58, 0.42);
        if (n > 0u) {
            tgt = u32(d_hash11(run * 1.37 + seed * 4.1) * f32(n)) % n;
            let tr = windows.rects[tgt];
            lo = vec2<f32>(tr.x * ar, tr.y);
            hi = vec2<f32>((tr.x + tr.z) * ar, tr.y + tr.w);
        }

        // The kind must be resolved AGAINST THE TARGET — a window with no room
        // above it cannot be attacked from above, and every part of the plan
        // has to agree about that.
        let kind = dragon_kind_at(k, run, lo);
        let u = dragon_stride(u_raw, kind);

        let attack = dragon_attack(k, run, lo, hi, ar);
        let pos = dragon_path(k, u, lo, hi, run, ar);
        // A step either side of here, on the SAME plan: one gives the heading,
        // the pair gives the curvature the dragon banks into.
        let ahead = dragon_path(k, min(u + 0.008, 1.0), lo, hi, run, ar);
        let back = dragon_path(k, max(u - 0.008, 0.0), lo, hi, run, ar);
        var dir = ahead - pos;
        if (length(dir) < 1e-5) {
            dir = vec2<f32>(1.0, 0.0);
        }
        dir = normalize(dir);

        let kn = dragon_fire_window(kind);
        // The aim is FROZEN at the moment fire starts, not tracked.
        //
        // Tracking cannot be made stable: a dragon flies PAST its target, and
        // the direction to a point you pass sweeps through half a turn as you
        // go by — so a look-at slams from one extreme to the other exactly
        // during the burst, however hard the offset is clamped. Taking the
        // bearing once, from where the animal will be when it opens up, gives a
        // fixed attitude to hold through the strike, which is what a firing run
        // looks like anyway: nose steady, fire sweeping across the target.
        let u_fire = dragon_stride(kn.x, kind);
        let pos_fire = dragon_path(k, u_fire, lo, hi, run, ar);

        // Where the fire lands: the point on the window nearest to THE DRAGON
        // AT THE MOMENT IT FIRES, fixed for the run.
        //
        // Taking it from the attack point instead is wrong in a way that shows
        // plainly: the attack point lies beyond the window on the far side,
        // because the run crosses to it — but fire starts before the dragon
        // gets there. The jet therefore left from the near side and landed on
        // the opposite edge, burning the far side of a window while the flame
        // was visibly over this one.
        var anchor = clamp(pos_fire, lo, hi);
        if (kind == 2u) {
            // Pouring down: straight below it, on the top edge.
            anchor = vec2<f32>(clamp(pos_fire.x, lo.x, hi.x), lo.y + 0.015);
        }
        d_anchor[k] = anchor;

        // Turn onto the target across the APPROACH, well before the fire comes
        // out, and release after the burst. Ramping this in as the fire starts
        // makes the animal snap round at the moment it opens up, which next to
        // another window reads as it switching targets mid-breath.
        let look = smoothstep(0.10, kn.x, u_raw) * smoothstep(kn.w + 0.18, kn.w, u_raw);
        let to_aim = anchor - pos_fire;
        // How far off the flight line the target sits. Kept, because fire may
        // only leave along the nose — see below.
        var aim_off = 0.0;
        if (length(to_aim) > 1e-4) {
            // Held relative to the flight direction and clamped, so it reads as
            // the animal angling its head, not as a sprite pinned to a compass.
            let a0 = atan2(dir.y, dir.x);
            let a1 = atan2(to_aim.y, to_aim.x);
            var delta = a1 - a0;
            delta = delta - TAU * floor((delta + 3.14159265) / TAU);   // to [-pi, pi]
            aim_off = delta;
            // A deliberate SWEEP across the burst: the head rakes through a
            // slow arc rather than holding one line, so the fire is dragged
            // across the target instead of painting a single spot.
            let bu = clamp((u_raw - kn.x) / max(kn.z - kn.x, 1e-3), 0.0, 1.0);
            let sweep = (bu - 0.5) * 0.55;
            // 54 degrees. Direction variety comes from the PATHS differing —
            // a run that pours fire down now descends onto its target, so its
            // flight line already points down — not from cranking the head over
            // at an angle no animal holds.
            let turn = (clamp(delta, -0.95, 0.95) + sweep) * look;
            dir = vec2<f32>(cos(a0 + turn), sin(a0 + turn));
        }

        // Whether this run breathes at all has to be settled BEFORE the sprite
        // is sampled: the atlas carries jaws-shut and jaws-open columns, and an
        // animal that is about to breathe fire opens its mouth first.
        let aimed = abs(aim_off) < 1.05;
        let fires = aimed && n > 0u
                    && d_hash11(run * 5.91 + seed * 3.31) < clamp(frate, 0.0, 1.0);
        // Open a moment before the fire comes out, shut a moment after it stops.
        let maw = select(0.0, 1.0,
                         fires && u_raw > kn.x - 0.07 && u_raw < kn.w + 0.08);

        // Stride phase, not raw phase: the layer crossing is tied to where the
        // dragon IS along its plan, so it finishes on the approach for every
        // kind of run — including a hover, which arrives early in raw time.
        let depth = dragon_depth(k, u, run);
        let ext = dsize * dragon_scale(k, depth);
        // Art faces RIGHT, so flying left is a mirror — and the facing comes
        // from the RUN's direction, which is fixed, never from the current
        // heading, which passes through vertical on a diving attack.
        let flip = dragon_side(k, run);
        // BOUNDED PITCH. A side-on sprite rotated toward vertical reads as an
        // animal standing on its tail, and past vertical it reads as upside
        // down — neither of which any roll frame is responsible for. The path
        // tangent does go near-vertical on a hard pull-out, so the displayed
        // angle is bounded, and `max(fx, ...)` stops the nose ever swinging
        // behind the animal.
        let fx = dir.x * flip;
        let ang = clamp(atan2(dir.y, max(fx, 0.30)), -0.95, 0.95);
        // The nose follows the BOUNDED angle, not the raw heading, so the fire
        // leaves exactly where the head is drawn pointing.
        let nose = vec2<f32>(cos(ang), sin(ang)) * flip;

        // Roll. The atlas holds twelve real roll poses, so this is an INDEX now
        // rather than a squash of the picture: one barrel roll on the run-in,
        // plus a lean into whatever the path is doing.
        //
        // The roll is finished before the fire starts, and the bank is released
        // across the burst: a dragon steadies itself to breathe. The bank comes
        // from path curvature, which PEAKS at the flare, so leaving it on is
        // what made them lean hardest exactly while breathing.
        let steady = smoothstep(kn.x - 0.10, kn.x, u_raw)
                     * smoothstep(kn.w + 0.14, kn.w, u_raw);
        let barrel = TAU * smoothstep(0.06, 0.26, u) * rollamt;
        let bank = dragon_bank(back, pos, ahead);
        let roll_ang = barrel + bank * 0.9 * (1.0 - steady);
        // The half-row offset is not cosmetic. A completed barrel roll lands on
        // exactly 2pi, which is the BOUNDARY between the last roll cell and the
        // first — so the least jitter in the bank flips the pose 30 degrees back
        // and forth for the rest of the run. Offsetting by half a cell puts the
        // resting attitude in the middle of a cell, where nothing tips it over.
        let rrow = floor(fract(roll_ang / TAU + 0.5 / ROLLS) * ROLLS);

        // Wingbeat. Hovering is hard work and beats faster; a raking pass
        // stalls its wings through the flare. Both are expressed as a SHIFT of
        // the animation clock rather than a change of rate, because a rate that
        // varies with time makes the phase jump backwards.
        let hover = select(0.0, 1.0, kind == 1u)
                    * smoothstep(0.30, 0.45, u_raw) * smoothstep(0.80, 0.65, u_raw);
        let stall = smoothstep(0.32, 0.50, u_raw) * smoothstep(0.86, 0.64, u_raw)
                    * (1.0 - hover);
        let rate = flap * (0.85 + 0.30 * seed);
        let fcol = floor(rate * (t - 0.35 * stall + 0.25 * hover) + seed * FLAPS) % FLAPS;
        let mcol = maw * FLAPS + fcol;

        // Pixel -> sprite cell.
        let cs = cos(-ang);
        let sn = sin(-ang);
        let q = (p - pos) / max(ext, 1e-4);
        let u0 = vec2<f32>(q.x * cs - q.y * sn, q.x * sn + q.y * cs);
        let local = vec2<f32>(u0.x * flip, u0.y) + vec2<f32>(0.5, 0.5);

        var rgba = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        if (all(local >= vec2<f32>(0.0)) && all(local <= vec2<f32>(1.0))) {
            let tx = textureSampleLevel(
                dragon, samp, atlas_cell(ddims, local, mcol, rrow, DCOLS, ROLLS), 0.0);
            // Per-dragon hide, so a flight of them is not one animal copied.
            let ht = d_hash11(f32(k) * 17.31 + 0.7);
            let tint = mix(vec3<f32>(0.82, 0.90, 1.04), vec3<f32>(1.16, 0.94, 0.76), ht)
                       * mix(0.84, 1.14, d_hash11(f32(k) * 4.91 + 3.3));
            // Aerial perspective: the far dragon is flatter, cooler and hazier,
            // exactly as the far ridge is in `land.wgsl`. Same rule, same scene.
            var rgb = tx.rgb * tint * mix(0.55, 1.0, depth);
            rgb = mix(rgb, vec3<f32>(0.26, 0.24, 0.30), (1.0 - depth) * 0.45);
            // Lit by whatever fire it is flying through — its own or another's.
            rgb = rgb + vec3<f32>(1.00, 0.45, 0.15) * amb * 0.55;
            rgba = vec4<f32>(rgb, tx.a);
        }
        d_rgba[k] = rgba;
        d_ext[k] = ext;

        // Silhouette offset away from the low sun: the shadow it throws on
        // anything it passes in FRONT of, and the cue that sells which side of
        // a window it is on.
        let sh = vec2<f32>(-0.045, -0.030) * ext * 1.6;
        let qs = (p - sh - pos) / max(ext, 1e-4);
        let u0s = vec2<f32>(qs.x * cs - qs.y * sn, qs.x * sn + qs.y * cs);
        let ls = vec2<f32>(u0s.x * flip, u0s.y) + vec2<f32>(0.5, 0.5);
        var sha = 0.0;
        if (all(ls >= vec2<f32>(0.0)) && all(ls <= vec2<f32>(1.0))) {
            sha = textureSampleLevel(
                dragon, samp, atlas_cell(ddims, ls, mcol, rrow, DCOLS, ROLLS), 0.0).a;
        }
        d_shadow[k] = sha;

        // Per-pixel slot, with a SOFT boundary.
        let dpix = clamp(depth + dweave * (local.x - 0.5) * 0.75, 0.0, 1.0);
        let sf = dpix * f32(n + 1u);
        var base = floor(sf);
        var frac = smoothstep(0.35, 0.65, sf - base);
        if (base >= f32(n)) {
            base = f32(n);
            frac = 0.0;
        }
        d_slot[k] = u32(base);
        d_frac[k] = frac;

        // Fire belongs to the plan: released across the flare. `frate` decides
        // how many runs are attack runs at all — the rest are fly-bys.
        var burst = 0.0;
        var jet = 0.0;
        // `fires` was settled above, so the jaws and the jet cannot disagree
        // about whether this run is an attack.
        if (fires) {
            burst = smoothstep(kn.x, kn.y, u_raw) * smoothstep(kn.z, kn.w, u_raw);
            jet = smoothstep(kn.x, kn.y + 0.05, u_raw);
        }
        // Glass does not go cold the instant the fire stops. The afterglow is
        // analytic — a decay from the end of the burst — so it needs no buffer
        // and cannot drift with frame rate.
        let after = exp(-max(u_raw - kn.w, 0.0) * 13.0)
                    * smoothstep(kn.x, kn.y, u_raw) * 0.55;

        // The mouth, carried through the same rotation as the sprite: the head
        // sits at (0.86, 0.43) in cell space.
        let uh = vec2<f32>(0.36 * flip, -0.07);
        let cs2 = cos(ang);
        let sn2 = sin(ang);
        let qh = vec2<f32>(uh.x * cs2 - uh.y * sn2, uh.x * sn2 + uh.y * cs2);
        d_mouth[k] = pos + qh * ext;
        // The nose direction. By construction this IS `dir`: a cell-space
        // offset along +x maps to `flip * (cos ang, sin ang)`, and `ang` was
        // built from `dir` through the same flip. Fire leaves along this.
        d_dir[k] = nose;
        // Where the fire actually lands: along the nose, at the target's
        // distance. Because the head sweeps through the burst, this point rakes
        // across the window rather than sitting on one spot.
        let fwd = max(dot(anchor - d_mouth[k], nose), 0.12);
        d_impact[k] = d_mouth[k] + nose * fwd;
        // Fire from the far side of the window plane is seen through everything
        // in front of it, so it burns dimmer.
        let seen = mix(0.45, 1.0, depth);
        d_fire[k] = vec4<f32>(burst * seen, jet, f32(tgt), seed);
        d_heat[k] = vec2<f32>(max(burst, after) * seen, max(u_raw - kn.w, 0.0));
    }

    // ---- the band, back to front ------------------------------------------
    var col = textureSampleLevel(land, samp, uv, 0.0).rgb;
    // The ground and the air hold the fire that crossed them: scorched where it
    // burned, still glowing while it cools.
    // Darkening stays light-handed: the field covers sky as well as ground, and
    // a heavy scorch would paint a dark smear across mid-air where a jet passed.
    col = mix(col, col * 0.72, clamp(amb * 0.7, 0.0, 1.0));
    col = col + vec3<f32>(1.00, 0.40, 0.12) * amb * 0.9;

    for (var i = 0u; i <= n; i = i + 1u) {
        // Dragons behind window `i`, including the part-weight of any that
        // straddle this boundary.
        for (var k = 0u; k < dn; k = k + 1u) {
            var w = 0.0;
            if (d_slot[k] == i) {
                w = 1.0 - d_frac[k];
            } else if (d_slot[k] + 1u == i) {
                w = d_frac[k];
            }
            if (w > 0.002) {
                let s = d_rgba[k];
                if (s.a > 0.002) {
                    col = mix(col, s.rgb, s.a * w);
                }
            }
        }
        if (i >= n) {
            break;
        }

        // The window itself. Under ownership this pass MUST draw every entry or
        // the desktop loses windows.
        let r = windows.rects[i];
        let hi = r.xy + r.zw;
        if (uv.x < r.x || uv.x > hi.x || uv.y < r.y || uv.y > hi.y) {
            continue;
        }
        let s = windows.srcs[i];
        let lw = (uv - r.xy) / max(r.zw, vec2<f32>(0.0001));
        let px = textureSampleLevel(win_tex[i], samp, s.xy + lw * s.zw, 0.0);
        var wc = px.rgb;
        var a = clamp(px.a, 0.0, 1.0) * windows.attrs[i].y;

        // attrs.x = 0 is a client window; 1 is an iced world panel, which is not
        // a window and is blitted plainly.
        if (windows.attrs[i].x < 0.5) {
            // Shadows from anything passing in FRONT of this window.
            var shade = 0.0;
            for (var k = 0u; k < dn; k = k + 1u) {
                if (d_slot[k] > i) {
                    shade = max(shade, d_shadow[k]);
                }
            }
            wc = wc * (1.0 - 0.34 * shade);

            var heat = 0.0;
            for (var k = 0u; k < dn; k = k + 1u) {
                if (d_heat[k].x <= 0.005) {
                    continue;
                }
                // Distance to the whole JET, not just to where it ends — and
                // with no check that this is the window being aimed at.
                //
                // Fire burns what it touches. Gating this on the target index
                // is why one strike could visibly cross two windows and mark
                // only one of them: the second was never even considered, no
                // matter how squarely the flame lay across it.
                let dline = seg_dist(p, d_mouth[k], d_impact[k]);
                let dend = length(p - d_impact[k]);
                heat = heat + d_heat[k].x
                       * (exp(-dline * 11.0) + 0.70 * exp(-dend * 8.5));
            }
            // And whatever the world's fire has settled on goes on burning,
            // whoever lit it and however long ago.
            heat = max(heat, amb);

            if (heat > 0.004) {
                // The glass heats where the jet lands, and keeps glowing for a
                // moment after.
                let h = clamp(heat * scorch, 0.0, 2.0);
                wc = wc * mix(1.0, 1.30, min(h, 1.0))
                     + vec3<f32>(1.00, 0.42, 0.14) * h * 0.55;

                // BURN-THROUGH. Where the fire is hottest the window tears
                // open along a noise contour and what is behind it shows
                // through — the landscape, or a dragon that happens to be back
                // there. Only a pass that owns the band can do this: `content`
                // has already resolved the window over its background, so there
                // would be nothing behind to reveal.
                //
                // It heals as the heat decays, because the level is driven by
                // the heat itself — so the tear is a brief wound, not damage.
                let burn = clamp(heat * scorch * 1.5, 0.0, 1.0) * tearamt;
                if (burn > 0.01) {
                    let nz = d_fbm(vec2<f32>(uv.x * 34.0 + 5.1, uv.y * 34.0), 4);
                    let e = burn - nz;
                    let hole = smoothstep(0.0, 0.09, e);
                    a = a * (1.0 - hole);
                    // A charred margin outside the hole, and a hot ember rim
                    // right at the edge where the sheet is still burning.
                    let char_ = smoothstep(-0.30, -0.04, e) * (1.0 - hole);
                    wc = wc * (1.0 - 0.62 * char_);
                    let rim = smoothstep(-0.11, -0.015, e) * (1.0 - smoothstep(-0.015, 0.05, e));
                    wc = wc + vec3<f32>(1.60, 0.52, 0.10) * rim * 2.4;
                }
            }
        }
        col = col * (1.0 - a) + wc * a;
    }

    // ---- the jets ----------------------------------------------------------
    var fire = vec3<f32>(0.0, 0.0, 0.0);
    var spill = 0.0;
    if (n > 0u) {
        for (var k = 0u; k < dn; k = k + 1u) {
            let fs = d_fire[k];
            let mouth = d_mouth[k];
            let anchor = d_anchor[k];
            let ext = d_ext[k];

            if (fs.x > 0.01) {
                // ONE stretched quad sampling the animated jet atlas, rather
                // than a row of round puffs. A puff repeated along a line never
                // reads as flame however well it is drawn; the atlas holds the
                // tongues, and they stream because the frames advect.
                // The jet leaves along the NOSE, never toward the target point.
                // Aiming the quad at the anchor instead is what made dragons
                // breathe backwards: the head turn is clamped to a cone, so
                // whenever the target sat outside it the animal faced one way
                // and the fire went another.
                let dirf = d_dir[k];
                let perp = vec2<f32>(-dirf.y, dirf.x);
                // Reach to the target measured ALONG the nose, so the jet ends
                // where the fire actually arrives.
                let seg = anchor - mouth;
                let base_len = max(dot(seg, dirf), 0.12);
                let jl = base_len * clamp(freach, 0.2, 1.6) * clamp(fs.y, 0.05, 1.0);
                let rel = p - mouth;
                let along = dot(rel, dirf);
                let across = dot(rel, perp);
                let halfw = jl * 0.27;
                if (along >= 0.0 && along <= jl && abs(across) <= halfw) {
                    let lf = vec2<f32>(along / jl, across / halfw * 0.5 + 0.5);
                    let frf = floor(t * 24.0 + fs.w * FRAMES) % FRAMES;
                    let tx = textureSampleLevel(breath, samp,
                                                atlas_uv(bdims, lf, frf), 0.0);
                    let flicker = 0.86 + 0.28 * sin(t * 37.0 + fs.w * 19.0);
                    let lit = tx.a * fs.x * flicker;
                    fire = fire + tx.rgb * lit;

                    // DEPOSIT. The fire writes itself into the world wherever it
                    // is actually drawn — no second copy of the flight maths to
                    // drift out of step with this one. `atomicMax` is what makes
                    // it safe: every pixel of the jet, across every dragon, races
                    // to write the same cells, and max has no order to get wrong.
                    if (lit > 0.03) {
                        atomicMax(&field.cell[heat_cell(uv)], heat_pack(lit, t));
                    }
                }

                // Firelight falls on the whole scene near the jet — the
                // landscape, and any window beside the one being burned.
                spill = spill + fs.x * exp(-seg_dist(p, mouth, d_impact[k]) * 5.0);
            }

            // Smoke, after the fire stops: it rises off the impact and thins.
            // Alpha-blended, not additive — smoke occludes what is behind it.
            let sage = d_heat[k].y;
            if (sage > 0.0 && sage < 0.30) {
                let smk = smoothstep(0.0, 0.05, sage) * smoothstep(0.30, 0.12, sage);
                for (var j = 0; j < SMOKE; j = j + 1) {
                    let hj = d_hash11(fs.w * 5.9 + f32(j) * 11.3);
                    let c = d_impact[k] + vec2<f32>((hj - 0.5) * 0.28 * ext,
                                               -(0.10 + 3.0 * sage) * ext);
                    let sz = ext * (0.30 + 2.4 * sage) * (0.7 + 0.5 * hj);
                    let lf = (p - c) / max(sz, 1e-4) + vec2<f32>(0.5, 0.5);
                    if (any(lf < vec2<f32>(0.0)) || any(lf > vec2<f32>(1.0))) {
                        continue;
                    }
                    let frf = 12.0 + floor(hj * 4.0);
                    let tx = textureSampleLevel(flame, samp, atlas_uv(fdims, lf, frf), 0.0);
                    let sa = clamp(tx.a * smk * 0.55, 0.0, 1.0);
                    col = mix(col, vec3<f32>(0.085, 0.080, 0.090), sa);
                }
            }
        }
    }
    // Additive: overlapping fire should build rather than occlude, which alpha
    // compositing would get wrong.
    col = col + fire * fbright;
    col = col + vec3<f32>(1.00, 0.46, 0.17) * spill * 0.35 * fbright;

    // No sRGB encode here: the after-content `glow` pass writes the final
    // `output` and does the encode once. Doing it twice is the classic
    // washed-out bundle.
    return vec4<f32>(max(col, vec3<f32>(0.0)), 1.0);
}
