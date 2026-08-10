// storm-forest, pass 4/5 — the weather, and the whole window band.
//
// This bundle owns the band (`"windows": "world"`), which is what makes the tear
// possible: the engine never blits a window over the background, so when the
// crack opens there is still a forest behind it to show through.
//
// Order of business, back to front:
//   forest (blurred, rain-streaked glass) → far rain → the bolt → every window,
//   shaken, rained on, lit, possibly torn → near rain → impact flare.
//
// TWO CLOCKS FIRE A STRIKE
// ------------------------
// 1. The periodic one, from lib/quake.wgsl. It is a pure function of time, which
//    is what lets the CPU pointer evaluator reproduce the shake exactly, so
//    clicks track the windows while they rattle.
// 2. A window OPENING (`window_times`). Unpredictable by definition, so the
//    pointer evaluator cannot know about it — it reads no window state. That is
//    why an open-strike shakes at a fraction of the amplitude and is ADDED on
//    top of the periodic offset: the pointer-matched part stays exact, and the
//    uncorrected part stays down at a few pixels for well under a second.
//
// @prop rain      float default=1.00  min=0.0  max=2.5  step=0.01  label="Rain"                   group="Weather"
// @prop wind      float default=0.35  min=-1.0 max=1.0  step=0.01  label="Wind"                   group="Weather"
// @prop period    float default=9.00  min=2.0  max=40.0 step=0.5   label="Seconds between bolts"  group="Lightning"
// @prop flash     float default=1.00  min=0.0  max=2.5  step=0.01  label="Lightning brightness"   group="Lightning"
// @prop shake     float default=0.011 min=0.0  max=0.05 step=0.001 label="Lightning shake"        group="Lightning"
// @prop tearlife  float default=2.20  min=0.0  max=6.0  step=0.05  label="Tear duration"          group="Tear"
// @prop tearsize  float default=0.55  min=0.0  max=1.5  step=0.01  label="Tear size"              group="Tear"
// @prop tearglow  float default=1.00  min=0.0  max=3.0  step=0.01  label="Tear glow"              group="Tear"
// @prop glass     float default=0.55  min=0.0  max=2.0  step=0.01  label="Rain on backdrop"       group="Weather"
// @prop bolt      float default=1.00  min=0.0  max=2.5  step=0.01  label="Bolt brightness"        group="Lightning"
// @prop lightpick float default=0.80  min=0.0  max=2.5  step=0.01  label="Window lighting"        group="Lightning"
// @prop rainfront float default=0.60  min=0.0  max=2.5  step=0.01  label="Rain in front"          group="Weather"
// @prop rainwin   float default=0.90  min=0.0  max=2.5  step=0.01  label="Rain on windows"        group="Weather"

enable wgpu_binding_array;

#import storm::quake::strike_at
#import storm::quake::quake_offset
#import storm::quake::quake_shape
#import storm::quake::q_hash
#import storm::field::s_fbm
#import storm::field::s_noise
#import storm::field::s_hash11
#import storm::field::flash_curve
#import storm::field::rain_layer

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 4>,
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
@group(1) @binding(1) var win_tex: binding_array<texture_2d<f32>>;

struct Times {
    count: u32,
    _pad: vec3<u32>,
    life: array<vec4<f32>, 256>,    // x = opened, y = entered, z = left
    state: array<vec4<f32>, 256>,
    drag: array<vec4<f32>, 256>,
};
@group(1) @binding(2) var<uniform> times: Times;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var forest: texture_2d<f32>;

const JAG_OCT: i32 = 3;   // @optimized 2

/// Distance, in window pixels, to the nearest arm of the fracture.
///
/// Three rays out of the impact point, each with a sinusoidal wander across its
/// own axis and a taper that thins it toward the tip. Dividing by the taper is
/// what makes the crack narrow as it runs out, rather than stopping square.
fn crack_dist(p: vec2<f32>, o: vec2<f32>, seed: f32, len: f32) -> f32 {
    let d = p - o;
    // The impact itself is a small hole, so the arms have somewhere to start.
    var best = max(length(d) - len * 0.035, 0.0) * 2.0;
    for (var j = 0; j < 3; j = j + 1) {
        let fj = f32(j);
        let ang = s_hash11(seed * 3.1 + fj * 7.7) * 6.2831853;
        let ca = cos(ang);
        let sa = sin(ang);
        let u = d.x * ca + d.y * sa;
        let v = -d.x * sa + d.y * ca;
        let l = len * (0.55 + 0.75 * s_hash11(seed * 5.3 + fj * 3.3));
        if (u < 0.0 || u > l) {
            continue;
        }
        let wander = sin(u * 0.031 + fj * 5.0 + seed) * 7.0
                   + sin(u * 0.091 + fj * 2.1 + seed * 2.0) * 3.2
                   + sin(u * 0.210 + fj) * 1.1;
        let taper = 1.0 - u / l;
        best = min(best, abs(v - wander) / max(taper, 0.14));
    }
    return best;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    let t = pc.res_zoom_time.w;
    let aspect = vec2<f32>(res.x / max(res.y, 1.0), 1.0);

    let rain = pc.params[0].x;
    let wind = pc.params[0].y;
    let period = pc.params[0].z;
    let flash_amt = pc.params[0].w;
    let shake = pc.params[1].x;
    let tearlife = pc.params[1].y;
    let tearsize = pc.params[1].z;
    let tearglow = pc.params[1].w;
    let glass = pc.params[2].x;
    let bolt = pc.params[2].y;
    let lightpick = pc.params[2].z;
    let rainfront = pc.params[2].w;
    let rainwin = pc.params[3].x;

    let n = min(windows.count, 256u);
    let periodic = strike_at(t, period);

    // ---------------------------------------------------------------- strike
    // A window that just opened outranks the clock. `life.x` is the absolute
    // moment it opened, or NEVER (-1), so the test is `>= 0.0` — a forgotten
    // check would read NEVER as an event in the distant past.
    var open_t = -1000.0;
    var open_i = 0u;
    var has_open = false;
    let nt = min(min(times.count, 256u), n);
    for (var i = 0u; i < nt; i = i + 1u) {
        if (windows.attrs[i].x > 0.5) {
            continue;                       // panels do not summon lightning
        }
        let op = times.life[i].x;
        if (op >= 0.0 && op > open_t) {
            open_t = op;
            open_i = i;
            has_open = true;
        }
    }

    var st = periodic.x;
    var seed = periodic.y;
    var forced = false;
    if (has_open && open_t > periodic.x) {
        st = open_t;
        seed = floor(open_t * 37.0);        // a distinct hash seed per event
        forced = true;
    }
    let age = t - st;
    let fl = flash_curve(age) * flash_amt;

    // Which window took the hit. A forced strike always hits the window that
    // caused it; otherwise pick one off the strike seed.
    var has_hit = false;
    var hit_i = 0u;
    if (forced) {
        hit_i = open_i;
        has_hit = true;
    } else if (n > 0u) {
        let want = u32(q_hash(seed * 5.3 + 2.0) * f32(n));
        for (var j = 0u; j < n; j = j + 1u) {
            let i = (want + j) % n;
            if (windows.attrs[i].x < 0.5) {
                hit_i = i;
                has_hit = true;
                break;
            }
        }
    }

    // A forced strike shakes at full amplitude, same as a periodic one. It only
    // used to be scaled down to keep the pointer warp's error small; with the
    // warp gone there is nothing to stay in step with.
    let forced_amt = select(0.0, shake, forced);

    let imp_local = vec2<f32>(
        0.18 + 0.64 * q_hash(seed * 9.1 + 4.0),
        0.14 + 0.46 * q_hash(seed * 11.3 + 6.0),
    );
    let bolt_x = fract(sin(seed * 12.9898) * 43758.5453);
    var strike_pt = vec2<f32>(0.12 + 0.76 * q_hash(seed * 3.7 + 1.0), 0.58);
    if (has_hit) {
        let hr = windows.rects[hit_i];
        strike_pt = hr.xy + hr.zw * imp_local
                  + quake_offset(shake, period, t, f32(hit_i))
                  + quake_shape(forced_amt, age, seed, f32(hit_i));
    }

    // ------------------------------------------------------------- backdrop
    // Rain running on the glass, displacing the out-of-focus forest behind it.
    // Two plain noise lookups, not two fbm chains — this runs at full
    // resolution on every pixel of the screen and the image it displaces is
    // already blurred, so the extra octaves bought nothing visible.
    var buv = uv;
    if (glass > 0.0) {
        let gp = vec2<f32>(uv.x * 15.0 * aspect.x, uv.y * 15.0 - t * 0.09);
        let d1 = s_noise(gp);
        let d2 = s_noise(gp * 1.9 + vec2<f32>(37.0, 11.0));
        // The vertical term is stronger: water runs down, it does not wander.
        buv = uv + (vec2<f32>(d1, d2) - 0.5) * vec2<f32>(0.012, 0.020) * glass;
    }
    var col = textureSampleLevel(forest, samp, clamp(buv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb;

    // Wind arrives in gusts. A perfectly constant slant is the other half of
    // what makes procedural rain look artificial.
    let slant = wind * 0.45 * (0.70 + 0.60 * s_noise(vec2<f32>(t * 0.17, 3.0)));
    let rain_lit = 0.30 + 2.20 * fl;

    // Two far sheets, drawn before the band, so the windows occlude them for
    // free. Distant rain: many fine threads, moderate speed, short streaks.
    var far = rain_layer(uv, t, 190.0, 1.15, 0.035, 1.4 * 190.0 / res.x, slant, 1.0) * 0.45;
    far = far + rain_layer(uv, t, 130.0, 0.90, 0.055, 1.7 * 130.0 / res.x, slant * 0.85, 2.0) * 0.34;

    // A drizzle veil between the discrete drops, stretched along the fall so it
    // reads as depth of water in the air rather than as noise.
    let veil_n = s_noise(vec2<f32>(
        (uv.x + uv.y * slant) * 110.0 * aspect.x,
        uv.y * 7.0 - t * 2.4,
    ));
    far = far + smoothstep(0.58, 1.0, veil_n) * 0.22;

    col = col + vec3<f32>(0.40, 0.48, 0.62) * far * rain * rain_lit * 0.55;

    // ------------------------------------------------------------------ bolt
    let bolt_life = 0.17;
    if (age >= 0.0 && age < bolt_life && bolt > 0.0) {
        let env = 1.0 - age / bolt_life;
        let sy = max(strike_pt.y, 0.05);
        let a = clamp(uv.y / sy, 0.0, 1.0);
        let below = step(uv.y, strike_pt.y);

        let jag = (s_fbm(vec2<f32>(uv.y * 13.0 + seed * 23.0, 0.5), JAG_OCT) - 0.5) * 0.060
                + (s_fbm(vec2<f32>(uv.y * 58.0 + seed * 7.0, 3.5), JAG_OCT) - 0.5) * 0.015;
        let path = mix(bolt_x, strike_pt.x, a * a * 0.85 + a * 0.15) + jag * (1.0 - a * 0.55);
        let dx = abs(uv.x - path) * aspect.x;
        let core = (1.0 - smoothstep(0.0006, 0.0022, dx)) * below;
        let glow = (0.0030 / (0.0030 + dx * dx * 110.0)) * below;
        col = col + (vec3<f32>(0.80, 0.88, 1.05) * core * 3.4
                   + vec3<f32>(0.32, 0.46, 0.88) * glow * 1.30) * bolt * env;

        // A fork, peeling off partway down and dying before the ground.
        let fy = sy * (0.34 + 0.28 * q_hash(seed * 17.0));
        if (uv.y > fy) {
            let fa = clamp((uv.y - fy) / max(sy - fy, 0.05), 0.0, 1.0);
            let fend = 0.45 + 0.40 * q_hash(seed * 19.0);
            let lean = (q_hash(seed * 23.0) - 0.5) * 0.34;
            let fjag = (s_fbm(vec2<f32>(uv.y * 22.0 + seed * 41.0, 8.0), JAG_OCT) - 0.5) * 0.040;
            let fpath = mix(bolt_x, strike_pt.x, fy / sy) + lean * fa + fjag;
            let fdx = abs(uv.x - fpath) * aspect.x;
            let alive = step(fa, fend) * (1.0 - smoothstep(fend * 0.6, fend, fa));
            let fcore = (1.0 - smoothstep(0.0004, 0.0016, fdx)) * alive;
            let fglow = (0.0018 / (0.0018 + fdx * fdx * 160.0)) * alive;
            col = col + (vec3<f32>(0.74, 0.83, 1.02) * fcore * 2.2
                       + vec3<f32>(0.28, 0.40, 0.82) * fglow * 0.85) * bolt * env;
        }
    }

    // --------------------------------------------------------------- the band
    // Back to front, exactly the engine's own order — index 0 is furthest back.
    var covered = 0.0;   // how much of this pixel a client window ended up owning
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let sc = windows.srcs[i];
        let fi = f32(i);

        // Panels are compositor UI, not client windows: no shake, no rain, no
        // tear. They get the plain blit the engine would have done.
        if (windows.attrs[i].x > 0.5) {
            let local = (uv - r.xy) / max(r.zw, vec2<f32>(0.0001));
            if (local.x < 0.0 || local.x > 1.0 || local.y < 0.0 || local.y > 1.0) {
                continue;
            }
            let ppx = textureSampleLevel(win_tex[i], samp, sc.xy + local * sc.zw, 0.0);
            let pa = clamp(ppx.a, 0.0, 1.0) * windows.attrs[i].y;
            col = col * (1.0 - pa) + ppx.rgb * windows.attrs[i].y;
            continue;
        }

        let off = quake_offset(shake, period, t, fi)
                + quake_shape(forced_amt, age, seed, fi);
        let local = (uv - off - r.xy) / max(r.zw, vec2<f32>(0.0001));
        if (local.x < 0.0 || local.x > 1.0 || local.y < 0.0 || local.y > 1.0) {
            continue;
        }

        let wpx = max(r.zw * res, vec2<f32>(8.0, 8.0));

        // ------------------------------------------------- rain on the glass
        // The near rain does not pass through a window — it lands on it. What
        // it becomes here is runnels sliding down the window's own surface
        // (same rain function, evaluated in the window's frame) plus beads
        // clinging to it, and the film drags the content underneath.
        var wet = 0.0;
        var wet_off = vec2<f32>(0.0, 0.0);
        if (rainwin > 0.0 && rain > 0.0) {
            // Runnels, in the window's own frame: water on glass runs far
            // slower than water in the air, and in fatter threads.
            let run = rain_layer(local, t, 20.0, 0.30, 0.22, 0.30, 0.0, fi + 3.0);
            let bn = s_noise(vec2<f32>(
                local.x * wpx.x * 0.045,
                local.y * wpx.y * 0.045 - t * 0.35 + fi * 11.0,
            ));
            let bead = smoothstep(0.66, 0.93, bn);
            wet = clamp(run * 0.85 + bead * 0.55, 0.0, 1.0) * rain * rainwin;
            wet_off = vec2<f32>(0.0, wet * 2.4) / wpx;
        }

        // ONE sample, taken at the displaced coordinate — the water refraction
        // costs nothing extra because it moves this read rather than adding one.
        let px = textureSampleLevel(win_tex[i], samp, sc.xy + (local + wet_off) * sc.zw, 0.0);
        let cov = clamp(px.a, 0.0, 1.0);
        var rgb = px.rgb;
        var opacity = windows.attrs[i].y;
        var add = vec3<f32>(0.0, 0.0, 0.0);

        // ------------------------------------------------------------- tear
        if (has_hit && i == hit_i && tearlife > 0.0 && age >= 0.0 && age < tearlife) {
            let p = local * wpx;
            let o = imp_local * wpx;
            let reach = max(tearsize, 0.001) * 0.55 * max(wpx.x, wpx.y);
            let cd = crack_dist(p, o, seed, reach);

            let ta = age / tearlife;
            let open = smoothstep(0.0, 0.05, ta) * (1.0 - smoothstep(0.70, 1.0, ta));
            let half_w = (0.9 + 3.2 * open) * (0.55 + 1.45 * tearsize);

            let strain = exp(-max(cd - half_w, 0.0) / (16.0 + 46.0 * tearsize))
                       * (0.55 + 0.45 * s_noise(p * 0.09 + seed)) * open;

            if (strain > 0.02) {
                let sh = strain * (1.4 + 3.4 * tearsize) / wpx.x;
                let base = sc.xy + (local + wet_off) * sc.zw;
                let cr = textureSampleLevel(win_tex[i], samp, base + vec2<f32>(sh * sc.z, 0.0), 0.0).r;
                let cb = textureSampleLevel(win_tex[i], samp, base - vec2<f32>(sh * sc.z, 0.0), 0.0).b;
                rgb = vec3<f32>(cr, rgb.g, cb);
            }
            let lum = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
            rgb = mix(rgb, vec3<f32>(lum) * 0.72, clamp(strain * 0.75, 0.0, 1.0));

            // The rip itself: alpha punched out, so the storm shows through.
            let rip = (1.0 - smoothstep(half_w, half_w + 1.3, cd)) * open;
            opacity = opacity * (1.0 - rip);

            let heat = 1.0 - smoothstep(0.0, 0.30, ta);
            let ember = 1.0 - smoothstep(0.20, 1.0, ta);
            let band = exp(-max(cd - half_w, 0.0) / (2.2 + 7.0 * open));
            let gcol = mix(vec3<f32>(1.00, 0.46, 0.16), vec3<f32>(0.88, 0.94, 1.25), heat);
            add = add + gcol * band * open * tearglow * (0.30 * ember + 2.40 * heat);
        }

        // Wet glass catches the light: the film is what the flash glints off.
        rgb = rgb + vec3<f32>(0.30, 0.38, 0.52) * wet * (0.20 + 2.40 * fl) * cov;

        // Splash line where the rain breaks on the window's top edge.
        if (rainwin > 0.0 && rain > 0.0) {
            let lip = 1.0 - smoothstep(0.0, 5.0 / max(wpx.y, 1.0), local.y);
            let spl = rain_layer(vec2<f32>(local.x, 0.35), t, 34.0, 1.20, 0.50, 0.40, 0.0, fi + 9.0);
            rgb = rgb + vec3<f32>(0.46, 0.54, 0.68) * lip * spl * rain * rainwin
                      * (0.35 + 2.0 * fl) * cov;
        }

        // Lightning falling on the window, strongest nearest the strike.
        let dstr = length((uv - strike_pt) * aspect);
        let lit = fl * lightpick * (0.20 + 0.90 / (1.0 + dstr * dstr * 16.0));
        rgb = rgb + vec3<f32>(0.48, 0.58, 0.86) * lit * cov;

        let a = cov * opacity;
        col = col * (1.0 - a) + rgb * opacity;
        col = col + add;
        covered = max(covered, a);
    }

    // ------------------------------------------------------------ foreground
    // Near rain falls in FRONT of everything — but it is thinned over windows,
    // because those streaks already became runnels on the glass above.
    // Close rain: fewer threads, much faster, longer streaks. Fast AND long is
    // motion blur; slow and long is a meteor.
    var near = rain_layer(uv, t, 80.0, 2.60, 0.115, 2.0 * 80.0 / res.x, slant * 1.20, 5.0) * 0.55;
    near = near + rain_layer(uv, t, 46.0, 3.60, 0.180, 2.6 * 46.0 / res.x, slant * 1.40, 6.0) * 0.40;
    col = col + vec3<f32>(0.52, 0.60, 0.76) * near * rain * rainfront * rain_lit * 0.45
              * mix(1.0, 0.25, covered);

    // The impact flare sits on top of the torn window, not behind it.
    if (has_hit && age >= 0.0 && age < 0.5) {
        let e = 1.0 - age / 0.5;
        let dstr = length((uv - strike_pt) * aspect);
        col = col + vec3<f32>(0.82, 0.90, 1.10)
                  * (0.010 / (0.010 + dstr * dstr * 45.0)) * e * e * e * bolt * 2.2;
    }

    // Whole-frame lift, so the flash reads as light in the air.
    col = col + vec3<f32>(0.09, 0.12, 0.19) * fl * 0.35;

    // No sRGB encode: passes/smear.wgsl writes the swapchain.
    return vec4<f32>(max(col, vec3<f32>(0.0)), 1.0);
}
