// Built-in background: "Harbor Beacon" — a mood piece. A night bay seen from the
// shore, everything gently out of focus: a lighthouse on a dark headland sweeps
// its beam through the sea mist and flares once per turn, channel buoys blink
// their red/green rhythms out on the water, boats drift across two lanes trailing
// mast- and cabin-light reflections, and a moon glade shimmers over the swell.
// Same Push / `@prop` contract as the rest of the built-in set.
//
// Design notes (screen-anchored like rain_glass — the bay is a vista, so it lives
// in zoom-independent screen space; a whisper of pan gives parallax):
//   * Every light is a defocus disc (`bdisc`): radius grows and energy dims with
//     the Defocus knob, so the whole harbor reads as dreamy night bokeh.
//   * The beacon is three coupled signals: two fog-gated spokes rotating in
//     screen space around the lantern, a lantern flare that peaks once per
//     revolution (`pow(cos)` pulse) as the beam sweeps past the viewer, and a
//     shimmering reflection column on the water that surges with the flare.
//   * Buoys keep nautical time: a short flash then a long dark (Fl 2.5s green /
//     Fl 4s red by hash), bobbing on the swell, each blinking its own smeared
//     reflection on and off.
//   * Boats are hash-present cells in two lanes (far slow, near faster and the
//     other way): a soft hull-and-cabin silhouette, white masthead light, warm
//     cabin glow, and a direction-coded red/green nav light at the bow.
//   * Sea mist is a noise band hugging the horizon, veiling the scene — but the
//     beam and flare draw *after* it, because mist is what makes them visible.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop beacon_speed float default=1.0 min=0.0 max=3.0 label="Beacon speed" group="Beacon"
// @prop beacon_glow float default=1.0 min=0.0 max=2.0 label="Beacon glow" group="Beacon"
// @prop boat_traffic float default=1.0 min=0.0 max=2.0 label="Boat traffic" group="Bay"
// @prop buoy_density float default=1.0 min=0.0 max=2.0 label="Buoy markers" group="Bay"
// @prop shore_lights float default=1.0 min=0.0 max=2.0 label="Shore lights" group="Bay"
// @prop warmth float default=0.55 min=0.0 max=1.0 label="Cool → warm lights" group="Bay"
// @prop fog float default=0.55 min=0.0 max=1.0 label="Sea mist" group="Mood"
// @prop defocus float default=0.85 min=0.0 max=1.5 label="Defocus" group="Mood"
// @prop moonlight float default=0.7 min=0.0 max=1.5 label="Moonlight" group="Mood"
// @prop swell float default=1.0 min=0.0 max=2.0 label="Swell" group="Water"
// @prop vignette float default=0.35 min=0.0 max=1.0 label="Vignette amount" group="Frame"
// @prop vignette_radius float default=1.15 min=0.5 max=2.0 label="Vignette radius" group="Frame"
// @prop vignette_softness float default=0.7 min=0.05 max=2.0 label="Vignette softness" group="Frame"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 4>,
};
var<immediate> pc: Push;

struct VsOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    let uv = vec2<f32>(f32((vid << 1u) & 2u), f32(vid & 2u));
    var o: VsOut;
    o.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

// `hash` is a pure function of two fract lanes: the vec3's z lane always
// duplicates x, since both are `fract(p.x * 0.1031)`. Splitting the lane setup
// out lets `noise` compute its four corner lanes once and share them across all
// four taps. Every operand and operation order below is unchanged, so the output
// is bit-identical to the vec3 form this replaced.
fn hash_lane(x: f32, y: f32) -> f32 {
    let p3 = vec3<f32>(x, y, x);
    let d = dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + vec3<f32>(33.33));
    return fract(((x + d) + (y + d)) * (x + d));
}
fn hash(p: vec2<f32>) -> f32 {
    let q = fract(p * 0.1031);
    return hash_lane(q.x, q.y);
}
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    var f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    // One vec4 mul+fract for all four corner lanes, each still computed exactly as
    // the four separate `hash` calls did: fract(i.x*K), fract((i.x+1)*K),
    // fract(i.y*K), fract((i.y+1)*K).
    let q = fract(vec4<f32>(i.x, i.x + 1.0, i.y, i.y + 1.0) * 0.1031);
    return mix(
        mix(hash_lane(q.x, q.z), hash_lane(q.y, q.z), f.x),
        mix(hash_lane(q.x, q.w), hash_lane(q.y, q.w), f.x),
        f.y,
    );
}

// A defocus disc: radius grows with `blur`, edge softens, and total energy stays
// roughly constant (defocused lights get bigger AND dimmer, like real bokeh).
fn bdisc(d: f32, r: f32, blur: f32) -> f32 {
    let rr = r * (1.0 + 2.4 * blur);
    let e = 0.30 + 0.55 * blur;
    // Partial energy conservation: fully physical (r²/rr²) makes misted lights
    // vanish; half-way keeps the dreamy big-bokeh look bright enough to matter.
    let atten = mix(1.0, (r * r) / (rr * rr), 0.55);
    return smoothstep(rr, rr * (1.0 - e), d) * atten;
}

// Warm sodium/amber vs cool white/blue harbor light, chosen per-lamp by hash.
fn lightcol(h: f32, warmth: f32) -> vec3<f32> {
    let warm = vec3<f32>(1.0, 0.58, 0.22);
    let cool = vec3<f32>(0.55, 0.72, 1.0);
    return mix(cool, warm, smoothstep(1.0 - warmth, 1.0, fract(h * 41.0)));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let frag = in.pos.xy;
    let res = pc.res_zoom_time.xy;
    let time = pc.res_zoom_time.w;
    let pan_in = pc.pan_flow.xy;
    let lock_amount = pc.lock_alpha.x;
    let alpha = pc.lock_alpha.y;

    let beacon_speed = pc.params[0].x;
    let beacon_glow = pc.params[0].y;
    let traffic = pc.params[0].z;
    let buoys = pc.params[0].w;
    let shore = pc.params[1].x;
    let warmth = clamp(pc.params[1].y, 0.0, 1.0);
    let fog = clamp(pc.params[1].z, 0.0, 1.0);
    let blur = clamp(pc.params[1].w, 0.0, 1.5);
    let moonlight = pc.params[2].x;
    let swell = pc.params[2].y;
    let vignette = pc.params[2].z;
    let vig_radius = pc.params[2].w;
    let vig_softness = pc.params[3].x;

    let screen_uv = (frag - 0.5 * res) / res.y;
    // The bay is screen space; a whisper of pan gives the vista a little parallax.
    // Fragment y grows downward, so flip into up-positive space — everything below
    // is authored with +y = up (sky above the horizon, water beneath it).
    let pan = vec2<f32>(pan_in.x, -pan_in.y);
    let uv = vec2<f32>(screen_uv.x + pan.x * 0.00015, -screen_uv.y + pan.y * 0.00015);

    let h0 = -0.05;
    let above = uv.y - h0;
    let reflmask = smoothstep(0.01, -0.03, above);
    // Shared water shimmer: every reflection smear rides this, at the swell's pace.
    let shim = 0.7 + 0.6 * noise(vec2<f32>(uv.x * 55.0, uv.y * 16.0 - time * (0.25 + 0.35 * swell)));

    // ---- Sky and water base -------------------------------------------------
    let sky = mix(vec3<f32>(0.012, 0.016, 0.030), vec3<f32>(0.034, 0.043, 0.056),
                  exp(-max(above, 0.0) * 2.6));
    let wave = noise(vec2<f32>(uv.x * 6.0 - time * 0.06 * swell, uv.y * 30.0 + time * 0.10 * swell));
    let sheen = 0.75 + 0.5 * noise(vec2<f32>(uv.x * 9.0, uv.y * 42.0 - time * 0.35 * swell));
    let water = vec3<f32>(0.010, 0.016, 0.022) * sheen * (0.8 + 0.4 * wave);
    var col = mix(water, sky, smoothstep(-0.006, 0.006, above));

    // ---- Stars: sparse, dim, twinkling; the mist swallows them --------------
    let sp = uv * 16.0;
    let sip = floor(sp);
    let sh = hash(sip + 71.0);
    if (above > 0.05 && sh > 0.80) {
        let sj = vec2<f32>(hash(sip + 3.0), hash(sip + 9.0)) - 0.5;
        let sd = length(fract(sp) - 0.5 - sj * 0.6) / 16.0;
        let twinkle = 0.7 + 0.3 * sin(time * (1.0 + sh * 3.0) + sh * 40.0);
        col = col + vec3<f32>(0.60, 0.70, 0.90)
                  * bdisc(sd, 0.0016 + 0.0020 * fract(sh * 7.0), blur)
                  * 0.35 * twinkle * smoothstep(0.05, 0.15, above) * (1.0 - 0.6 * fog);
    }

    // ---- Moon and its glade on the swell ------------------------------------
    let mp = vec2<f32>(0.36, 0.30);
    let md = length(uv - mp);
    let mc = vec3<f32>(0.72, 0.80, 0.94);
    col = col + mc * (bdisc(md, 0.030, blur * 0.7) * 0.9 + exp(-md * 6.5) * 0.10) * moonlight;
    // Glade: a broad column that widens as the mist thickens.
    let glade = exp(-abs(uv.x - mp.x) * (7.0 - 3.0 * fog)) * exp(-(h0 - uv.y) * 2.2) * reflmask;
    col = col + mc * glade * shim * 0.06 * moonlight;

    // ---- Far shore: a string of bokeh lights hugging the horizon ------------
    let glowc = mix(vec3<f32>(0.20, 0.28, 0.42), vec3<f32>(0.44, 0.28, 0.15), warmth);
    col = col + glowc * 0.07 * exp(-abs(above) * 6.0) * (0.3 + 0.7 * shore);
    let ssc = 3.4;
    let scell0 = floor(uv.x * ssc);
    for (var i = -1; i <= 1; i = i + 1) {
        let cell = scell0 + f32(i);
        let h = hash(vec2<f32>(cell, 5.3));
        if (h > 0.30) {
            let cx = (cell + 0.5 + (h - 0.5) * 0.7) / ssc;
            let ly = h0 + 0.006 + fract(h * 13.0) * 0.018;
            let d = length(uv - vec2<f32>(cx, ly));
            let c = lightcol(h, warmth);
            col = col + c * (bdisc(d, 0.0035 + 0.006 * fract(h * 29.0), blur) * 1.6
                             + exp(-d * 20.0) * 0.05) * shore;
            let rf = exp(-abs(uv.x - cx) * 30.0) * exp(-(h0 - uv.y) * 4.2) * shim * reflmask;
            col = col + c * rf * 0.10 * shore;
        }
    }

    // ---- Boats: two lanes, hash-present cells like harbor traffic -----------
    for (var lane = 0; lane < 2; lane = lane + 1) {
        let near = lane == 1;
        let laneY = select(h0 - 0.045, h0 - 0.13, near);
        let sc = select(1.6, 0.95, near);
        let dir = select(0.012, -0.018, near);
        let hl = select(0.030, 0.062, near); // hull half-length
        let hh = select(0.008, 0.016, near); // hull half-height
        let mh = select(0.030, 0.058, near); // masthead height
        let cp = uv.x * sc + time * dir;
        let cip = floor(cp);
        for (var i = -1; i <= 1; i = i + 1) {
            let cell = cip + f32(i);
            let h = hash(vec2<f32>(cell, 33.0 + f32(lane) * 7.0));
            if (h > 1.0 - 0.4 * traffic) {
                let cx = (cell + 0.5 + (h - 0.5) * 0.30 - time * dir) / sc;
                let bob = sin(time * (0.5 + 0.4 * h) + h * 40.0) * 0.004 * swell
                        * select(0.7, 1.3, near);
                let yb = laneY + bob;
                // Soft silhouette: hull ellipse plus a cabin bump amidships.
                let hd = (uv - vec2<f32>(cx, yb)) / vec2<f32>(hl, hh);
                let cd = (uv - vec2<f32>(cx - hl * 0.15, yb + hh * 1.2))
                       / vec2<f32>(hl * 0.45, hh * 1.4);
                let sil = clamp(smoothstep(1.0, 0.55, length(hd))
                              + smoothstep(1.0, 0.50, length(cd)), 0.0, 1.0);
                col = mix(col, vec3<f32>(0.010, 0.012, 0.016), sil * 0.85);
                // Lights: white masthead, warm cabin, direction-coded bow nav.
                let mastc = vec3<f32>(0.95, 0.97, 1.00);
                let cabc = vec3<f32>(1.00, 0.72, 0.38);
                col = col + mastc * bdisc(length(uv - vec2<f32>(cx, yb + mh)),
                                          select(0.0035, 0.0050, near), blur) * 1.6;
                col = col + cabc * bdisc(length(uv - vec2<f32>(cx - hl * 0.15, yb + hh * 1.1)),
                                         select(0.0040, 0.0060, near), blur) * 0.8;
                let navc = select(vec3<f32>(0.10, 1.00, 0.35), vec3<f32>(1.00, 0.12, 0.10),
                                  dir < 0.0);
                let bowx = cx + select(hl, -hl, dir < 0.0) * 0.85;
                col = col + navc * bdisc(length(uv - vec2<f32>(bowx, yb + hh * 0.8)),
                                         0.0022, blur) * 0.9;
                // Smeared reflection under the lights, fading down from the hull.
                let below = smoothstep(0.005, -0.015, uv.y - yb);
                let rfb = exp(-abs(uv.x - cx) * 30.0) * exp(min(uv.y - yb, 0.0) * 6.0)
                        * shim * below;
                col = col + (mastc * 0.5 + cabc * 0.5) * rfb * 0.10;
            }
        }
    }

    // ---- Buoys: blinking channel markers keeping nautical time --------------
    for (var row = 0; row < 2; row = row + 1) {
        let near = row == 1;
        let by = select(h0 - 0.075, h0 - 0.175, near);
        let bsc = select(1.9, 1.2, near);
        let boff = f32(row) * 13.7;
        let cell0 = floor(uv.x * bsc + boff);
        for (var i = -1; i <= 1; i = i + 1) {
            let cell = cell0 + f32(i);
            let h = hash(vec2<f32>(cell, 57.0 + f32(row) * 11.0));
            if (h > 1.0 - 0.35 * buoys) {
                let h2 = hash(vec2<f32>(cell, 71.0 + f32(row)));
                let bx = (cell + 0.5 + (h - 0.5) * 0.6 - boff) / bsc;
                let y = by + sin(time * (0.7 + 0.6 * h2) + h * 50.0) * 0.005 * swell;
                // Fl 4s red / Fl 2.5s green: a short flash, then a long dark.
                let red = h2 > 0.5;
                let period = select(2.5, 4.0, red);
                let fr = fract(time / period + h * 7.0);
                let blink = smoothstep(0.0, 0.05, fr) * (1.0 - smoothstep(0.10, 0.18, fr));
                let c = select(vec3<f32>(0.15, 1.00, 0.35), vec3<f32>(1.00, 0.10, 0.08), red);
                let d = length(uv - vec2<f32>(bx, y));
                col = col + c * (bdisc(d, select(0.0030, 0.0045, near), blur) * 2.2
                                 + exp(-d * 24.0) * 0.10) * blink;
                // The barely-there dark float under the lamp.
                let bodym = smoothstep(1.0, 0.5,
                    length((uv - vec2<f32>(bx, y - 0.006)) / vec2<f32>(0.006, 0.008)));
                col = mix(col, vec3<f32>(0.012, 0.014, 0.018), bodym * 0.5);
                let rfb = exp(-abs(uv.x - bx) * 40.0) * exp(min(uv.y - y, 0.0) * 8.0)
                        * shim * smoothstep(0.0, -0.01, uv.y - y);
                col = col + c * rfb * 0.16 * blink;
            }
        }
    }

    // ---- The headland and lighthouse silhouette -----------------------------
    let lx = -0.60;
    let ltop = h0 + 0.185;
    // A dark landmass rising toward the left edge; the tower stands on it.
    let landh = 0.012 + 0.030 * smoothstep(-0.45, -0.85, uv.x)
              + noise(vec2<f32>(uv.x * 3.0, 7.7)) * 0.008;
    let landm = smoothstep(0.004, -0.004, uv.y - (h0 + landh)) * smoothstep(-0.30, -0.55, uv.x)
              * smoothstep(h0 - 0.03, h0 - 0.005, uv.y);
    col = mix(col, vec3<f32>(0.010, 0.012, 0.016), landm * 0.8);
    let rockm = smoothstep(1.0, 0.45,
        length((uv - vec2<f32>(lx, h0 - 0.012)) / vec2<f32>(0.085, 0.020)));
    col = mix(col, vec3<f32>(0.008, 0.010, 0.013), rockm * 0.85);
    // Tower: tapered shaft, a wider gallery, a narrow lantern room on top.
    let t = clamp((uv.y - h0) / (ltop - h0), 0.0, 1.0);
    var w = mix(0.021, 0.012, clamp(t / 0.8, 0.0, 1.0));
    w = mix(w, 0.019, smoothstep(0.78, 0.82, t) * (1.0 - smoothstep(0.90, 0.93, t)));
    w = mix(w, 0.009, smoothstep(0.90, 0.94, t));
    let edge = 0.0035 + 0.004 * blur;
    let iny = smoothstep(h0 - 0.02, h0 - 0.005, uv.y) * (1.0 - smoothstep(ltop, ltop + edge, uv.y));
    let sil = smoothstep(w + edge, w - edge * 0.5, abs(uv.x - lx)) * iny;
    col = mix(col, vec3<f32>(0.010, 0.011, 0.015), sil * 0.88);

    // ---- Sea mist: a noise band hugging the horizon (veils everything so far)
    let mist = noise(vec2<f32>(uv.x * 2.2 + time * 0.015, uv.y * 7.0)) * 0.6
             + noise(vec2<f32>(uv.x * 5.0 - time * 0.010, uv.y * 13.0)) * 0.4;
    let fogm = clamp(fog * exp(-abs(above) * 3.2) * (0.45 + 0.75 * mist), 0.0, 1.0);
    col = mix(col, vec3<f32>(0.045, 0.055, 0.070), fogm * 0.55);

    // ---- The beacon: drawn after the mist, because mist is what reveals it ---
    let lp = vec2<f32>(lx, h0 + 0.168);
    let th = time * beacon_speed * 0.65;
    let cs = cos(th);
    let sn = sin(th);
    let u2 = uv - lp;
    let rx = cs * u2.x + sn * u2.y;
    let ry = -sn * u2.x + cs * u2.y;
    // Two spokes rotating around the lantern: a cone that widens with distance,
    // visible only as far as the mist scatters it; the back spoke runs dimmer.
    let beamvis = (0.30 + 0.70 * fog) * beacon_glow;
    let wc = 0.010 + 0.14 * abs(rx);
    let lobe = exp(-(ry * ry) / (wc * wc));
    let fwd = lobe * smoothstep(0.0, 0.04, rx) * exp(-max(rx, 0.0) * 2.2);
    let bwd = lobe * smoothstep(0.0, 0.04, -rx) * exp(-max(-rx, 0.0) * 2.2);
    let lampc = vec3<f32>(1.00, 0.93, 0.74);
    col = col + lampc * (fwd + bwd * 0.35) * 0.28 * beamvis;
    // Lantern flare: once per revolution the beam sweeps past and the light blooms.
    let flash = pow(max(cs, 0.0), 24.0);
    let dl = length(u2);
    col = col + lampc * (bdisc(dl, 0.011, blur) * (0.7 + 3.0 * flash)
                         + exp(-dl * 9.0) * (0.06 + 0.45 * flash)) * beacon_glow;
    // Its column on the water surges with the flare.
    let rfl = exp(-abs(uv.x - lx) * 22.0) * exp(-(h0 - uv.y) * 3.0) * shim * reflmask;
    col = col + lampc * rfl * (0.05 + 0.40 * flash) * beacon_glow;

    // Lock-screen ease: dim the bay and let the night settle.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.5 + vec3<f32>(0.004, 0.006, 0.010), l);

    // Guarded because the default is off: the length() and the smoothstep would
    // otherwise run on every pixel only to be multiplied by 1.0. The guard also
    // makes a 0 softness safe — that smoothstep divides by zero, and
    // mix(1.0, NaN, 0.0) is NaN, not 1.0.
    let vig_amount = clamp(vignette, 0.0, 1.0);
    if (vig_amount > 0.0) {
        let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
        col = col * mix(1.0, vig, vig_amount);
    }
    // Per-world sRGB flag (push lock_alpha.z): gamma-encode for the brighter,
    // preview-matching look on a non-sRGB scanout buffer. Off = raw values.
    var outc = col;
    // An `if`, not `select` — `select` evaluates both arms, so these three
    // pow()s ran on every pixel even with the flag off (the default). The branch is
    // on a push constant, so it is uniform across the whole draw.
    if (pc.lock_alpha.z > 0.5) {
        outc = pow(max(outc, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(outc, 1.0) * (alpha * 0.75);
}
