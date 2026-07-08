// Built-in background: "Rain on Glass" — a mood piece. You're looking through a wet
// window at night onto a defocused city street: window bokeh above a glowing
// horizon, street lamps, car lights sliding past with wet-asphalt reflection
// smears — and rain running down the pane in front. Same Push / `@prop` contract
// as the rest of the built-in set.
//
// Design notes (screen-anchored — the glass is the display surface, so the rain
// lives in zoom-independent screen space; the street only drifts a touch with the
// canvas pan for a parallax cue):
//   * The street is a single procedural scene function taking a per-pixel `blur`:
//     every light is a defocus disc whose radius grows and energy dims with blur.
//     Fogged glass sees the blurry version; inside a droplet the same scene is
//     re-evaluated nearly sharp, so drops read as tiny lenses full of city light.
//   * Rain is three things, like on a real pane: running drops that surge and
//     stall (a steady grid scroll plus nested-sine jitter — never a constant
//     glide) while swinging along fixed wavy rivulet paths; bead trails pinned to
//     the glass above each head, shrinking and fading; and a slow population of
//     clinging static droplets that swell and evaporate.
//   * Drops refract: the street is sampled through the drop's local offset
//     (negated — droplets invert), with a rim shade and an up-left glint.
//   * Glass fog dims and flattens the street; drop trails wipe it locally clean.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop rain_speed float default=1.0 min=0.0 max=4.0 label="Rain speed" group="Rain"
// @prop droplet_density float default=1.0 min=0.0 max=2.0 label="Droplet density" group="Rain"
// @prop bokeh float default=1.0 min=0.0 max=2.0 label="Street lights" group="Street"
// @prop warmth float default=0.5 min=0.0 max=1.0 label="Cool → warm lights" group="Street"
// @prop streak_length float default=1.0 min=0.2 max=2.0 label="Streak length" group="Rain"
// @prop vignette float default=0.35 min=0.0 max=1.0 label="Vignette amount" group="Frame"
// @prop vignette_radius float default=1.15 min=0.5 max=2.0 label="Vignette radius" group="Frame"
// @prop vignette_softness float default=0.7 min=0.05 max=2.0 label="Vignette softness" group="Frame"
// @prop glass_fog float default=0.55 min=0.0 max=1.0 label="Glass fog" group="Street"

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

fn hash(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 = p3 + dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + vec3<f32>(33.33));
    return fract((p3.x + p3.y) * p3.z);
}
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    var f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash(i), hash(i + vec2<f32>(1.0, 0.0)), f.x),
        mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), f.x),
        f.y,
    );
}

// A defocus disc: radius grows with `blur`, edge softens, and total energy stays
// roughly constant (defocused lights get bigger AND dimmer, like real bokeh).
fn bdisc(d: f32, r: f32, blur: f32) -> f32 {
    let rr = r * (1.0 + 2.4 * blur);
    let e = 0.30 + 0.55 * blur;
    // Partial energy conservation: fully physical (r²/rr²) makes fogged lights
    // vanish; half-way keeps the dreamy big-bokeh look bright enough to matter.
    let atten = mix(1.0, (r * r) / (rr * rr), 0.55);
    return smoothstep(rr, rr * (1.0 - e), d) * atten;
}

// Warm sodium/amber vs cool white/blue city light, chosen per-lamp by hash.
fn lightcol(h: f32, warmth: f32) -> vec3<f32> {
    let warm = vec3<f32>(1.0, 0.58, 0.22);
    let cool = vec3<f32>(0.55, 0.72, 1.0);
    return mix(cool, warm, smoothstep(1.0 - warmth, 1.0, fract(h * 41.0)));
}

// The street behind the glass, defocused by `blur` (0 = sharp, ~1 = fogged).
// Horizon + city glow, window bokeh above, street lamps, two lanes of moving car
// lights, and vertical reflection smears on the wet asphalt below the horizon.
fn street(uv: vec2<f32>, blur: f32, warmth: f32, amt: f32, time: f32) -> vec3<f32> {
    let h0 = -0.08;
    let above = uv.y - h0;

    // Night sky fading into the city haze; wet asphalt with a faint shimmer.
    let sky = mix(vec3<f32>(0.030, 0.027, 0.031), vec3<f32>(0.010, 0.013, 0.024),
                  clamp(above * 3.2, 0.0, 1.0));
    let sheen = 0.8 + 0.4 * noise(vec2<f32>(uv.x * 7.0, uv.y * 30.0 - time * 0.15));
    let road = vec3<f32>(0.016, 0.017, 0.021) * sheen;
    var col = mix(road, sky, smoothstep(-0.008, 0.008, above));

    // City glow hugging the horizon (mirrored faintly onto the road).
    let glowc = mix(vec3<f32>(0.24, 0.30, 0.44), vec3<f32>(0.46, 0.30, 0.16), warmth);
    col = col + glowc * 0.09 * exp(-abs(above) * 6.5) * (0.4 + 0.6 * amt);

    // Far depth: a few huge, ultra-soft discs drifting behind everything.
    let gp = uv * 1.5 + vec2<f32>(time * 0.006, 0.0);
    let gip = floor(gp);
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let g = vec2<f32>(f32(x), f32(y));
            let h = hash(gip + g + 51.0);
            if (h > 0.55) {
                let j = vec2<f32>(hash(gip + g + 3.0), hash(gip + g + 8.0)) - 0.5;
                let d = length(fract(gp) - g - 0.5 - j * 0.7);
                col = col + lightcol(h, warmth) * smoothstep(0.75, 0.0, d) * 0.06 * amt;
            }
        }
    }

    // Window bokeh: rows of small lit windows stacked above the horizon.
    let wscale = vec2<f32>(4.5, 8.0);
    let wp = vec2<f32>(uv.x, above) * wscale;
    let wip = floor(wp);
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let g = vec2<f32>(f32(x), f32(y));
            let cell = wip + g;
            let h = hash(cell + 7.0);
            if (h > 0.52 && cell.y >= 0.0) {
                let j = vec2<f32>(hash(cell + 13.0), hash(cell + 29.0)) - 0.5;
                let cw = cell + 0.5 + j * 0.7;
                let cuv = vec2<f32>(cw.x / wscale.x, cw.y / wscale.y + h0);
                let r = 0.006 + 0.018 * fract(h * 23.0);
                // Higher floors thin out; a slow flicker keeps them alive.
                let fade = exp(-(cuv.y - h0) * 2.4);
                let flick = 0.85 + 0.15 * sin(time * (0.4 + h) + h * 30.0);
                col = col + lightcol(h, warmth) * bdisc(length(uv - cuv), r, blur)
                          * (0.9 + 1.1 * fract(h * 57.0)) * fade * flick * amt;
            }
        }
    }

    // Street lamps: a sparse warm row with misty halos and wet reflections.
    let lampc = mix(vec3<f32>(0.80, 0.88, 1.00), vec3<f32>(1.00, 0.62, 0.24),
                    clamp(warmth * 1.3, 0.0, 1.0));
    let lsc = 2.6;
    let lcell0 = floor(uv.x * lsc);
    let reflmask = smoothstep(0.01, -0.03, above);
    let shim = 0.7 + 0.6 * noise(vec2<f32>(uv.x * 55.0, uv.y * 16.0 - time * 0.6));
    for (var i = -1; i <= 1; i = i + 1) {
        let cell = lcell0 + f32(i);
        let h = hash(vec2<f32>(cell, 3.7));
        let cx = (cell + 0.5 + (h - 0.5) * 0.25) / lsc;
        let ly = h0 + 0.135 + (hash(vec2<f32>(cell, 9.1)) - 0.5) * 0.03;
        let d = length(uv - vec2<f32>(cx, ly));
        col = col + lampc * (bdisc(d, 0.026, blur) * 2.6 + exp(-d * 14.0) * 0.07) * amt;
        // Vertical smear on the asphalt under the lamp, shimmering with the rain.
        let rf = exp(-abs(uv.x - cx) * (26.0 - 12.0 * clamp(blur, 0.0, 1.0)))
               * exp(-(h0 - uv.y) * 3.0) * shim * reflmask;
        col = col + lampc * rf * 0.16 * amt;
    }

    // Two lanes of traffic: white headlight pairs one way, red taillights the
    // other, each dragging a reflection streak across the wet road.
    for (var lane = 0; lane < 2; lane = lane + 1) {
        let toward = lane == 0;
        let laneY = select(h0 + 0.030, h0 + 0.016, toward);
        let dir = select(-0.11, 0.15, toward);
        let cc = select(vec3<f32>(1.0, 0.15, 0.08), vec3<f32>(1.0, 0.93, 0.72), toward);
        let sc = select(1.4, 1.7, toward);
        let cp = uv.x * sc + time * dir;
        let cip = floor(cp);
        for (var i = -1; i <= 1; i = i + 1) {
            let cell = cip + f32(i);
            let h = hash(vec2<f32>(cell, 21.0 + f32(lane) * 9.0));
            if (h > 0.45) {
                let cx = (cell + 0.5 + (h - 0.5) * 0.4 - time * dir) / sc;
                let sep = select(0.009, 0.012, toward);
                let d1 = length(uv - vec2<f32>(cx - sep, laneY));
                let d2 = length(uv - vec2<f32>(cx + sep, laneY));
                let r = select(0.007, 0.009, toward);
                col = col + cc * (bdisc(d1, r, blur) + bdisc(d2, r, blur)) * 2.4 * amt;
                let rf = exp(-abs(uv.x - cx) * 34.0) * exp(-(h0 - uv.y) * 4.5) * shim * reflmask;
                col = col + cc * rf * 0.14 * amt;
            }
        }
    }
    return col;
}

// One layer of running drops. Tall cells; the grid scrolls down steadily while
// the head oscillates inside its cell through nested sines — the sum is the
// surge-stall-surge crawl of a real drop. The head swings along a fixed wavy
// rivulet (a function of *screen* y, so successive drops retrace the same wet
// path), and leaves a trail of shrinking beads pinned to the glass above it.
// Returns: xy = refraction offset (uv units), z = droplet mask, w = fog wipe.
fn drop_layer(uv: vec2<f32>, sc: vec2<f32>, rad: f32, time: f32, speed: f32,
              density: f32, streak: f32, seed: f32) -> vec4<f32> {
    var p = uv * sc;
    let scroll = time * speed * 0.55;
    p.y = p.y + scroll;
    let id = floor(p);
    let h1 = hash(id + seed);
    if (h1 > 0.15 + 0.55 * density) { return vec4<f32>(0.0); }
    let h2 = hash(id + seed + 17.0);
    let f = fract(p) - 0.5;

    // Path anchor + wiggle stay well inside the cell so the head never straddles
    // a neighbour boundary (|cx| + |wig| + head half-width < 0.5).
    let cx = (h2 - 0.5) * 0.4;
    let wig = 0.09 * sin(uv.y * 7.0 + h1 * 40.0) + 0.04 * sin(uv.y * 19.0 + h2 * 31.0);
    let dx = f.x - cx - wig;
    // Nested sines at the scroll's own rate: velocity swings ~0..2x, never negative
    // for long — the drop visibly sticks, then breaks free and slides.
    let tt = scroll + h1 * 61.0;
    let hy = 0.40 * sin(tt + sin(tt * 1.7 + sin(tt * 2.9) * 0.65) * 0.85);
    let dy = f.y - hy;

    // Head: a slightly tall lens.
    let duv = vec2<f32>(dx / sc.x, dy / sc.y);
    let m = smoothstep(rad, rad * 0.45, length(duv * vec2<f32>(1.0, 0.80)));
    var off = duv * m;
    var mask = m;

    // Bead trail: droplets left on the pane above the head, shrinking with
    // distance; pinned in screen space (they must not scroll with the grid).
    let bq = uv.y * sc.y * 2.5 + h1 * 7.0;
    let bf = fract(bq) - 0.5;
    let bh = hash(vec2<f32>(id.x + seed, floor(bq)));
    let up = f.y - hy;
    let fadeup = 1.0 - clamp(up / (0.9 * streak + 0.15), 0.0, 1.0);
    let bd = vec2<f32>(dx / sc.x, bf / (sc.y * 2.5));
    let br = rad * (0.25 + 0.35 * bh) * (0.35 + 0.65 * fadeup);
    let bm = smoothstep(br, br * 0.3, length(bd))
           * step(0.0, up) * fadeup * step(0.25, bh);
    off = off + bd * bm;
    mask = mask + bm * 0.8;

    // Fog wipe: the strip the drop just cleaned, fading as it re-fogs upward.
    // Windowed to zero at the cell borders — neighbouring cells know nothing of
    // this drop, so without the window the wipe would seam along the grid.
    let ew = smoothstep(0.50, 0.36, abs(f.x)) * smoothstep(0.50, 0.38, abs(f.y));
    let wipe = smoothstep(0.30, 0.04, abs(dx))
             * smoothstep(hy - 0.12, hy + 0.30, f.y)
             * (1.0 - clamp(up / (1.3 * streak + 0.2), 0.0, 1.0)) * ew;
    return vec4<f32>(off, mask, wipe);
}

// Static droplets clinging to the pane: swell in, sit, evaporate; refract a touch.
fn still_layer(uv: vec2<f32>, sc: f32, time: f32, density: f32, seed: f32) -> vec3<f32> {
    let p = uv * sc;
    let id = floor(p);
    let h1 = hash(id + seed);
    if (h1 > 0.03 + 0.20 * density) { return vec3<f32>(0.0); }
    let h2 = hash(id + seed + 5.0);
    let h3 = hash(id + seed + 11.0);
    let f = fract(p) - 0.5;
    let c = vec2<f32>(h2, h3) * 0.5 - 0.25;
    let lf = fract(h1 * 91.0 + time * 0.03 * (0.4 + h2));
    let env = smoothstep(0.0, 0.08, lf) * (1.0 - smoothstep(0.5, 1.0, lf));
    let r = (0.22 + 0.30 * h3) * env / sc;
    let duv = (f - c) / sc;
    let m = smoothstep(r, r * 0.4, length(duv));
    return vec3<f32>(duv * m, m);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let frag = in.pos.xy;
    let res = pc.res_zoom_time.xy;
    let time = pc.res_zoom_time.w;
    let pan_in = pc.pan_flow.xy;
    let lock_amount = pc.lock_alpha.x;
    let alpha = pc.lock_alpha.y;

    let rain_speed = pc.params[0].x;
    let droplet_density = pc.params[0].y;
    let bokeh_amt = pc.params[0].z;
    let warmth = clamp(pc.params[0].w, 0.0, 1.0);
    let streak = pc.params[1].x;
    let vignette = pc.params[1].y;
    let vig_radius = pc.params[1].z;
    let vig_softness = pc.params[1].w;
    let glass_fog = clamp(pc.params[2].x, 0.0, 1.0);

    let screen_uv = (frag - 0.5 * res) / res.y;
    // The pane is screen space; a whisper of pan gives the street a little parallax.
    let pan = vec2<f32>(pan_in.x, -pan_in.y);
    let guv = screen_uv + pan * 0.00015;

    // Rain on the pane: two running layers plus two static-droplet scales.
    let r0 = drop_layer(screen_uv, vec2<f32>(9.0, 2.1), 0.018, time, rain_speed,
                        droplet_density, streak, 0.0);
    let r1 = drop_layer(screen_uv, vec2<f32>(13.0, 3.0), 0.012, time, rain_speed * 1.25,
                        droplet_density, streak, 31.0);
    let s0 = still_layer(screen_uv, 24.0, time, droplet_density, 3.0);
    let s1 = still_layer(screen_uv, 44.0, time, droplet_density, 47.0);
    let off = r0.xy * 1.3 + r1.xy * 1.1 + s0.xy * 0.7 + s1.xy * 0.45;
    let mask = clamp(r0.z + r1.z + s0.z + s1.z, 0.0, 1.0);
    let wipe = clamp(r0.w + r1.w, 0.0, 1.0);

    // Focus: fogged glass sees a very defocused street; a wiped trail sharpens it;
    // inside a droplet the scene snaps nearly into focus.
    let base_blur = (0.55 + 0.55 * glass_fog) * (1.0 - 0.55 * wipe);
    let blur = mix(base_blur, 0.15, clamp(mask * 1.4, 0.0, 1.0));

    // Droplets invert and compress the scene behind them — the negative gain
    // sweeps the neighbourhood across each drop, so lights swim inside it.
    var col = street(guv - off * 4.5, blur, warmth, bokeh_amt, time);

    // Fog veil: dims and flattens the street except where wiped or lensed.
    let fog = glass_fog * (1.0 - 0.8 * clamp(mask * 1.5 + wipe, 0.0, 1.0));
    col = mix(col, vec3<f32>(0.032, 0.036, 0.046), fog * 0.38);

    // Droplet shading: darkened rim, and a soft up-left glint catching the sky.
    let rim = mask * (1.0 - mask) * 4.0;
    col = col * (1.0 - rim * 0.14);
    let g = clamp(dot(off / (length(off) + 1e-4), vec2<f32>(-0.45, 0.80)), 0.0, 1.0);
    col = col + vec3<f32>(0.75, 0.85, 1.00) * pow(g, 5.0) * mask * 0.22;

    // Lock-screen ease: dim the street and let the rain settle.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.5 + vec3<f32>(0.006, 0.008, 0.012), l);

    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    // Per-world sRGB flag (push lock_alpha.z): gamma-encode for the brighter,
    // preview-matching look on a non-sRGB scanout buffer. Off = raw values.
    let outc = select(col, pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)), pc.lock_alpha.z > 0.5);
    return vec4<f32>(outc, 1.0) * (alpha * 0.75);
}
