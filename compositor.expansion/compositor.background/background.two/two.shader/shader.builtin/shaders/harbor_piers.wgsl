// Built-in background: "Harbor Piers" — the perspective companion to "Harbor
// Beacon". The view from inside a marina at night: you stand on a plank
// promenade that runs across the whole foreground behind its handrail, big warm
// quay lamps pooling light at your feet, and the piers leave *from it* — three
// broad decks angling out into the bay on their own headings, plus a floating
// dock out in the channel, all raised on pilings, each with a coloured marker
// light at its head. Moored boats sleep beside the decks, a thicket of sailboat
// masts sways beyond them, the channel buoys blink their nautical rhythms, low
// mist drifts in patches over the water, and the lighthouse sweeps its beam off
// to the left. The camera never moves; only the water, the lights and the beam
// do. Same Push / `@prop` contract as the rest of the built-in set.
//
// Design notes (up-positive screen space, `y = -uv.y`; screen-anchored vista,
// zoom ignored, a whisper of pan):
//   * Perspective: a pixel `dy` below the horizon lies at depth `d = K / dy`,
//     with the eye high enough (K) that the near water keeps texture. Waves are
//     world-space noise octaves biased along z plus a screen-space micro-ripple;
//     a slow `wz` drift animates the surface without any camera motion.
//   * Each deck is a raised slab on its own *heading*: the strip's centreline is
//     `wx = wx0 + m·z`, so every pier converges toward its own vanishing point
//     `x = m·K` — spread off-centre instead of stacking at the screen middle.
//     The walking surface lives at LIFT of eye height (projected with K2); the
//     channel-side face is the vertical plane between deck lip and waterline
//     (`z = wxe·K / (x - m·K)`), ribbed by pilings that blur into a mid tone
//     with distance; a moonlit lip line and a handrail run along the same edge.
//   * The quay shows only its walking surface and lip — from on top, your own
//     quay's side face is occluded by the deck (you'd have to lean over), so
//     none is drawn. Its handrail is drawn LAST of the solids, after the boats
//     and buoys, because it is the nearest thing in the scene — bokeh behind a
//     railing is the marina look — with gaps where the piers leave.
//   * Lamp posts stand ON the decks, pooling warm light onto the planks; their
//     streaks are masked off the decks — reflections belong to the water.
//   * Everything luminous is a defocus disc (`bdisc`), the beacon is the same
//     three coupled signals as Harbor Beacon (rotating fog-gated spokes, a
//     once-per-revolution flare, a surging water column), and the mist veils
//     the scene before the beam draws — mist is what reveals a beam.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop beacon_speed float default=1.0 min=0.0 max=3.0 label="Beacon speed" group="Beacon"
// @prop beacon_glow float default=1.0 min=0.0 max=2.0 label="Beacon glow" group="Beacon"
// @prop pier_lights float default=1.0 min=0.0 max=2.0 label="Deck lights" group="Piers"
// @prop buoy_density float default=1.0 min=0.0 max=2.0 label="Buoy markers" group="Bay"
// @prop shore_lights float default=1.0 min=0.0 max=2.0 label="Shore lights" group="Bay"
// @prop boat_traffic float default=1.0 min=0.0 max=2.0 label="Boat traffic" group="Bay"
// @prop warmth float default=0.55 min=0.0 max=1.0 label="Cool → warm lights" group="Bay"
// @prop fog float default=0.5 min=0.0 max=1.0 label="Sea mist" group="Mood"
// @prop defocus float default=0.75 min=0.0 max=1.5 label="Defocus" group="Mood"
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
    let pier = pc.params[0].z;
    let buoys = pc.params[0].w;
    let shore = pc.params[1].x;
    let traffic = pc.params[1].y;
    let warmth = clamp(pc.params[1].z, 0.0, 1.0);
    let fog = clamp(pc.params[1].w, 0.0, 1.0);
    let blur = clamp(pc.params[2].x, 0.0, 1.5);
    let moonlight = pc.params[2].y;
    let swell = pc.params[2].z;
    let vignette = pc.params[2].w;
    let vig_radius = pc.params[3].x;
    let vig_softness = pc.params[3].y;

    let screen_uv = (frag - 0.5 * res) / res.y;
    // Work in up-positive space (fragment y grows downward); a whisper of pan.
    let pan = vec2<f32>(pan_in.x, -pan_in.y);
    let p = vec2<f32>(screen_uv.x + pan.x * 0.00015, -screen_uv.y + pan.y * 0.00015);

    // Horizon a touch above centre, and the ground-plane projection constant —
    // K is the eye height: a pixel `dy` below the horizon lies at depth K / dy.
    let yh0 = 0.06;
    let K = 0.11;
    let dyp = yh0 - p.y;                 // drop below the horizon (<0 in the sky)
    let shim = 0.7 + 0.6 * noise(vec2<f32>(p.x * 50.0, p.y * 14.0 - time * 0.9));

    // ---- Water: a still perspective plane, alive with a slow drift ----------
    let d = K / max(dyp, 1e-4);
    let wx = p.x * d / K;
    let wz = d;
    let wzs = wz - time * 0.05 * (0.5 + 0.5 * swell);  // gentle current, no camera
    let det = clamp(dyp * 8.0, 0.0, 1.0);   // fine detail fades toward the horizon
    let amp = 0.55 + 0.45 * swell;
    // Octaves biased along z so crests foreshorten without smearing; a screen-
    // space micro-ripple keeps the near field textured.
    let w1 = noise(vec2<f32>(wx * 1.8, wzs * 2.8));
    let w2 = noise(vec2<f32>(wx * 4.2 + 7.0, wzs * 6.0));
    let w3 = noise(vec2<f32>(wx * 9.0, wzs * 13.0));
    let wr = noise(vec2<f32>(p.x * 46.0, p.y * 26.0 - time * 0.35));
    let wave = (w1 * 0.45 + w2 * 0.30 + w3 * 0.15 * det + wr * 0.10) * amp;
    let fogd = 1.0 - exp(-d * (0.05 + 0.18 * fog));  // distance haze on the plane
    var wcol = vec3<f32>(0.010, 0.016, 0.022) * (0.65 + 0.7 * wave);
    wcol = mix(wcol, vec3<f32>(0.040, 0.050, 0.064), fogd * 0.8);
    // Crests catch a little pale light; a slope tap gives moonlit facets.
    let crest = pow(clamp(wave * 1.4 - 0.35, 0.0, 1.0), 2.0);
    wcol = wcol + vec3<f32>(0.50, 0.60, 0.75) * crest * 0.05 * (0.4 + 0.6 * moonlight) * det;
    let w2b = noise(vec2<f32>(wx * 4.2 + 7.0, wzs * 6.0 - 0.30));
    let facet = clamp((w2 - w2b) * 3.0, 0.0, 1.0);
    let mc = vec3<f32>(0.72, 0.80, 0.94);
    wcol = wcol + mc * facet * 0.03 * det * moonlight;

    // ---- Sky, joined across the horizon -------------------------------------
    let sky = mix(vec3<f32>(0.012, 0.016, 0.030), vec3<f32>(0.034, 0.043, 0.056),
                  exp(-max(p.y - yh0, 0.0) * 2.6));
    var col = mix(wcol, sky, smoothstep(-0.004, 0.004, p.y - yh0));

    // ---- Stars and moon ------------------------------------------------------
    let sp = p * 16.0;
    let sip = floor(sp);
    let sh = hash(sip + 71.0);
    if (p.y - yh0 > 0.05 && sh > 0.80) {
        let sj = vec2<f32>(hash(sip + 3.0), hash(sip + 9.0)) - 0.5;
        let sd = length(fract(sp) - 0.5 - sj * 0.6) / 16.0;
        let twinkle = 0.7 + 0.3 * sin(time * (1.0 + sh * 3.0) + sh * 40.0);
        col = col + vec3<f32>(0.60, 0.70, 0.90)
                  * bdisc(sd, 0.0016 + 0.0020 * fract(sh * 7.0), blur)
                  * 0.35 * twinkle * smoothstep(0.05, 0.15, p.y - yh0) * (1.0 - 0.6 * fog);
    }
    let mp = vec2<f32>(0.30, 0.31);
    let md = length(p - mp);
    col = col + mc * (bdisc(md, 0.028, blur * 0.7) * 0.9 + exp(-md * 6.5) * 0.10) * moonlight;
    // Glade: a sparkling column under the moon, widening as the mist thickens.
    let glade = exp(-abs(p.x - mp.x * 0.85) * (6.0 - 2.5 * fog))
              * exp(-max(dyp, 0.0) * 1.6) * step(0.0, dyp);
    col = col + mc * glade * (0.04 + 0.28 * pow(w2, 6.0)) * moonlight;

    // ---- Far shore: glow band plus a string of bokeh lights -----------------
    let glowc = mix(vec3<f32>(0.20, 0.28, 0.42), vec3<f32>(0.44, 0.28, 0.15), warmth);
    col = col + glowc * 0.06 * exp(-abs(p.y - yh0) * 7.0) * (0.3 + 0.7 * shore);
    let ssc = 3.4;
    let scell0 = floor(p.x * ssc);
    for (var i = -1; i <= 1; i = i + 1) {
        let cell = scell0 + f32(i);
        let h = hash(vec2<f32>(cell, 5.3));
        if (h > 0.30) {
            let cx = (cell + 0.5 + (h - 0.5) * 0.7) / ssc;
            let ly = yh0 + 0.005 + fract(h * 13.0) * 0.014;
            let dsl = length(p - vec2<f32>(cx, ly));
            let c = lightcol(h, warmth);
            col = col + c * (bdisc(dsl, 0.0030 + 0.005 * fract(h * 29.0), blur) * 1.4
                             + exp(-dsl * 22.0) * 0.04) * shore;
            let rf = exp(-abs(p.x - cx) * 30.0) * exp(-max(dyp, 0.0) * 4.2) * shim
                   * step(0.0, dyp);
            col = col + c * rf * 0.08 * shore;
        }
    }

    // ---- A distant boat crawling across the bay mouth -----------------------
    let laneY = yh0 - 0.022;
    let bsc = 1.6;
    let bcp = p.x * bsc + time * 0.010;
    let bcip = floor(bcp);
    for (var i = -1; i <= 1; i = i + 1) {
        let cell = bcip + f32(i);
        let h = hash(vec2<f32>(cell, 33.0));
        if (h > 1.0 - 0.4 * traffic) {
            let cx = (cell + 0.5 + (h - 0.5) * 0.30 - time * 0.010) / bsc;
            let yb = laneY + sin(time * (0.5 + 0.4 * h) + h * 40.0) * 0.003 * swell;
            let hd = (p - vec2<f32>(cx, yb)) / vec2<f32>(0.028, 0.0075);
            let cd = (p - vec2<f32>(cx - 0.004, yb + 0.009)) / vec2<f32>(0.013, 0.010);
            let sil = clamp(smoothstep(1.0, 0.55, length(hd))
                          + smoothstep(1.0, 0.50, length(cd)), 0.0, 1.0);
            col = mix(col, vec3<f32>(0.010, 0.012, 0.016), sil * 0.85);
            let mastc = vec3<f32>(0.95, 0.97, 1.00);
            let cabc = vec3<f32>(1.00, 0.72, 0.38);
            col = col + mastc * bdisc(length(p - vec2<f32>(cx, yb + 0.028)), 0.0032, blur) * 1.5;
            col = col + cabc * bdisc(length(p - vec2<f32>(cx - 0.004, yb + 0.008)), 0.0038, blur) * 0.8;
            let rfb = exp(-abs(p.x - cx) * 34.0) * exp(min(p.y - yb, 0.0) * 7.0)
                    * shim * smoothstep(0.004, -0.012, p.y - yb);
            col = col + (mastc * 0.5 + cabc * 0.5) * rfb * 0.09;
        }
    }

    // ---- Sailboat masts swaying beyond the piers ----------------------------
    let msc = 9.0;
    let mcell0 = floor(p.x * msc);
    for (var i = -1; i <= 1; i = i + 1) {
        let cell = mcell0 + f32(i);
        let h = hash(vec2<f32>(cell, 61.0));
        if (h > 0.45) {
            let mx_ = (cell + 0.5 + (h - 0.5) * 0.8) / msc;
            let band = smoothstep(0.02, 0.12, mx_) * (1.0 - smoothstep(0.70, 0.90, mx_));
            if (band > 0.01) {
                let hm = 0.035 + 0.075 * fract(h * 7.0);
                let wm = 0.0012 + 0.0012 * fract(h * 3.0);
                let sway = sin(time * (0.5 + h) + h * 30.0) * 0.004 * swell;
                let lean = sway * clamp((p.y - yh0) / hm, 0.0, 1.0);
                let mastm = smoothstep(wm, wm * 0.3, abs(p.x - mx_ - lean));
                let vert = smoothstep(yh0 - 0.012, yh0 - 0.006, p.y)
                         * (1.0 - smoothstep(yh0 + hm, yh0 + hm + 0.004, p.y));
                col = mix(col, vec3<f32>(0.010, 0.012, 0.016), mastm * vert * band * 0.85);
                // The hull huddle at the waterline, and a masthead spark on some.
                let hb = smoothstep(1.0, 0.5,
                    length((p - vec2<f32>(mx_, yh0 - 0.010)) / vec2<f32>(0.020, 0.006)));
                col = mix(col, vec3<f32>(0.010, 0.012, 0.016), hb * band * 0.7);
                if (fract(h * 11.0) > 0.6) {
                    col = col + vec3<f32>(0.90, 0.85, 0.70)
                              * bdisc(length(p - vec2<f32>(mx_ + sway, yh0 + hm)), 0.0016, blur)
                              * 0.8 * band;
                }
            }
        }
    }

    // ---- The marina decks ---------------------------------------------------
    // Shared projection for every raised surface: the walking planes live at
    // LIFT of eye height and project with K2.
    let LIFT = 0.10;
    let K2 = K * (1.0 - LIFT);
    let z_top = K2 / max(dyp, 1e-4);       // depth on the raised deck plane
    let wxt = p.x * z_top / K;             // world x on that plane
    let plank = smoothstep(0.0, 0.30, abs(fract(z_top / 0.13) - 0.5));
    let grain = noise(vec2<f32>(wxt * 16.0, z_top * 9.0));
    let fog_top = 1.0 - exp(-z_top * (0.05 + 0.18 * fog));
    let mistc = vec3<f32>(0.040, 0.050, 0.064);
    var deckall = 0.0;

    // The quay: the promenade you are standing on, filling the frame bottom.
    // Only the walking surface and its moonlit lip — the side face is occluded
    // by the deck itself from this eye height. Board joins cross the planks,
    // and the nearest boards fall into your own shadow.
    let zq = 0.30;
    let qm = (1.0 - smoothstep(zq, zq + 0.08, z_top)) * step(0.0, dyp);
    let board = smoothstep(0.34, 0.50, abs(fract(wxt / 0.30) - 0.5));
    var quaycol = vec3<f32>(0.020, 0.018, 0.015) * (0.70 + 0.25 * plank + 0.20 * grain);
    quaycol = quaycol * (1.0 - 0.10 * board) * (1.0 - 0.22 * smoothstep(0.30, 0.52, dyp));
    quaycol = mix(quaycol, mistc, fog_top * 0.8);
    col = mix(col, quaycol, qm);
    let lipq = smoothstep(0.0022, 0.0, abs(dyp - K2 / zq));
    col = col + vec3<f32>(0.55, 0.62, 0.75) * lipq * 0.05 * moonlight;
    deckall = max(deckall, qm);

    // The piers: three decks leaving the quay on their own headings, plus a
    // floating dock out in the channel. Centreline wx = wx0 + m·z, so each
    // strip converges toward its own off-centre vanishing point x = m·K.
    var a_wx0 = array<f32, 4>(-0.55, 0.30, 1.60, 1.30);
    var a_pw = array<f32, 4>(0.26, 0.22, 0.24, 0.11);
    var a_m = array<f32, 4>(-0.90, 0.60, 0.30, 0.0);
    var a_z0 = array<f32, 4>(0.18, 0.18, 0.18, 0.60);
    var a_z1 = array<f32, 4>(2.60, 1.80, 0.90, 0.95);
    for (var pi = 0; pi < 4; pi = pi + 1) {
        let wx0 = a_wx0[pi];
        let pw = a_pw[pi];
        let m = a_m[pi];
        let z0 = a_z0[pi];
        let z1 = a_z1[pi];
        let wxe = select(wx0 - pw, wx0 + pw, wx0 < 0.0);   // edge facing the channel
        // The walking surface: planks foreshortening on the raised plane.
        let ax = abs(wxt - (wx0 + m * z_top));
        let mx = smoothstep(pw, pw - 0.06, ax);
        let mz = smoothstep(z0 - 0.08, z0, z_top) * (1.0 - smoothstep(z1, z1 + 0.4, z_top))
               * step(0.0, dyp);
        let deckm = mx * mz;
        var deckcol = vec3<f32>(0.020, 0.018, 0.015) * (0.70 + 0.25 * plank + 0.20 * grain);
        deckcol = deckcol * (1.0 - 0.40 * smoothstep(pw - 0.10, pw - 0.02, ax));
        deckcol = mix(deckcol, mistc, fog_top * 0.8);
        col = mix(col, deckcol, deckm);
        // Map each screen x to the depth of the channel-side edge plane there.
        let xr0 = p.x - m * K;
        let xr = select(xr0, 1e-4, abs(xr0) < 1e-4);
        let zf = clamp(wxe * K / xr, -100.0, 100.0);
        let zfp = max(zf, 0.05);
        let sideok = step(0.0, xr * wxe)
                   * smoothstep(max(z0, zq), max(z0, zq) + 0.05, zfp)
                   * (1.0 - smoothstep(z1, z1 + 0.3, zfp));
        // Moonlit lip line along the deck's edge.
        let dyl = xr * (1.0 - LIFT) / wxe;
        let lip = smoothstep(0.0016 + dyp * 0.004, 0.0, abs(dyp - dyl));
        col = col + vec3<f32>(0.55, 0.62, 0.75) * lip * sideok * 0.05 * moonlight;
        // Side face between lip and waterline, ribbed by pilings; the gaps
        // between piles read almost black — the dark under-deck. Far away the
        // ribs melt into a mid tone instead of shimmering.
        let ft = K2 / zfp;
        let fb = K / zfp;
        let fm = smoothstep(ft - 0.001, ft + 0.002, dyp)
               * (1.0 - smoothstep(fb - 0.001, fb + 0.003, dyp)) * sideok;
        var postband = smoothstep(0.30, 0.16, abs(fract(zfp / 0.45) - 0.5));
        postband = mix(postband, 0.45, smoothstep(1.2, 2.4, zfp));
        var fcol = mix(vec3<f32>(0.004, 0.005, 0.006), vec3<f32>(0.014, 0.013, 0.011), postband);
        fcol = mix(fcol, mistc, (1.0 - exp(-zfp * (0.05 + 0.18 * fog))) * 0.8);
        col = mix(col, fcol, fm);
        // A thin handrail with posts above the same edge.
        let dyr = xr * (1.0 - LIFT - 0.10) / wxe;
        let railm = smoothstep(0.0018 + dyp * 0.010, 0.0, abs(dyp - dyr)) * sideok;
        col = mix(col, vec3<f32>(0.008, 0.009, 0.011), railm * 0.7);
        let rpm = postband * smoothstep(dyr - 0.001, dyr + 0.001, dyp)
                * (1.0 - smoothstep(dyl - 0.001, dyl + 0.001, dyp)) * sideok;
        col = mix(col, vec3<f32>(0.008, 0.009, 0.011), rpm * 0.55);
        // The water in the deck's lee sits a shade darker, lifting the slab.
        let axw = abs(wx - (wx0 + m * wz));
        let sm = (1.0 - smoothstep(pw, pw - 0.05, axw))
               * smoothstep(pw + 0.28, pw + 0.03, axw)
               * smoothstep(max(z0, zq) - 0.05, max(z0, zq) + 0.05, wz)
               * (1.0 - smoothstep(z1, z1 + 0.5, wz)) * step(0.0, dyp);
        col = col * (1.0 - sm * 0.16);
        deckall = max(deckall, max(deckm, fm));
    }

    // ---- Deck lamps: posts standing on the planks, receding edge-wise -------
    var l_wxl = array<f32, 4>(-0.35, 0.14, 1.42, 1.25);
    var l_m = array<f32, 4>(-0.90, 0.60, 0.30, 0.0);
    var l_z0 = array<f32, 4>(0.45, 0.45, 0.40, 0.65);
    var l_dz = array<f32, 4>(0.45, 0.40, 0.25, 0.23);
    var l_n = array<i32, 4>(5, 4, 3, 2);
    for (var pi = 0; pi < 4; pi = pi + 1) {
        for (var k = 0; k < 5; k = k + 1) {
            if (k < l_n[pi]) {
                let zk = l_z0[pi] + f32(k) * l_dz[pi];
                let wxl = l_wxl[pi] + l_m[pi] * zk;
                let dyw = K / zk;
                if (dyw < 0.75 && dyw > 0.008) {
                    let h = hash(vec2<f32>(f32(pi) * 19.0 + f32(k), 47.0));
                    let xs = wxl * K / zk;
                    let ytop = yh0 - (1.0 - LIFT) * dyw;    // post base on the deck
                    let yl = yh0 - 0.52 * dyw;              // lamp head height
                    let yw = yh0 - dyw;                     // the lamp's waterline
                    let c = lightcol(h, warmth) * (0.7 + 0.6 * fract(h * 9.0));
                    let fade = exp(-zk * (0.05 + 0.12 * fog));
                    let postm = smoothstep(0.016 * dyw, 0.006 * dyw, abs(p.x - xs))
                              * smoothstep(ytop - 0.003, ytop, p.y)
                              * (1.0 - smoothstep(yl, yl + 0.004, p.y));
                    col = mix(col, vec3<f32>(0.010, 0.011, 0.014), postm * 0.85);
                    let hp = vec2<f32>(xs, yl + 0.02 * dyw);
                    let r = clamp((0.014 + 0.008 * fract(h * 5.0)) * dyw * 2.2, 0.001, 0.05);
                    col = col + c * (bdisc(length(p - hp), r, blur) * 1.5
                                     + exp(-length(p - hp) * (5.0 / max(dyw, 0.10))) * 0.04)
                              * fade * pier;
                    // A warm pool on the planks (and water) under the lamp.
                    let pxw = wxt - (l_wxl[pi] + l_m[pi] * z_top);
                    let pool = exp(-(pxw * pxw * 2.0
                                     + (z_top - zk) * (z_top - zk)) * 9.0) * step(0.0, dyp);
                    col = col + c * pool * 0.05 * fade * pier;
                    // Streak beside the deck — masked off the planks; reflections
                    // belong to the water. Width and run shrink with dy.
                    let rf = exp(-abs(p.x - xs) * (10.0 / max(dyw, 0.03)))
                           * exp(-max(yw - p.y, 0.0) * (2.5 / max(dyw, 0.03)))
                           * smoothstep(0.004, -0.004, p.y - yw) * shim
                           * (1.0 - deckall * 0.9);
                    col = col + c * rf * 0.10 * fade * pier;
                }
            }
        }
    }

    // Two big quay lamps standing on the promenade — the nearest, warmest bokeh
    // in the frame, pooling light onto the boards at your feet.
    for (var q = 0; q < 2; q = q + 1) {
        let wxq = select(-0.75, 0.62, q == 1);
        let zql = 0.24;
        let dyw = K / zql;
        let xs = wxq * K / zql;
        let h = hash(vec2<f32>(f32(q) * 29.0, 53.0));
        let ytop = yh0 - (1.0 - LIFT) * dyw;
        let yl = yh0 - 0.52 * dyw;
        let c = lightcol(h, clamp(warmth + 0.25, 0.0, 1.0));
        let postm = smoothstep(0.016 * dyw, 0.006 * dyw, abs(p.x - xs))
                  * smoothstep(ytop - 0.003, ytop, p.y)
                  * (1.0 - smoothstep(yl, yl + 0.004, p.y));
        col = mix(col, vec3<f32>(0.010, 0.011, 0.014), postm * 0.85);
        let hp = vec2<f32>(xs, yl + 0.02 * dyw);
        col = col + c * (bdisc(length(p - hp), 0.014 * dyw * 2.2, blur) * 1.5
                         + exp(-length(p - hp) * (5.0 / dyw)) * 0.05) * pier;
        let pxw = wxt - wxq;
        let pool = exp(-(pxw * pxw * 2.0 + (z_top - zql) * (z_top - zql)) * 9.0)
                 * step(0.0, dyp);
        col = col + c * pool * 0.06 * pier;
    }

    // Coloured marker lights at every deck head — red, green, amber, amber —
    // steady (unlike the blinking buoys), each with a short streak.
    for (var pi = 0; pi < 4; pi = pi + 1) {
        let ze = a_z1[pi] - 0.12;
        let wxl = l_wxl[pi] + l_m[pi] * ze;
        let dyw = K / ze;
        let xs = wxl * K / ze;
        let yl = yh0 - 0.52 * dyw;
        let ec = select(select(vec3<f32>(1.00, 0.12, 0.10), vec3<f32>(0.15, 1.00, 0.35), pi == 1),
                        vec3<f32>(1.00, 0.75, 0.35), pi >= 2);
        let r = clamp(0.008 * dyw * 2.2, 0.001, 0.02);
        col = col + ec * bdisc(length(p - vec2<f32>(xs, yl)), r, blur) * 1.6 * pier;
        let rf = exp(-abs(p.x - xs) * (12.0 / max(dyw, 0.03)))
               * exp(-max(yh0 - dyw - p.y, 0.0) * (4.0 / max(dyw, 0.03)))
               * smoothstep(0.004, -0.004, p.y - (yh0 - dyw)) * shim * (1.0 - deckall * 0.9);
        col = col + ec * rf * 0.10 * pier;
    }

    // ---- Moored boats sleeping beside the decks -----------------------------
    let tb = clamp(traffic, 0.0, 1.0);
    for (var b = 0; b < 2; b = b + 1) {
        let bx = select(-0.09, 0.29, b == 1);
        let bdy = select(0.26, 0.24, b == 1);
        let seed = f32(b) * 17.0 + 3.0;
        let bob = sin(time * (0.7 + 0.2 * f32(b)) + seed) * 0.02 * bdy * swell;
        let ybw = yh0 - bdy + bob;               // the boat's waterline
        let hl = 0.30 * bdy;
        let hh = 0.075 * bdy;
        let hd = (p - vec2<f32>(bx, ybw + hh * 0.6)) / vec2<f32>(hl, hh);
        let cd = (p - vec2<f32>(bx - hl * 0.2, ybw + hh * 1.7)) / vec2<f32>(hl * 0.45, hh * 1.1);
        let sil = clamp(smoothstep(1.0, 0.55, length(hd))
                      + smoothstep(1.0, 0.50, length(cd)), 0.0, 1.0);
        col = mix(col, vec3<f32>(0.009, 0.011, 0.014), sil * 0.88 * tb);
        // A thin mast up to a dim anchor light, and a warm porthole still lit.
        let mtx = bx + hl * 0.1;
        let mty = ybw + 0.34 * bdy;
        let mastm = smoothstep(0.010 * bdy, 0.004 * bdy, abs(p.x - mtx))
                  * smoothstep(ybw + hh * 1.4, ybw + hh * 1.8, p.y)
                  * (1.0 - smoothstep(mty, mty + 0.004, p.y));
        col = mix(col, vec3<f32>(0.009, 0.011, 0.014), mastm * 0.8 * tb);
        let mastc = vec3<f32>(0.95, 0.97, 1.00);
        let cabc = vec3<f32>(1.00, 0.72, 0.38);
        col = col + mastc * bdisc(length(p - vec2<f32>(mtx, mty)), 0.020 * bdy, blur) * 1.2 * tb;
        col = col + cabc * bdisc(length(p - vec2<f32>(bx - hl * 0.2, ybw + hh * 1.5)),
                                 0.024 * bdy, blur) * 0.7 * tb;
        let rfb = exp(-abs(p.x - bx) * (9.0 / max(bdy, 0.05)))
                * exp(-max(ybw - p.y, 0.0) * (3.0 / max(bdy, 0.05)))
                * smoothstep(0.004, -0.004, p.y - ybw) * shim * (1.0 - deckall * 0.9);
        col = col + (mastc * 0.4 + cabc * 0.6) * rfb * 0.10 * tb;
    }

    // ---- Channel buoys: red right, green left, keeping nautical time --------
    for (var side = 0; side < 2; side = side + 1) {
        let s = select(1.0, -1.0, side == 1);
        for (var k = 0; k < 4; k = k + 1) {
            let zk = 0.35 * pow(2.0, f32(k)) * (1.0 + f32(side) * 0.35);
            let dy = K / zk;
            let h = hash(vec2<f32>(f32(k) * 7.0 + f32(side), 91.0));
            if (h > 1.0 - 0.45 * buoys) {
                let red = side == 0;                    // starboard hand
                let c = select(vec3<f32>(0.15, 1.00, 0.35), vec3<f32>(1.00, 0.10, 0.08), red);
                let period = select(2.5, 4.0, red);
                let fr = fract(time / period + h * 7.0);
                let blink = smoothstep(0.0, 0.05, fr) * (1.0 - smoothstep(0.10, 0.18, fr));
                let bob = sin(time * (1.2 + h) + h * 50.0) * 0.02 * dy * swell;
                let xs = s * (0.20 + 0.08 * fract(h * 13.0)) * dy;
                let yb = yh0 - 0.80 * dy + bob;
                let fade = exp(-zk * (0.03 + 0.08 * fog));
                let r = clamp(0.010 * dy * 2.2, 0.001, 0.032);
                col = col + c * bdisc(length(p - vec2<f32>(xs, yb)), r, blur)
                          * 2.4 * blink * fade;
                // The barely-there dark float under the lamp.
                let bodym = smoothstep(1.0, 0.4,
                    length((p - vec2<f32>(xs, yh0 - 0.94 * dy + bob))
                           / vec2<f32>(0.030 * dy, 0.045 * dy)));
                col = mix(col, vec3<f32>(0.012, 0.014, 0.018), bodym * 0.6);
                let rfb = exp(-abs(p.x - xs) * (14.0 / max(dy, 0.03)))
                        * exp(-max(yh0 - dy - p.y, 0.0) * (4.0 / max(dy, 0.03)))
                        * smoothstep(0.004, -0.004, p.y - (yh0 - dy)) * shim
                        * (1.0 - deckall * 0.85);
                col = col + c * rfb * 0.14 * blink * fade;
            }
        }
    }

    // ---- The quay handrail: the nearest thing in the scene, drawn over the
    // bay so the bokeh sits behind it, with gaps where the piers leave.
    var gap = 0.0;
    for (var gi = 0; gi < 3; gi = gi + 1) {
        gap = max(gap, smoothstep(a_pw[gi] + 0.05, a_pw[gi] - 0.02,
                                  abs(wxt - (a_wx0[gi] + a_m[gi] * z_top))));
    }
    let wxf = p.x * zq / K;
    let dyrq = (1.0 - LIFT - 0.10) * K / zq;
    let railq = smoothstep(0.0022, 0.0, abs(dyp - dyrq)) * (1.0 - gap);
    col = mix(col, vec3<f32>(0.008, 0.009, 0.011), railq * 0.75);
    let postq = smoothstep(0.30, 0.14, abs(fract(wxf / 0.45) - 0.5))
              * smoothstep(dyrq - 0.0015, dyrq + 0.0015, dyp)
              * (1.0 - smoothstep(K2 / zq - 0.0015, K2 / zq + 0.0015, dyp)) * (1.0 - gap);
    col = mix(col, vec3<f32>(0.008, 0.009, 0.011), postq * 0.6);

    // ---- The headland and lighthouse, off to the left -----------------------
    let lx = -0.50;
    let ltop = yh0 + 0.13;
    let landh = 0.010 + 0.020 * smoothstep(-0.42, -0.85, p.x)
              + noise(vec2<f32>(p.x * 3.0, 7.7)) * 0.006;
    let landm = smoothstep(0.006, -0.006, (p.y - yh0) - landh)
              * smoothstep(-0.004, 0.006, p.y - yh0) * smoothstep(-0.28, -0.55, p.x);
    col = mix(col, vec3<f32>(0.010, 0.012, 0.016), landm * 0.85);
    let rockm = smoothstep(1.0, 0.45,
        length((p - vec2<f32>(lx, yh0 + 0.002)) / vec2<f32>(0.050, 0.012)));
    col = mix(col, vec3<f32>(0.008, 0.010, 0.013), rockm * 0.85);
    let t = clamp((p.y - yh0) / (ltop - yh0), 0.0, 1.0);
    var w = mix(0.014, 0.008, clamp(t / 0.8, 0.0, 1.0));
    w = mix(w, 0.013, smoothstep(0.78, 0.82, t) * (1.0 - smoothstep(0.90, 0.93, t)));
    w = mix(w, 0.006, smoothstep(0.90, 0.94, t));
    let edge = 0.0025 + 0.0035 * blur;
    let iny = smoothstep(yh0 - 0.008, yh0, p.y) * (1.0 - smoothstep(ltop, ltop + edge, p.y));
    let sil = smoothstep(w + edge, w - edge * 0.5, abs(p.x - lx)) * iny;
    col = mix(col, vec3<f32>(0.010, 0.011, 0.015), sil * 0.88);
    // A dim warm window low in the tower — the keeper is in.
    col = col + vec3<f32>(1.00, 0.70, 0.35)
              * bdisc(length(p - vec2<f32>(lx + 0.004, yh0 + 0.045)), 0.0022, blur) * 0.5;

    // ---- Sea mist: patches drifting low over the water, then the horizon band
    let wisp = noise(vec2<f32>(wx * 0.8 + time * 0.02, wz * 0.5))
             * noise(vec2<f32>(wx * 2.0, wz * 1.3 - time * 0.015));
    let wispm = fog * smoothstep(0.02, 0.10, dyp) * (1.0 - smoothstep(0.18, 0.40, dyp)) * wisp;
    col = mix(col, mistc * 1.15, clamp(wispm * 0.8, 0.0, 1.0) * 0.35);
    let mist = noise(vec2<f32>(p.x * 2.2 + time * 0.015, p.y * 7.0)) * 0.6
             + noise(vec2<f32>(p.x * 5.0 - time * 0.010, p.y * 13.0)) * 0.4;
    let fogm = clamp(fog * exp(-abs(p.y - yh0) * 3.2) * (0.45 + 0.75 * mist), 0.0, 1.0);
    col = mix(col, vec3<f32>(0.045, 0.055, 0.070), fogm * 0.55);

    // ---- The beacon: drawn after the mist, because mist reveals it ----------
    let lp = vec2<f32>(lx, yh0 + 0.118);
    let th = time * beacon_speed * 0.65;
    let cs = cos(th);
    let sn = sin(th);
    let u2 = p - lp;
    let rx = cs * u2.x + sn * u2.y;
    let ry = -sn * u2.x + cs * u2.y;
    let beamvis = (0.30 + 0.70 * fog) * beacon_glow;
    let wc = 0.008 + 0.12 * abs(rx);
    let lobe = exp(-(ry * ry) / (wc * wc));
    let fwd = lobe * smoothstep(0.0, 0.03, rx) * exp(-max(rx, 0.0) * 2.8);
    let bwd = lobe * smoothstep(0.0, 0.03, -rx) * exp(-max(-rx, 0.0) * 2.8);
    let lampc = vec3<f32>(1.00, 0.93, 0.74);
    col = col + lampc * (fwd + bwd * 0.35) * 0.25 * beamvis;
    let flash = pow(max(cs, 0.0), 24.0);
    let dlan = length(u2);
    col = col + lampc * (bdisc(dlan, 0.009, blur) * (0.7 + 3.0 * flash)
                         + exp(-dlan * 10.0) * (0.05 + 0.42 * flash)) * beacon_glow;
    // Its column on the water widens with perspective and surges on each flash.
    let rfl = exp(-abs(p.x - lx) * 24.0 / (1.0 + max(dyp, 0.0) * 2.5))
            * exp(-max(dyp, 0.0) * 2.2) * step(0.0, dyp) * shim * (1.0 - deckall * 0.85);
    col = col + lampc * rfl * (0.05 + 0.40 * flash) * beacon_glow;

    // Lock-screen ease: dim the bay and let the night settle.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.5 + vec3<f32>(0.004, 0.006, 0.010), l);

    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    // Per-world sRGB flag (push lock_alpha.z): gamma-encode for the brighter,
    // preview-matching look on a non-sRGB scanout buffer. Off = raw values.
    let outc = select(col, pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)), pc.lock_alpha.z > 0.5);
    return vec4<f32>(outc, 1.0) * (alpha * 0.75);
}
