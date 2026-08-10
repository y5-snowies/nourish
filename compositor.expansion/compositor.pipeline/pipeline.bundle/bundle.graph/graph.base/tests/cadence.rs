//! Cadence, and the one way to get it wrong that produces no error on its own.
//!
//! A target written by two passes at DIFFERENT rates is not held between frames —
//! it alternates between whatever the two passes produced. On screen that is a
//! flicker proportional to how much the target contributes, which reads as the
//! effect being unstable rather than as the graph being wrong, and is close to
//! impossible to attribute by looking at it.
//!
//! It is also entirely static: the manifest says who writes what and how often.
//! So it is refused at load, with the two pass names in the message.

use compositor_pipeline_bundle_graph_base::graph::plan;
use compositor_pipeline_bundle_manifest_base::manifest::parse;

fn manifest(passes: &str) -> String {
    format!(
        r#"{{"name":"t","version":1,
             "targets":{{"a":{{"format":"rgba16f","scale":0.5}}}},
             "passes":[{passes}]}}"#
    )
}

fn plan_of(passes: &str) -> Result<(), String> {
    let m = parse(&manifest(passes)).expect("parses");
    plan(&m).map(|_| ())
}

/// The shape that produced a real per-frame flicker: a downsample refreshing the
/// target every frame while the blur that is supposed to own it runs every other.
#[test]
fn two_rates_writing_one_target_are_refused() {
    let err = plan_of(
        r#"{"name":"fill","shader":"p.wgsl","output":"a"},
           {"name":"blur","shader":"p.wgsl","inputs":{"src":"a"},"output":"a","cadence":2},
           {"name":"out","shader":"p.wgsl","inputs":{"src":"a"},"output":"output"}"#,
    )
    .expect_err("mismatched cadences on one target must not load");
    assert!(err.contains("fill") && err.contains("blur"), "name both passes: {err}");
    assert!(err.contains('a'), "name the target: {err}");
}

/// …and the corrected form loads. Two passes may share a target as long as they
/// agree how often it is refreshed — which is the ping-pong-free case of a pass
/// producing into a target another pass then refines at the same rate.
#[test]
fn one_rate_writing_one_target_is_fine() {
    plan_of(
        r#"{"name":"fill","shader":"p.wgsl","output":"a","cadence":2},
           {"name":"blur","shader":"p.wgsl","inputs":{"src":"a"},"output":"a","cadence":2},
           {"name":"out","shader":"p.wgsl","inputs":{"src":"a"},"output":"output"}"#,
    )
    .expect("matching cadences are legal");
}

/// The default is every frame, and two default passes writing one target must
/// stay legal — that is an ordinary chain, not a mistake.
#[test]
fn the_default_cadence_does_not_trip_the_rule() {
    plan_of(
        r#"{"name":"fill","shader":"p.wgsl","output":"a"},
           {"name":"blur","shader":"p.wgsl","inputs":{"src":"a"},"output":"a"},
           {"name":"out","shader":"p.wgsl","inputs":{"src":"a"},"output":"output"}"#,
    )
    .expect("an ordinary same-rate chain is legal");
}

/// `output` is exempt: it has no cadence to disagree about (that is refused
/// separately) and several bands may write it.
#[test]
fn the_swapchain_is_exempt() {
    plan_of(
        r#"{"name":"a1","shader":"p.wgsl","output":"a","cadence":3},
           {"name":"out","shader":"p.wgsl","inputs":{"src":"a"},"output":"output"}"#,
    )
    .expect("one writer at any cadence is legal");
}
