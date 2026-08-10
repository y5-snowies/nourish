// EVERY descriptor and EVERY timestamp, in one picture.
//
// This is a test bundle, not a nice one. It owns the whole world band
// (`windows: "world"`) so it can draw each window itself, and every visual choice
// below is picked to make ONE descriptor unmistakable rather than to look good.
// If a value is wrong you should be able to say which one without reading code.
//
// WHAT TO LOOK FOR
// ----------------
//   flags    ACTIVATED   focused window keeps full colour; unfocused ones grey out
//            TOPMOST     a bright outline, on exactly one window
//            FULLSCREEN  the outline turns solid instead of dashed
//            RESIZING    the window shears with a scanline wobble while resizing
//            MOVING      the window leans, top-to-bottom, while it is dragged
//
//   times    opened      a new window scales up and fades in over ~0.6 s
//            entered     coming back on-screen flashes a white rim, ~0.5 s
//            left        a window that was away LONGER than 3 s comes back with a
//                        blue rim instead of a white one
//            focused     a ring sweeps inward from the border on focus, ~0.5 s
//            topmost     the outline pulses once on becoming front-most
//            resize end  a green settle-flash when a resize finishes, ~0.4 s
//            resize start is what drives the shear above
//            move start  eases the tilt in over ~0.2 s rather than snapping
//            move end    an amber settle-flash when a drag is dropped, ~0.4 s
//
//   pointer  position    a crosshair follows the cursor, over everything
//            buttons     the crosshair fills in while a button is held —
//                        white = left, red = right, green = middle
//            last down   a ring expands from the click point, ~0.5 s
//            last up     a thinner ring, one that CONTRACTS, on release
//
// The six `@prop`s are gains, not switches: turn one to 0 to take an effect out
// of the picture while you check another.
//
// AGES, NOT TIMESTAMPS. `times.*` are absolute moments on the shared clock and `t`
// is on that same clock, so an age is `t - moment`. An event that never happened
// is negative (`NEVER`), and `t - NEVER` is a large age — an animation already
// over — which is why nothing here special-cases it except where a value means
// "how long was it gone".
//
// @prop life    float default=1.0 min=0.0 max=1.0 step=0.01 label="Open / enter"   group="Timing"
// @prop focus   float default=1.0 min=0.0 max=1.0 step=0.01 label="Focus ring"     group="Timing"
// @prop resize  float default=1.0 min=0.0 max=1.0 step=0.01 label="Resize"         group="Timing"
// @prop drag    float default=1.0 min=0.0 max=1.0 step=0.01 label="Move"           group="Timing"
// @prop mark    float default=1.0 min=0.0 max=1.0 step=0.01 label="Outlines"       group="Marks"
// @prop cursor  float default=1.0 min=0.0 max=1.0 step=0.01 label="Cursor"         group="Marks"

enable wgpu_binding_array;

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;

// `_pad: vec3<u32>` is 16-byte aligned, so the arrays start at offset 32. Keep it.
struct Windows {
    count: u32,
    _pad: vec3<u32>,
    rects: array<vec4<f32>, 256>,
    srcs: array<vec4<f32>, 256>,
    // x = kind (0 = client window, 1 = iced-world panel), y = element alpha,
    // z = descriptor flags, w = reserved.
    attrs: array<vec4<f32>, 256>,
};
@group(1) @binding(0) var<uniform> windows: Windows;
@group(1) @binding(1) var win_tex: binding_array<texture_2d<f32>>;

// `window_times`. Index-aligned with `windows`, and only bound because the
// manifest asked for it — without the requirement this block is never written.
struct Times {
    count: u32,
    _pad: vec3<u32>,
    // x = opened, y = entered, z = left, w = reserved
    life: array<vec4<f32>, 256>,
    // x = focused, y = topmost, z = selected, w = deselected
    state: array<vec4<f32>, 256>,
    // x = resize started, y = resize ended, z = move started, w = move ended
    drag: array<vec4<f32>, 256>,
};
@group(1) @binding(2) var<uniform> times: Times;

// `pointer_state`. ONE value for the frame, not one per drawable — which is why
// it is its own block rather than a lane in the arrays above.
struct Pointer {
    // xy = screen UV, z = held buttons, w = reserved
    at: vec4<f32>,
    // x = last pressed, y = last released, z / w = reserved
    moment: vec4<f32>,
};
@group(1) @binding(3) var<uniform> pointer: Pointer;

const BTN_LEFT: u32 = 1u;
const BTN_RIGHT: u32 = 2u;
const BTN_MIDDLE: u32 = 4u;

const ACTIVATED: u32 = 1u;
const TOPMOST: u32 = 2u;
const FULLSCREEN: u32 = 4u;
const RESIZING: u32 = 8u;
const MOVING: u32 = 16u;
const SELECTED: u32 = 32u;
const PRIMARY: u32 = 64u;

fn has(flags: u32, bit: u32) -> bool {
    return (flags & bit) != 0u;
}

/// Seconds since `moment`, clamped at zero. A moment that has not happened is
/// negative, so this reports it as very old rather than as about to happen.
fn age(t: f32, moment: f32) -> f32 {
    return max(t - moment, 0.0);
}

/// 1 at the event, falling to 0 over `span`.
fn decay(t: f32, moment: f32, span: f32) -> f32 {
    return 1.0 - smoothstep(0.0, span, age(t, moment));
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    let aspect = res.x / max(res.y, 1.0);

    let g_life = pc.params[0].x;
    let g_focus = pc.params[0].y;
    let g_resize = pc.params[0].z;
    let g_mark = pc.params[0].w;
    let g_drag = pc.params[1].x;
    let g_cursor = pc.params[1].y;

    // A dim, slowly-moving backdrop. Deliberately plain: everything interesting in
    // this bundle happens on the windows.
    var col = mix(
        vec3<f32>(0.03, 0.04, 0.07),
        vec3<f32>(0.07, 0.05, 0.10),
        0.5 + 0.5 * sin((uv.x + uv.y) * 2.0 + t * 0.15),
    );

    // Composite the band BACK TO FRONT — the published order — so this draws what
    // the engine would have drawn, with the descriptors applied on the way past.
    let n = min(windows.count, 256u);
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let flags = u32(windows.attrs[i].z);
        let is_window = windows.attrs[i].x < 0.5;

        // OPENED — the window grows into its rect. Applied to the RECT, before the
        // coverage test, so the window genuinely occupies less of the screen while
        // it opens rather than being scaled inside a full-size hole.
        let opened = select(1.0, smoothstep(0.0, 0.60, age(t, times.life[i].x)), is_window);
        let grow = mix(0.90, 1.0, mix(1.0, opened, g_life));
        let mid = r.xy + r.zw * 0.5;
        let size = r.zw * grow;
        var lo = mid - size * 0.5;
        var hi = mid + size * 0.5;

        // RESIZING — a horizontal shear that wobbles with the scanline, so a
        // window being dragged is obvious even in a still screenshot. Driven by
        // the resize-start moment, which is what makes it settle rather than
        // switch on: the wobble decays over the first second of the drag.
        var shear = 0.0;
        if (is_window && has(flags, RESIZING)) {
            let held = decay(t, times.drag[i].x, 1.0) * 0.6 + 0.4;
            shear = sin(uv.y * 90.0 + t * 22.0) * 0.010 * held * g_resize;
        }

        // MOVING — a steady tilt rather than a wobble, so the two gestures never
        // read as the same effect. The move-start moment EASES it in: the flag
        // alone would snap the window sideways the instant the drag began, which
        // is the difference a timestamp buys over a boolean.
        var lean = 0.0;
        if (is_window && has(flags, MOVING)) {
            let ease = smoothstep(0.0, 0.20, age(t, times.drag[i].z));
            lean = (uv.y - (r.y + r.w * 0.5)) * 0.14 * ease * g_drag;
        }
        shear = shear + lean;
        let suv = vec2<f32>(uv.x - shear, uv.y);

        if (suv.x < lo.x || suv.x > hi.x || suv.y < lo.y || suv.y > hi.y) {
            continue;
        }
        let s = windows.srcs[i];
        let local = (suv - lo) / max(size, vec2<f32>(0.0001));
        let pp = textureSampleLevel(win_tex[i], samp, s.xy + local * s.zw, 0.0);
        var rgb = pp.rgb;
        var a = clamp(pp.a, 0.0, 1.0) * windows.attrs[i].y;

        // An iced-world panel is not a client window: blit it plainly and let the
        // descriptors alone. A placeholder greyed out for being "unfocused" would
        // read as a bug in the compositor rather than as a demo.
        if (!is_window) {
            col = col * (1.0 - a) + rgb * a;
            continue;
        }

        // ACTIVATED — the unfocused windows desaturate and dim. The single
        // clearest read of a boolean descriptor: click between two windows and the
        // whole picture answers.
        if (!has(flags, ACTIVATED)) {
            let grey = dot(rgb, vec3<f32>(0.299, 0.587, 0.114));
            rgb = mix(rgb, vec3<f32>(grey), 0.75) * 0.72;
        }

        // OPENED, part two — fade in with the grow above.
        a = a * mix(1.0, opened, g_life);

        // RESIZE ENDED — a green wash as the window settles. Distinct from the
        // shear so you can see the transition between them: the shear stops and
        // this fires.
        let settled = decay(t, times.drag[i].y, 0.40) * g_resize;
        rgb = rgb + vec3<f32>(0.0, 0.45, 0.15) * settled;

        // MOVE ENDED — amber, so a drop is distinguishable from a resize settle at
        // a glance. Same shape, different lane and different colour: if the two
        // ever swap you will see it immediately.
        let dropped = decay(t, times.drag[i].w, 0.40) * g_drag;
        rgb = rgb + vec3<f32>(0.50, 0.32, 0.0) * dropped;

        col = col * (1.0 - a) + rgb * a;

        // ---- edge marks, drawn over the window's own pixels ----
        // Distance to the nearest edge of this window, in aspect-corrected units
        // so a mark is the same thickness horizontally and vertically.
        let d = min(
            min(suv.x - lo.x, hi.x - suv.x) * aspect,
            min(suv.y - lo.y, hi.y - suv.y),
        );

        // TOPMOST — an outline on exactly one window. FULLSCREEN switches it from
        // dashed to solid, so the two flags are legible at once on the same mark.
        if (has(flags, TOPMOST)) {
            let along = (suv.x * aspect + suv.y) * 60.0;
            let dashed = step(0.0, sin(along));
            let solid = select(dashed, 1.0, has(flags, FULLSCREEN));
            // …and a single pulse on BECOMING topmost, from the timestamp.
            let pulse = decay(t, times.state[i].y, 0.45);
            let width = 0.0035 + 0.0045 * pulse;
            let edge = (1.0 - smoothstep(0.0, width, d)) * solid;
            col = col + vec3<f32>(1.0, 0.85, 0.35) * edge * (0.55 + 0.45 * pulse) * g_mark;
        }

        // FOCUSED (the moment, not the flag) — a ring that sweeps INWARD from the
        // border and fades. The flag says which window; this says how long ago,
        // and the two are visibly different things on screen.
        let f_age = age(t, times.state[i].x);
        if (f_age < 0.5) {
            let sweep = f_age / 0.5;
            let ring = 1.0 - smoothstep(0.0, 0.012, abs(d - sweep * 0.05));
            col = col + vec3<f32>(0.35, 0.75, 1.0) * ring * (1.0 - sweep) * g_focus;
        }

        // ENTERED / LEFT — a rim when a window comes back on-screen, coloured by
        // how long it had been gone. `left` is only ever readable in hindsight:
        // while a window is off-screen it has no entry here at all, so this is the
        // only moment either value can be observed.
        let e_age = age(t, times.life[i].y);
        if (e_age < 0.5) {
            let gone = times.life[i].y - times.life[i].z;
            let long_gone = select(0.0, 1.0, times.life[i].z >= 0.0 && gone > 3.0);
            let tint = mix(vec3<f32>(1.0), vec3<f32>(0.30, 0.55, 1.0), long_gone);
            let rim = 1.0 - smoothstep(0.0, 0.010, d);
            col = col + tint * rim * (1.0 - e_age / 0.5) * g_life;
        }
    }

    // ---- the pointer, over everything ----
    // Drawn after the band so it is never occluded by a window: this is a probe,
    // and a probe you cannot see when it matters is not one.
    if (g_cursor > 0.0) {
        // Aspect-corrected distance, so the crosshair is round and the rings are
        // circles rather than ellipses on a wide screen.
        let d = length((uv - pointer.at.xy) * vec2<f32>(aspect, 1.0));
        let held = u32(pointer.at.z);

        // BUTTONS — the colour says which, so a bundle reading the wrong bit is
        // visible rather than merely wrong. Nothing held draws the outline only.
        var tint = vec3<f32>(0.85, 0.90, 1.0);
        if ((held & BTN_RIGHT) != 0u) { tint = vec3<f32>(1.0, 0.35, 0.30); }
        else if ((held & BTN_MIDDLE) != 0u) { tint = vec3<f32>(0.40, 1.0, 0.45); }

        // POSITION — a ring outline always, filled in only while a button is
        // down. The fill is the flag; the ring is the position.
        let ring = (1.0 - smoothstep(0.010, 0.014, d)) * smoothstep(0.007, 0.009, d);
        let fill = select(0.0, 1.0 - smoothstep(0.005, 0.008, d), held != 0u);
        col = col + tint * (ring + fill * 0.8) * g_cursor;

        // LAST PRESSED — a ring that EXPANDS away from the click and fades.
        let press = age(t, pointer.moment.x);
        if (press < 0.5) {
            let s0 = press / 0.5;
            let r = 1.0 - smoothstep(0.0, 0.010, abs(d - s0 * 0.09));
            col = col + tint * r * (1.0 - s0) * g_cursor;
        }
        // LAST RELEASED — one that CONTRACTS, so press and release are told apart
        // by direction rather than by having to catch which fired.
        let rel = age(t, pointer.moment.y);
        if (rel < 0.5) {
            let s1 = rel / 0.5;
            let r = 1.0 - smoothstep(0.0, 0.006, abs(d - (1.0 - s1) * 0.06));
            col = col + vec3<f32>(1.0, 0.85, 0.40) * r * (1.0 - s1) * 0.7 * g_cursor;
        }
    }

    if (pc.lock_alpha.z > 0.5) {
        col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0);
}
