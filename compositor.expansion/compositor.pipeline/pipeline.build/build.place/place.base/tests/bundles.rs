//! The shipped `tb-*` stress bundles must keep the offload class the plan claims
//! for them (`document/SHADER_PIPELINE_WORKER.md` §7). Editing a bundle so it
//! silently stops being worker-eligible is exactly the regression this catches.
//!
//! Reads the real manifests from `document/shader-examples/`. Skips (rather than
//! fails) when that tree is not present, so a vendored build never breaks on it.

use compositor_pipeline_bundle_graph_base::graph::plan;
use compositor_pipeline_bundle_manifest_base::manifest::{parse, Place, When};
use compositor_pipeline_build_place_base::place::{place, Env, Offload};
use std::path::PathBuf;

/// `document/shader-examples`, five levels above this crate.
fn examples() -> Option<PathBuf> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../document/shader-examples");
    d.is_dir().then(|| d)
}

const EXPECTED: &[(&str, Offload)] = &[
    // Closed before-only graphs — the bundles triple buffering can help today.
    ("tb-heavy-single", Offload::Whole),
    ("tb-chain-6", Offload::Whole),
    ("tb-wide-32f", Offload::Whole),
    ("tb-many-targets", Offload::Whole),
    ("tb-mixed-scale", Offload::Whole),
    // A worker background band + an inline after-content band.
    ("tb-after-trivial", Offload::BeforeBand),
    ("tb-after-history", Offload::BeforeBand),
    ("tb-window-rects", Offload::BeforeBand),
    ("tb-window-spotlight", Offload::BeforeBand),
    ("tb-window-tex", Offload::BeforeBand),
    ("tb-window-mirror", Offload::BeforeBand),
    ("tb-window-xray", Offload::BeforeBand),
    ("tb-velocity-probe", Offload::BeforeBand),
    ("tb-parallax-lights", Offload::BeforeBand),
    // `windows: pipeline` — one before-content pass that also composites the
    // windows, so it declares `needs` and is correctly not worker-eligible.
    ("tb-window-owned", Offload::Whole),
    ("tb-window-probe", Offload::BeforeBand),
    // Negative control: claims window compositing, draws none. Declares `needs`,
    // so it is correctly not worker-eligible.
    ("tb-window-none", Offload::Whole),
    // Auto chroma key: owns window compositing, so it declares `needs`.
    ("tb-window-chroma", Offload::Whole),
    // Whole-picture tube emulation: plain backdrop + after-content pass.
    ("tb-crt", Offload::BeforeBand),
    // Backdrop off-thread; the temporal `blur` pass inline. Only the twin was
    // pinned here before, which is the half that proves least.
    ("motion-blur", Offload::BeforeBand),
    ("tb-windows-layer", Offload::BeforeBand),
    // `windows: pipeline` never offloads, so this is None despite being
    // otherwise ordinary.
    ("tb-window-frost", Offload::BeforeBand),
    // Cadence demo: both passes are before-content with no needs, so the whole
    // graph is offloadable and the held target lives on the worker device.
    ("tb-cadence", Offload::Whole),
    // Self-verifying chain: all before-content, no needs -> fully offloadable,
    // which is what makes it the reference for an offload A/B.
    ("tb-chain-proof", Offload::Whole),
    // Stage validators. 5b is the only bundle that is BOTH Whole and needs
    // window textures, so it is the only one that exercises the cross-device
    // texture path at all.
    ("tb-stage-5b", Offload::Whole),
    // Stage 4 bundles. They ASK for the tail; the tail is disabled, so their
    // after pass runs inline and only the head offloads. They keep their entry
    // here so re-enabling stage 4 shows up as a verdict change rather than as
    // silence.
    ("tb-stage-4", Offload::BeforeBand),

    ("tb-crt-offload", Offload::BeforeBand),
    // Window metaballs: one before-content pass, so the WHOLE graph offloads.
    ("tb-window-metaballs", Offload::Whole),

    ("tb-worker-history", Offload::BeforeBand),
    ("tb-worker-windows", Offload::BeforeBand),
    ("tb-stage-2", Offload::Whole),
    ("tb-stage-6", Offload::Whole),
    ("tb-stage-5a", Offload::Whole),
    ("tb-stage-3", Offload::Whole),
    ("tb-stage-w", Offload::BeforeBand),
    // Window GEOMETRY crosses to the worker, so a rects-only pass offloads.
    ("tb-rects-offload", Offload::Whole),
    // `-inline` twins: identical graphs with every pass pinned `place: compositor`,
    // so the same scene can be compared with and without the worker path. Pinned
    // means Offload::None for all of them, including the ones whose ORIGINAL is not
    // offloadable yet — those pairs are pre-built for the later stages.
    ("bloom-inline", Offload::None),
    ("glass-inline", Offload::None),
    ("motion-blur-inline", Offload::None),
    ("tb-after-history-inline", Offload::None),
    ("tb-after-trivial-inline", Offload::None),
    ("tb-chain-6-inline", Offload::None),
    ("tb-crt-inline", Offload::None),
    ("tb-heavy-single-inline", Offload::None),
    ("tb-many-targets-inline", Offload::None),
    ("tb-mixed-scale-inline", Offload::None),
    ("tb-parallax-lights-inline", Offload::None),
    ("tb-velocity-probe-inline", Offload::None),
    ("tb-wide-32f-inline", Offload::None),
    ("tb-window-chroma-inline", Offload::None),
    ("tb-window-mirror-inline", Offload::None),
    ("tb-window-none-inline", Offload::None),
    ("tb-window-owned-inline", Offload::None),
    ("tb-window-probe-inline", Offload::None),
    ("tb-window-rects-inline", Offload::None),
    ("tb-window-spotlight-inline", Offload::None),
    ("tb-window-tex-inline", Offload::None),
    ("tb-cadence-inline", Offload::None),
    ("tb-chain-proof-inline", Offload::None),
    ("tb-stage-5b-inline", Offload::None),
    ("tb-stage-4-inline", Offload::None),
    ("tb-window-metaballs-inline", Offload::None),
    ("tb-worker-history-inline", Offload::None),
    ("tb-worker-windows-inline", Offload::None),
    ("tb-rects-offload-inline", Offload::None),
    ("tb-windows-layer-inline", Offload::None),
    ("tb-window-frost-inline", Offload::None),
    ("tb-window-xray-inline", Offload::None),
    ("vignette-inline", Offload::None),
    ("window-glow-inline", Offload::None),
];

#[test]
fn shipped_bundles_keep_their_offload_class() {
    let Some(root) = examples() else { return };
    for (name, want) in EXPECTED {
        let src = std::fs::read_to_string(root.join(name).join("pipeline.json"))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let m = parse(&src).unwrap_or_else(|e| panic!("{name}: {e}"));
        let p = plan(&m).unwrap_or_else(|e| panic!("{name}: {e}"));
        let got = place(&m, &p, Env { worker: true });
        assert_eq!(got.offload, *want, "{name}: offload class changed");
        // No shipped bundle may lose a band it did not ask to lose. `notes` is
        // advisory (an `auto` pass resolving inline), so any entry here is a
        // downgrade nobody intended.
        assert!(got.notes.is_empty(), "{name}: unexpected downgrade {:?}", got.notes);
        // Denials are the loud half. Exactly one thing earns one today: asking
        // for the TAIL (`place: "worker"` on an after pass) while the tail is
        // disabled — `tb-stage-4`, `tb-crt-offload`, `tb-worker-history`,
        // `tb-worker-windows`. Those now REFUSE TO LOAD rather than running
        // inline while the log claimed an offload they were not getting.
        //
        // Derived from the manifest rather than listed, so re-enabling the tail
        // flips this test by itself and a new bundle cannot quietly join the set.
        let wants_tail = m.passes.iter().any(|p| {
            p.place == Place::Worker && p.when == When::AfterContent
        });
        assert_eq!(
            !got.denied.is_empty(),
            wants_tail,
            "{name}: denied={:?}",
            got.denied
        );
    }
}

/// Every bundle that ships a `pipeline.json` must still parse and plan — the
/// single-pass examples alongside them have no manifest and are skipped.
#[test]
fn every_shipped_manifest_parses_and_plans() {
    let Some(root) = examples() else { return };
    let mut seen = 0;
    for e in std::fs::read_dir(&root).expect("readable") {
        let p = e.expect("entry").path().join("pipeline.json");
        if !p.is_file() {
            continue;
        }
        let src = std::fs::read_to_string(&p).expect("readable");
        let m = parse(&src).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        plan(&m).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        seen += 1;
    }
    assert!(seen >= EXPECTED.len(), "found only {seen} manifests");
}
