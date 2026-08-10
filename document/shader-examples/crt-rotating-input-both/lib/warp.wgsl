// BOTH ENTRY KINDS OF THE POINTER WARP, IN ONE BUNDLE.
//
// The chain has two entries here and they are undone in reverse of how the
// picture was built — last pass first:
//
//   1. `hit_inverse`  — the CRT barrel, laid over the whole band by the LAST pass.
//                       A pure function of position, so the engine BAKES it into a
//                       grid and the pointer costs a lookup (`hit.evaluate: map`).
//   2. `hit_drawable` — the per-window sway, applied by the world pass BEFORE the
//                       barrel. Discontinuous at every window edge, so no grid
//                       represents it: interpolating across the jump is not
//                       imprecise, it is wrong. Evaluated pointwise instead.
//
// Neither kind subsumes the other, which is the whole reason this bundle exists.
// A grid cannot hold entry 2; evaluating entry 1 per event re-derives the same
// smooth field thousands of times a second.
//
// `sway` is shared with the render pass, so the rotation the pointer removes is
// the rotation the shader applied — not a second implementation of it.
#define_import_path crt_both::warp

/// The per-window angle. ONE definition, imported by both readers.
fn sway(tilt: f32, t: f32, index: f32) -> f32 {
    return tilt * sin(t * 0.6 + index * 1.7);
}

/// Entry 1 — the fullscreen barrel. Griddable: position in, position out.
fn hit_inverse(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>) -> vec2<f32> {
    let k = params.x;
    let a = vec2<f32>(res.x / max(res.y, 1.0), 1.0);
    let c = (uv - vec2<f32>(0.5, 0.5)) * a;
    let r2 = dot(c, c);
    return vec2<f32>(0.5, 0.5) + c * (1.0 + k * r2) / a;
}

/// Entry 2 — one drawable's sway, undone.
///
/// `rect` is its screen-UV rect; `attrs` is `vec4(index, time, kind, alpha)`. The
/// engine walks the world set front-to-back and takes the first claim, so this
/// answers only about the drawable it was handed and never has to know the z
/// order. Returns `vec3(source_uv, claimed)`.
fn hit_drawable(
    uv: vec2<f32>,
    res: vec2<f32>,
    params: vec4<f32>,
    rect: vec4<f32>,
    attrs: vec4<f32>,
) -> vec3<f32> {
    // A panel is drawn axis-aligned by the pass, so it is not displaced and must
    // not be corrected — but it DOES still occupy its rect, so it claims the point
    // unchanged rather than falling through to a window behind it.
    let is_panel = attrs.z > 0.5;
    let aspect = vec2<f32>(res.x / max(res.y, 1.0), 1.0);
    let centre = rect.xy + rect.zw * 0.5;
    // The pass rotates by `-a` to read its source; undoing that is the same
    // expression, which is why the angle is shared rather than re-derived.
    let a = select(sway(params.y, attrs.y, attrs.x), 0.0, is_panel);
    let ca = cos(-a);
    let sa = sin(-a);
    let p = (uv - centre) * aspect;
    let rp = vec2<f32>(p.x * ca - p.y * sa, p.x * sa + p.y * ca) / aspect;
    let local = rp / max(rect.zw, vec2<f32>(0.0001)) + vec2<f32>(0.5);
    // Outside this drawable: no claim, the engine tries the next one down.
    if (local.x < 0.0 || local.x > 1.0 || local.y < 0.0 || local.y > 1.0) {
        return vec3<f32>(uv, 0.0);
    }
    // Claimed. Report where the content under the cursor actually lives: the
    // window's un-rotated position on screen.
    return vec3<f32>(rect.xy + local * rect.zw, 1.0);
}
