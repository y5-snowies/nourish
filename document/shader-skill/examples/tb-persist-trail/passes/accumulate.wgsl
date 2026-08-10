// TB — `targets.persist`: a target that survives the frame.
//
// Pass 1 of 2. It reads `prev` (which IS `trail`) and writes `trail`, and that is
// the whole feature: for a persistent target those are two different images, so
// what it reads is what it wrote LAST frame. Declaring the same target as both an
// input and the output of one pass is legal only because of that — on an ordinary
// target it would be a feedback loop, and the loader would be handing the pass the
// image it is rendering into.
//
// WHAT TO LOOK FOR
// ----------------
// Drag a window across the screen. It leaves a fading trail of its own pixels
// behind it, following the path it actually took — curves included, which is what
// separates this from a directional smear. Stop moving and the trail drains away
// over a second or two.
//
// Set `decay` to 1.0 and nothing ever fades: the screen fills with every position
// the window has occupied since the bundle was selected. That is the clearest
// proof the target is genuinely persisting rather than being re-cleared.
//
// WHY TWO PASSES
// --------------
// The accumulator has to be read back next frame, so it must be a target rather
// than the swapchain — the swapchain image is a different buffer every frame and
// belongs to the display, not to us. So pass 1 maintains the trail and pass 2
// draws it. That is also why `present` re-draws the windows: `trail` deliberately
// holds only the ghost.
//
// @prop decay   float default=0.94 min=0.80 max=1.00 step=0.005 label="Trail persistence" group="Trail"
// @prop soak    float default=0.85 min=0.00 max=1.00 step=0.01  label="Ink pickup"        group="Trail"

enable wgpu_binding_array;

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var prev: texture_2d<f32>;

struct Windows {
    count: u32,
    _pad: vec3<u32>,
    rects: array<vec4<f32>, 256>,
    srcs: array<vec4<f32>, 256>,
    attrs: array<vec4<f32>, 256>,
};
@group(1) @binding(0) var<uniform> windows: Windows;
@group(1) @binding(1) var win_tex: binding_array<texture_2d<f32>>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    let decay = pc.params[0].x;
    let soak = pc.params[0].y;

    // LAST FRAME'S accumulator, faded. The engine bound the other image of the
    // pair here, so this read cannot see anything written by this invocation.
    var acc = textureSampleLevel(prev, samp, uv, 0.0).rgb * decay;

    // Everything a window covers this frame is soaked into the accumulator at
    // full strength, so the head of the trail is the window and the tail is its
    // history. Front to back, first cover wins — a pixel belongs to whichever
    // window is on top of it.
    let n = min(windows.count, 256u);
    for (var k = 0u; k < n; k = k + 1u) {
        let i = n - 1u - k;
        if (windows.attrs[i].x > 0.5) {
            continue;
        }
        let r = windows.rects[i];
        let hi = r.xy + r.zw;
        if (uv.x < r.x || uv.x > hi.x || uv.y < r.y || uv.y > hi.y) {
            continue;
        }
        let s = windows.srcs[i];
        let local = (uv - r.xy) / max(r.zw, vec2<f32>(0.0001));
        let pp = textureSampleLevel(win_tex[i], samp, s.xy + local * s.zw, 0.0);
        acc = max(acc, pp.rgb * clamp(pp.a, 0.0, 1.0) * soak);
        break;
    }

    return vec4<f32>(acc, 1.0);
}
