// "mp-effects" — THE ONE THAT EXERCISES EVERYTHING.
//
// Eight knobs over the parallax, each off at 0. This is the bundle that asks the
// engine for nearly the whole surface, on purpose: the interesting effects are
// all about WHAT IS BEHIND A WINDOW, and that is precisely the thing the
// compositor's flattened `content` has already destroyed.
//
// THE CENTRAL IDEA
// ----------------
// Making a window see-through needs something behind it to reveal, and a
// flattened `content` has already overdrawn it. So this declares
// `"windows": "world"` — the SAME §8d mechanism `tb-window-chroma` uses. The
// engine leaves the whole world band out of `content` and publishes it instead;
// `content` is then exactly the backdrop, and this pass composites the band over
// it. At any pixel the accumulation just before a drawable IS what is behind it,
// exactly, including lower windows.
//
// It used to be `windows: "engine"` and re-derive the backdrop by evaluating
// `parallax_scene` a second time. That is a RECONSTRUCTION, not the picture: it
// knows only the parallax function, so anything else on the background band was
// missing from it, and the window region was then overwritten with a composite
// built from a different backdrop than the one around it. The seam showed at the
// window edge, worst with transparency up — the knob that reveals the most of it.
//
// `behind()` is that function, and frost / glass / transparency are three things
// done with its result:
//
//   * FROST     — average it over a small kernel: blurred backdrop, so a window
//                 reads as ground glass with the desk (or the window) under it
//                 smeared behind.
//   * GLASS     — sample it through a lens that grows toward the window's edges:
//                 the displacement a real pane of glass has, so what is behind
//                 bends as it passes the frame.
//   * CHROME    — composite the window back over it with a CHROMA-KEYED alpha,
//                 so the window's own flat chrome drops out and its text does
//                 not. This is `tb-window-chroma`'s auto-key, and it is what
//                 makes transparency read as a window made of glass rather than
//                 as a window turned down to 50%.
//
// STICKY EDGE is the odd one out and lives in `finish` rather than here: it
// displaces the COMPOSITED scene at window boundaries rather than the backdrop,
// which makes the edge itself look drawn-out and tacky rather than refractive. It
// is not physical and it is not trying to be — and there is no composited scene
// to displace until this pass has written one.
//
// WHAT EACH KNOB READS
// --------------------
// | knob        | reads                          | requirement                     |
// |-------------|--------------------------------|---------------------------------|
// | frost       | `behind()`, blurred            | world_geometry + world_textures |
// | glass       | `behind()`, through a lens     | world_geometry + world_textures |
// | chrome      | `behind()` + the auto-key      | world_geometry + world_textures |
// | blur        | `history` (the backdrop, one frame old) | previous_frame         |
// | lights      | nothing                        | —                               |
// | vignette    | nothing                        | —                               |
//
// COST, STATED PLAINLY. `behind()` walks the drawables below the one being looked
// through, and frost calls it once per tap. The auto-key probes each drawable's
// texture five times. With a few windows that is tens of samples on the pixels
// inside a window and nothing anywhere else; with fifty stacked windows it is
// not free. This bundle is a demonstration of the full surface, not a template
// for a cheap effect — `mp-levels` is that.
//
// The parallax is evaluated ONCE, at the pixel centre, and reused across frost's
// taps. It is a smooth field, so blurring it is very nearly a no-op; evaluating
// it per tap would cost forty noise hashes each and change nothing you can see.
//
// @prop frost    float default=0.0 min=0.0 max=1.0 step=0.01 label="Frost"         group="Through the window"
// @prop glass    float default=0.0 min=0.0 max=1.0 step=0.01 label="Glass"         group="Through the window"
// @prop chrome   float default=0.0 min=0.0 max=1.0 step=0.01 label="Transparency"  group="Through the window"
// @prop key_tol  float default=0.13 min=0.02 max=0.60 step=0.01 label="Key tolerance" group="Through the window"

enable wgpu_binding_array;


struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;

/// The world set. Layout is fixed by the engine's UBO — rects, then srcs, then
/// the per-entry attributes — so all three are declared even where only some are
/// read; dropping one would shift the others' offsets. `attrs`, not `meta`:
/// `meta` is a reserved WGSL keyword.
///
/// Whole-band membership: `windows: "world"` hands over the entire band, so
/// iced-world panels are in here too, kind-tagged in `attrs.x`. `behind()` needs
/// that — handed client windows only, a panel stacked between two windows would
/// not be in the set and the rebuild would show the wrong thing through the
/// glass. The `window_*` requirements name the BINDINGS; the ownership mode is
/// what decides the set's membership, which is why declaring `world_*` alongside
/// an ownership mode is refused.
struct Windows {
    count: u32,
    _pad: vec3<u32>,
    rects: array<vec4<f32>, 256>,   // xy = screen origin (uv), zw = size (uv)
    srcs: array<vec4<f32>, 256>,    // xy = texture-crop origin, zw = size
    attrs: array<vec4<f32>, 256>,   // x = kind (0 = window, 1 = panel), y = alpha
};
@group(1) @binding(0) var<uniform> windows: Windows;
@group(1) @binding(1) var win_tex: binding_array<texture_2d<f32>>;

fn covers(i: u32, p: vec2<f32>) -> bool {
    let r = windows.rects[i];
    let hi = r.xy + r.zw;
    return p.x >= r.x && p.x <= hi.x && p.y >= r.y && p.y <= hi.y;
}

/// How much `c` looks like drawable `i`'s dominant background colour: 1 = key it
/// out, 0 = keep it.
///
/// Five probes — the four corners at 8%/92%, plus the centre — and the dominant
/// is whichever CORNER has the most neighbours within `tol`. Corners land on
/// chrome, which is what should key out; the centre is a probe but never the
/// answer, so a full-bleed window cannot key its own content away.
///
/// `tb-window-chroma` uses nine probes and eighty-one comparisons for a better
/// estimate. This is the cheap version because `behind()` may call it once per
/// drawable PER BLUR TAP, and the difference on window chrome is not visible.
///
/// All `win_tex` reads are indexed by a wave-uniform value (`i` comes from a loop
/// counter at every call site) — see SHADER_PIPELINE.md §3.
fn keyness(i: u32, c: vec3<f32>, tol: f32) -> f32 {
    let s = windows.srcs[i];
    var probe: array<vec3<f32>, 5>;
    probe[0] = textureSampleLevel(win_tex[i], samp, s.xy + vec2<f32>(0.08, 0.08) * s.zw, 0.0).rgb;
    probe[1] = textureSampleLevel(win_tex[i], samp, s.xy + vec2<f32>(0.92, 0.08) * s.zw, 0.0).rgb;
    probe[2] = textureSampleLevel(win_tex[i], samp, s.xy + vec2<f32>(0.08, 0.92) * s.zw, 0.0).rgb;
    probe[3] = textureSampleLevel(win_tex[i], samp, s.xy + vec2<f32>(0.92, 0.92) * s.zw, 0.0).rgb;
    probe[4] = textureSampleLevel(win_tex[i], samp, s.xy + vec2<f32>(0.50, 0.50) * s.zw, 0.0).rgb;
    var dominant = probe[0];
    var best = -1;
    for (var a = 0u; a < 4u; a = a + 1u) {
        var votes = 0;
        for (var b = 0u; b < 5u; b = b + 1u) {
            if (distance(probe[a], probe[b]) < tol) {
                votes = votes + 1;
            }
        }
        if (votes > best) {
            best = votes;
            dominant = probe[a];
        }
    }
    // Soft shoulder so the key does not alias along an antialiased glyph edge.
    return 1.0 - smoothstep(tol * 0.5, tol, distance(c, dominant));
}

/// Composite drawable `i` over `dst` at screen point `p`. Identity when `p` is
/// outside its rect.
///
/// Textures are PREMULTIPLIED, so "over" is `src.rgb + dst*(1 - src.a)`, and
/// scaling the alpha means scaling both lanes by the same factor.
///
/// A PANEL (`attrs.x` >= 0.5) is never keyed: chroma-keying a placeholder would
/// dissolve it against its own flat fill, which is the one case where the key is
/// certainly wrong.
fn over(dst: vec3<f32>, i: u32, p: vec2<f32>, see: f32, tol: f32) -> vec3<f32> {
    if (!covers(i, p)) {
        return dst;
    }
    let r = windows.rects[i];
    let s = windows.srcs[i];
    let local = (p - r.xy) / max(r.zw, vec2<f32>(0.0001));
    let px = textureSampleLevel(win_tex[i], samp, s.xy + local * s.zw, 0.0);
    var m = windows.attrs[i].y;
    if (see > 0.0 && windows.attrs[i].x < 0.5) {
        m = m * (1.0 - keyness(i, px.rgb, tol) * see);
    }
    return px.rgb * m + dst * (1.0 - px.a * m);
}

/// Everything BELOW drawable `top`, composited over `bg` at `p`.
///
/// This is the whole point of the bundle: the exact colour that the drawable at
/// `top` is sitting on, which no engine-provided image contains.
fn behind(p: vec2<f32>, top: u32, bg: vec3<f32>, see: f32, tol: f32) -> vec3<f32> {
    var acc = bg;
    for (var i = 0u; i < top; i = i + 1u) {
        acc = over(acc, i, p, see, tol);
    }
    return acc;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;

    let p_frost = pc.params[0].x;
    let p_glass = pc.params[0].y;
    let p_chrome = pc.params[0].z;
    let p_tol = pc.params[0].w;

    // The front-most drawable covering this pixel: the one being looked THROUGH.
    // Walked in reverse because the published set is back-to-front — the same
    // rule the pointer warp applies, so the two agree about which window a pixel
    // belongs to.
    var top = -1;
    let n = min(windows.count, 256u);
    for (var k = 0u; k < n; k = k + 1u) {
        let i = n - 1u - k;
        if (covers(i, uv)) {
            top = i32(i);
            break;
        }
    }

    var col = textureSampleLevel(scene, samp, uv, 0.0).rgb;

    // THE WINDOW SECTION — and under `windows: "world"` it is not optional. The
    // engine drew no part of the band, so a pixel any drawable covers gets its
    // colour from here or from nowhere; knobs at 0 mean a plain composite, not a
    // skipped one. (While this bundle was `windows: "engine"` the whole branch
    // could be skipped at 0, because the engine had already drawn the band.)
    //
    // `behind()` puts everything below `top` over the backdrop and the `over()`
    // below adds `top` itself, so between them every covering drawable is drawn.
    if (top >= 0) {
        let ti = u32(top);
        let r = windows.rects[ti];
        // The backdrop the engine actually composited: `content` with the world
        // band left out of it, which is what `windows: "world"` buys. Sampled at
        // the pixel and shared across frost's taps — see the header.
        let bg = textureSampleLevel(scene, samp, uv, 0.0).rgb;

        // GLASS — a lens that grows toward the window's edges, so what is behind
        // bends as it passes the frame. In window-local units, then scaled by the
        // rect so a small window bends as much as a large one at the same knob.
        var bp = uv;
        if (p_glass > 0.0) {
            let ld = (uv - r.xy) / max(r.zw, vec2<f32>(0.0001)) - vec2<f32>(0.5);
            bp = uv + ld * dot(ld, ld) * 4.0 * p_glass * 0.06 * r.zw;
        }

        // FROST — average the backdrop over a five-tap cross. Five, not nine:
        // each tap re-composites everything below this window.
        var back: vec3<f32>;
        if (p_frost > 0.0) {
            let e = p_frost * 6.0 / res.y;
            back = behind(bp, ti, bg, p_chrome, p_tol);
            back = back + behind(bp + vec2<f32>(e, 0.0), ti, bg, p_chrome, p_tol);
            back = back + behind(bp - vec2<f32>(e, 0.0), ti, bg, p_chrome, p_tol);
            back = back + behind(bp + vec2<f32>(0.0, e), ti, bg, p_chrome, p_tol);
            back = back + behind(bp - vec2<f32>(0.0, e), ti, bg, p_chrome, p_tol);
            back = back / 5.0;
        } else {
            back = behind(bp, ti, bg, p_chrome, p_tol);
        }

        // …and the window itself back over it, keyed. At `chrome` 0 this is an
        // ordinary opaque composite, so frost and glass alone still show through
        // whatever transparency the window already had.
        col = over(back, ti, uv, p_chrome, p_tol);
    }

    // No sRGB encode and no `lock_alpha` here: this writes the LINEAR
    // intermediate that `finish` blurs and tone-maps. Encoding twice would
    // gamma-correct the picture and then correct it again.
    return vec4<f32>(col, 1.0);
}
