//! The baked map must agree with the function it was baked from.
//!
//! That is the whole contract: the map is an optimisation, so any place it
//! disagrees with pointwise evaluation is a place the cursor moves when a bundle
//! author flips `hit.evaluate` — which would make the two modes different effects
//! rather than two ways of serving one.

use compositor_pipeline_host_hit_base::hit::{parse, Args};
use compositor_pipeline_host_map_base::map::{Map, EDGE};

/// The shipped CRT barrel, verbatim.
const BARREL: &str = r#"
fn hit_inverse(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>) -> vec2<f32> {
    let k = params.x;
    let a = vec2<f32>(res.x / max(res.y, 1.0), 1.0);
    let c = (uv - vec2<f32>(0.5, 0.5)) * a;
    let r2 = dot(c, c);
    return vec2<f32>(0.5, 0.5) + c * (1.0 + k * r2) / a;
}
"#;

const RES: [f32; 2] = [1920.0, 1080.0];
const PARAMS: [f32; 4] = [0.18, 0.0, 0.0, 0.0];

/// Sampling the map agrees with evaluating the function, to within the
/// interpolation error a smooth field admits.
///
/// The tolerance is the claim: bilinear error on a continuous field is bounded by
/// curvature × cell size. At 128 cells and the shipped curvature that is well under
/// a thousandth of the screen — sub-pixel at any real resolution.
#[test]
fn the_map_agrees_with_pointwise_evaluation() {
    let w = parse(BARREL).expect("barrel parses");
    let m = Map::bake(&w, RES, PARAMS).expect("bakes");
    let args = Args { res: RES, params: PARAMS, time: 0.0 };
    let mut worst = 0.0f64;
    // Deliberately off the grid nodes — on them the map is exact by construction
    // and would prove nothing about the interpolation.
    for i in 0..37 {
        for j in 0..37 {
            let (u, v) = (0.0137 + i as f64 / 38.0, 0.0271 + j as f64 / 38.0);
            let (eu, ev) = w.eval((u, v), args).expect("evaluates");
            let (mu, mv) = m.sample(u, v);
            worst = worst.max((eu - mu).abs()).max((ev - mv).abs());
        }
    }
    assert!(worst < 1e-3, "map drifts from the function by {worst}");
}

/// Grid nodes are exact — nothing is interpolated there, so any error would be a
/// bug in the indexing rather than in the sampling.
#[test]
fn grid_nodes_are_exact() {
    let w = parse(BARREL).expect("barrel parses");
    let m = Map::bake(&w, RES, PARAMS).expect("bakes");
    let args = Args { res: RES, params: PARAMS, time: 0.0 };
    let last = (EDGE - 1) as f64;
    for &(x, y) in &[(0usize, 0usize), (EDGE - 1, 0), (0, EDGE - 1), (EDGE - 1, EDGE - 1), (64, 31)] {
        let (u, v) = (x as f64 / last, y as f64 / last);
        let (eu, ev) = w.eval((u, v), args).unwrap();
        let (mu, mv) = m.sample(u, v);
        assert!((eu - mu).abs() < 1e-6 && (ev - mv).abs() < 1e-6, "node ({x},{y})");
    }
}

/// Staleness is the map's only failure mode, so it has to be detectable. Both
/// inputs count: a resolution change rebuilds the aspect correction, and a prop
/// change rebuilds the curve.
#[test]
fn a_map_knows_when_its_inputs_moved() {
    let w = parse(BARREL).expect("barrel parses");
    let m = Map::bake(&w, RES, PARAMS).expect("bakes");
    assert!(m.matches(RES, PARAMS));
    assert!(!m.matches([1280.0, 720.0], PARAMS), "a mode switch must invalidate it");
    assert!(!m.matches(RES, [0.4, 0.0, 0.0, 0.0]), "a prop edit must invalidate it");
}

/// A bundle with only a per-drawable entry has nothing to bake, and must say so
/// rather than hand back an empty grid that would silently flatten the pointer.
#[test]
fn a_bundle_with_no_griddable_entry_bakes_nothing() {
    const DRAWABLE_ONLY: &str = r#"
fn hit_drawable(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>,
                rect: vec4<f32>, attrs: vec4<f32>) -> vec3<f32> {
    return vec3<f32>(uv, 0.0);
}
"#;
    let w = parse(DRAWABLE_ONLY).expect("parses");
    assert!(!w.has_map() && w.has_drawable());
    assert!(Map::bake(&w, RES, PARAMS).is_none());
}
