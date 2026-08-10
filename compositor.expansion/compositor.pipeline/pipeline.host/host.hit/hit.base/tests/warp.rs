//! The warp contract: a bundle's own WGSL, evaluated host-side.
//!
//! These pin the thing the whole design rests on — that the function the shader
//! samples through and the function the pointer is corrected by are the SAME
//! source, and that running it on the CPU produces the same numbers the GPU
//! would. If this crate is ever swapped for a registry of named warps, that
//! guarantee is what would be lost, and these tests are what would stop failing.

use compositor_pipeline_host_hit_base::hit::{parse, Args};

/// A CRT barrel, in the form a bundle actually ships it: `#define_import_path`
/// at the top (naga_oil's, not naga's), everything by parameter, no bindings.
const BARREL: &str = r#"
#define_import_path crt::warp

fn hit_inverse(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>) -> vec2<f32> {
    let k = params.x;
    let c = uv - vec2<f32>(0.5, 0.5);
    let r2 = dot(c, c);
    return vec2<f32>(0.5, 0.5) + c * (1.0 + k * r2);
}
"#;

fn barrel_rust(u: f64, v: f64, k: f64) -> (f64, f64) {
    let (cx, cy) = (u - 0.5, v - 0.5);
    let r2 = cx * cx + cy * cy;
    (0.5 + cx * (1.0 + k * r2), 0.5 + cy * (1.0 + k * r2))
}

fn args(k: f32) -> Args {
    Args { res: [1920.0, 1080.0], params: [k, 0.0, 0.0, 0.0], time: 0.0 }
}

#[test]
fn the_interpreter_reproduces_the_shaders_arithmetic() {
    let w = parse(BARREL).expect("barrel parses");
    for &(u, v) in &[(0.5, 0.5), (0.0, 0.0), (1.0, 1.0), (0.13, 0.87), (0.62, 0.24)] {
        let got = w.eval((u, v), args(0.25)).expect("evaluates");
        let want = barrel_rust(u, v, 0.25);
        assert!(
            (got.0 - want.0).abs() < 1e-9 && (got.1 - want.1).abs() < 1e-9,
            "({u},{v}): got {got:?} want {want:?}"
        );
    }
}

/// The centre is the fixed point of any radial warp, and it is the one value a
/// broken evaluator is most likely to still get right — so it is checked
/// alongside the off-centre cases above, never instead of them.
#[test]
fn the_centre_is_a_fixed_point() {
    let w = parse(BARREL).expect("parses");
    let (u, v) = w.eval((0.5, 0.5), args(0.9)).expect("evaluates");
    assert!((u - 0.5).abs() < 1e-12 && (v - 0.5).abs() < 1e-12, "{u},{v}");
}

/// `k = 0` must be exactly identity, not approximately — a warp that is "off"
/// has to cost the pointer nothing at all.
#[test]
fn zero_curvature_is_identity() {
    let w = parse(BARREL).expect("parses");
    for &(u, v) in &[(0.0, 0.0), (0.31, 0.77), (1.0, 1.0)] {
        let got = w.eval((u, v), args(0.0)).expect("evaluates");
        assert!((got.0 - u).abs() < 1e-12 && (got.1 - v).abs() < 1e-12, "{got:?}");
    }
}

#[test]
fn branches_and_locals_evaluate() {
    const SRC: &str = r#"
fn hit_inverse(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>) -> vec2<f32> {
    var out = uv;
    if (uv.x > 0.5) {
        out = vec2<f32>(uv.x - 0.25, uv.y);
    } else {
        out = vec2<f32>(uv.x + 0.25, uv.y);
    }
    return out;
}
"#;
    let w = parse(SRC).expect("parses");
    assert!((w.eval((0.75, 0.5), args(0.0)).expect("evaluates").0 - 0.5).abs() < 1e-12);
    assert!((w.eval((0.25, 0.5), args(0.0)).expect("evaluates").0 - 0.5).abs() < 1e-12);
}

/// The load-time rejections. Each of these is something the CPU cannot do at
/// all, so the bundle must fail loudly rather than run with a pointer that is
/// wrong exactly where the effect is strongest.
#[test]
fn constructs_the_cpu_cannot_honour_are_refused_at_load() {
    const NO_ENTRY: &str = "fn other(x: f32) -> f32 { return x; }";
    const WRONG_ARITY: &str = "fn hit_inverse(uv: vec2<f32>) -> vec2<f32> { return uv; }";
    const SAMPLES: &str = r#"
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
fn hit_inverse(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>) -> vec2<f32> {
    return textureSampleLevel(t, s, uv, 0.0).xy;
}
"#;
    let e = parse(NO_ENTRY).err().expect("must be refused");
    assert!(e.contains("neither"), "a module with no entry at all names both: {e}");
    let e = parse(WRONG_ARITY).err().expect("must be refused");
    assert!(e.contains("expected"), "{e}");
    let err = parse(SAMPLES).err().expect("must be refused");
    assert!(err.contains("texture"), "{err}");
}

/// A bundle is a file a user edits, and this runs on the input path — an
/// unbounded loop must cost a pointer event, not the session.
#[test]
fn a_runaway_loop_gives_up_instead_of_hanging() {
    const SPIN: &str = r#"
fn hit_inverse(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>) -> vec2<f32> {
    var i = 0.0;
    loop {
        i = i + 0.0;
    }
    return uv;
}
"#;
    let w = parse(SPIN).expect("it parses — the guard is at evaluation, not load");
    assert!(w.eval((0.5, 0.5), args(0.0)).is_none(), "must bail, not hang");
}

/// The per-drawable entry: claims only inside its own rect, and reports where the
/// content under the cursor really is.
///
/// The engine walks the set front-to-back and takes the FIRST claim, so an entry
/// that claimed everywhere would make the topmost drawable swallow the desktop.
#[test]
fn a_drawable_entry_claims_only_its_own_rect() {
    const SRC: &str = r#"
fn hit_drawable(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>,
                rect: vec4<f32>, attrs: vec4<f32>) -> vec3<f32> {
    let local = (uv - rect.xy) / max(rect.zw, vec2<f32>(0.0001));
    if (local.x < 0.0 || local.x > 1.0 || local.y < 0.0 || local.y > 1.0) {
        return vec3<f32>(uv, 0.0);
    }
    return vec3<f32>(rect.xy + local * rect.zw, 1.0);
}
"#;
    let w = parse(SRC).expect("parses");
    assert!(w.has_drawable() && !w.has_map());
    let args = Args { res: [1920.0, 1080.0], params: [0.0; 4], time: 0.0 };
    let rect = [0.25f32, 0.25, 0.5, 0.5];
    // Inside: claimed.
    let hit = w.eval_drawable((0.5, 0.5), args, rect, [0.0; 4], 0);
    assert!(hit.is_some(), "a point inside the rect must be claimed");
    // Outside: no claim, so the engine goes on to the next drawable down.
    assert!(w.eval_drawable((0.05, 0.05), args, rect, [0.0; 4], 0).is_none());
}

/// The descriptor argument is opt-in by ARITY: a five-argument `hit_drawable` is
/// still valid and never sees it, and a six-argument one is handed the bits.
///
/// Both halves matter. The first is the shipped example (`crt-rotating-input-both`)
/// continuing to load; the second is the whole point of passing the flags rather
/// than spending a lane of `attrs` on them.
#[test]
fn the_descriptor_argument_is_opt_in_by_arity() {
    const PLAIN: &str = r#"
fn hit_drawable(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>,
                rect: vec4<f32>, attrs: vec4<f32>) -> vec3<f32> {
    return vec3<f32>(uv, 1.0);
}
"#;
    // Claims only when the window is focused — the smallest thing that cannot be
    // written without the sixth argument.
    const DESCRIPTIVE: &str = r#"
fn hit_drawable(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>,
                rect: vec4<f32>, attrs: vec4<f32>, flags: f32) -> vec3<f32> {
    if ((u32(flags) & 1u) == 0u) {
        return vec3<f32>(uv, 0.0);
    }
    return vec3<f32>(uv * 0.5, 1.0);
}
"#;
    let args = Args { res: [1920.0, 1080.0], params: [0.0; 4], time: 0.0 };
    let rect = [0.0f32, 0.0, 1.0, 1.0];

    let plain = parse(PLAIN).expect("five arguments still parse");
    assert!(!plain.descriptive());
    // Flags are passed and simply not received — the call must not arity-mismatch.
    assert!(plain.eval_drawable((0.5, 0.5), args, rect, [0.0; 4], 0b1111).is_some());

    let d = parse(DESCRIPTIVE).expect("six arguments parse");
    assert!(d.descriptive());
    assert!(
        d.eval_drawable((0.5, 0.5), args, rect, [0.0; 4], 0).is_none(),
        "unfocused: the warp declined the point, so the flags reached it as zero",
    );
    let hit = d.eval_drawable((0.5, 0.5), args, rect, [0.0; 4], 1);
    assert_eq!(hit, Some((0.25, 0.25)), "focused: the ACTIVATED bit reached the warp");
}

/// A helper shared with the render pass is FOLLOWED, not refused.
///
/// This is what makes "one definition, two readers" possible: the bundle's pass
/// and its warp `#import` the same function instead of each carrying a copy of the
/// expression, and a copy is how the picture and the cursor drift apart.
#[test]
fn a_call_into_a_shared_helper_is_followed() {
    const SRC: &str = r#"
fn half(x: f32) -> f32 { return x * 0.5; }

fn hit_inverse(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>) -> vec2<f32> {
    return vec2<f32>(half(uv.x), half(uv.y));
}
"#;
    let w = parse(SRC).expect("a shared helper must not be refused");
    let args = Args { res: [100.0, 100.0], params: [0.0; 4], time: 0.0 };
    let (u, v) = w.eval((0.8, 0.4), args).expect("evaluates through the call");
    assert!((u - 0.4).abs() < 1e-9 && (v - 0.2).abs() < 1e-9, "got ({u}, {v})");
}
