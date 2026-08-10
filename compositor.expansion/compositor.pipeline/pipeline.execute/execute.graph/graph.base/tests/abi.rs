//! The world-set ABI is split across three places that must agree, and nothing
//! but this test makes them.
//!
//! `MAX_WORLD_ENTRIES` fixes the byte offsets of `srcs` and `attrs` inside the
//! UBO. Every bundle that declares `struct Windows` restates that length in WGSL,
//! because a uniform array's length must be a compile-time constant and bundles
//! are standalone files with no shared header. Get them out of step and nothing
//! errors: the shader reads `srcs` from where its own arithmetic says they are,
//! finds rect data, and paints every window with garbage crop coordinates. That
//! is the exact class of bug the 32-byte-header comment in `graph.rs` records
//! having already cost this feature once.
//!
//! Skips when the examples tree is absent (vendored builds).

use compositor_pipeline_execute_graph_base::graph::{MAX_WORLD_ENTRIES, TIMES_OFFSET};
use std::path::PathBuf;

fn examples() -> Option<PathBuf> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../../document/shader-examples");
    d.is_dir().then_some(d)
}

/// The CPU-side publish cap and the shader-side array length are the same number
/// by definition — one is what the other can hold.
#[test]
fn the_publish_cap_matches_the_uniform_array_length() {
    assert_eq!(compositor_pipeline_abi_worldset_base::base::MAX, MAX_WORLD_ENTRIES);
}

/// A uniform buffer is only guaranteed to be bindable up to 16 KiB
/// (`maxUniformBufferRange`'s required minimum). Three `vec4` arrays plus the
/// 32-byte header is what sets the ceiling on `MAX_WORLD_ENTRIES`, so raising it
/// past that has to become a storage buffer rather than a bigger number.
#[test]
fn the_ubo_fits_the_guaranteed_uniform_range() {
    let bytes = 32 + 3 * MAX_WORLD_ENTRIES * 16;
    assert!(bytes <= 16384, "world UBO is {bytes} bytes; the guaranteed minimum range is 16384");
}

/// The `Times` block (`@group(1) @binding(2)`) is a second UBO with the same
/// hazard: its three arrays are `MAX_WORLD_ENTRIES` long, a bundle restates that
/// length, and getting it wrong reads `state` out of the middle of `life` — every
/// window animating against another window's moments, with nothing to say why.
///
/// It sits in the same buffer as the geometry at a fixed offset, so its length is
/// also what keeps the two blocks from overlapping.
#[test]
fn every_bundle_declares_the_engine_times_length() {
    let Some(root) = examples() else { return };
    let want = format!("array<vec4<f32>, {MAX_WORLD_ENTRIES}>");
    for dir in std::fs::read_dir(&root).expect("readable") {
        let passes = dir.expect("entry").path().join("passes");
        if !passes.is_dir() {
            continue;
        }
        for f in std::fs::read_dir(&passes).expect("readable") {
            let f = f.expect("entry").path();
            if f.extension().is_none_or(|e| e != "wgsl") {
                continue;
            }
            let src = std::fs::read_to_string(&f).expect("readable");
            let Some(start) = src.find("struct Times") else { continue };
            let body = &src[start..src[start..].find('}').map_or(src.len(), |e| start + e)];
            for line in body.lines().filter(|l| l.contains("array<vec4<f32>,")) {
                assert!(
                    line.contains(&want),
                    "{}: `{}` disagrees with MAX_WORLD_ENTRIES ({MAX_WORLD_ENTRIES}) — \
                     `state` would be read out of the middle of `life`",
                    f.display(),
                    line.trim(),
                );
            }
        }
    }
}

/// Both UBO ranges fit the guaranteed uniform range INDEPENDENTLY. They share one
/// allocation, so the buffer is larger than 16 KiB and only the per-binding ranges
/// have to fit — which is exactly why the timestamps could be added without
/// shrinking the entry count that every shipped bundle restates.
#[test]
fn each_bound_range_fits_the_guaranteed_uniform_range() {
    let geometry = 32 + 3 * MAX_WORLD_ENTRIES * 16;
    let times = 32 + 3 * MAX_WORLD_ENTRIES * 16;
    assert!(geometry <= 16384, "geometry range is {geometry} bytes");
    assert!(times <= 16384, "times range is {times} bytes");
    // …and the times block starts at an offset every implementation can bind:
    // 256 is the largest `minUniformBufferOffsetAlignment` in practice. The
    // geometry block is 12320 bytes — 32 past a multiple of 256 — so this holds
    // only because the offset is explicitly rounded up, and it caught the version
    // that was not.
    assert_eq!(TIMES_OFFSET % 256, 0, "the times block starts at an unbindable offset");
    assert!(TIMES_OFFSET >= geometry as u64, "the times block overlaps the geometry block");
}

#[test]
fn every_bundle_declares_the_engine_array_length() {
    let Some(root) = examples() else { return };
    let want = format!("array<vec4<f32>, {MAX_WORLD_ENTRIES}>");
    let mut checked = 0;
    for dir in std::fs::read_dir(&root).expect("readable") {
        let passes = dir.expect("entry").path().join("passes");
        if !passes.is_dir() {
            continue;
        }
        for f in std::fs::read_dir(&passes).expect("readable") {
            let f = f.expect("entry").path();
            if f.extension().is_none_or(|e| e != "wgsl") {
                continue;
            }
            let src = std::fs::read_to_string(&f).expect("readable");
            let Some(start) = src.find("struct Windows") else { continue };
            let body = &src[start..src[start..].find('}').map_or(src.len(), |e| start + e)];
            for line in body.lines().filter(|l| l.contains("array<vec4<f32>,")) {
                assert!(
                    line.contains(&want),
                    "{}: `{}` disagrees with MAX_WORLD_ENTRIES ({MAX_WORLD_ENTRIES}) — \
                     the arrays after it would be read at the wrong offset",
                    f.display(),
                    line.trim(),
                );
            }
            // The count clamp is the other half: a bundle clamping to the OLD
            // capacity silently stops drawing everything past it.
            assert!(
                !src.contains("min(windows.count,") || src.contains(&format!("min(windows.count, {MAX_WORLD_ENTRIES}u)")),
                "{}: clamps `windows.count` to something other than {MAX_WORLD_ENTRIES}",
                f.display(),
            );
            checked += 1;
        }
    }
    assert!(checked >= 10, "only {checked} bundle shaders declare `struct Windows`; expected the family");
}
