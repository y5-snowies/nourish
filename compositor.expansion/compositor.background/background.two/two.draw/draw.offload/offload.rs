//! Whether a bundle runs off-thread this frame — and saying so when it stops.
//!
//! Split out of the render element because it is a decision, not a way of drawing:
//! it reads a compiled bundle and the live world set, returns a verdict, and the
//! element merely obeys.
//!
//! # The two kinds of "no"
//!
//! Everything else a bundle can be refused is settled at LOAD and refuses the
//! bundle outright (`place::Placement::denied`) — a shader cannot dictate something
//! it does not get. The condition here differs in kind: it is a property of what
//! happens to be on screen, it flips both ways during a session, and the bundle is
//! entirely correct. So it must not kill the bundle — and must not be silent,
//! which is what it was.

use compositor_pipeline_build_pipeline_base::pipeline::CompiledPipeline;
use compositor_pipeline_build_place_base::place::Offload;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

/// Whether the off-thread worker can reproduce what this bundle would draw.
///
/// The worker renders ONE fullscreen pass. A multipass bundle handed to it comes
/// back as a single pass — or, for a bundle with no single-pass source, as the
/// STOCK background — with no error anywhere. That is the silent wrong-background
/// bug: pick a `pipeline.json` shader with triple buffering on and get something
/// else entirely. Only a fully offloadable graph travels.
///
pub fn worker_can_render(
    pipeline: Option<&CompiledPipeline>,
    set: Option<&compositor_pipeline_abi_worldset_base::base::WorldSet>,
) -> bool {
    let Some(cp) = pipeline else { return true };
    if cp.offload != Offload::Whole {
        return false;
    }
    // Window textures need every drawable the shader is handed to be reachable by
    // a second device. A SHM surface uploaded into a device-local image has no fd
    // unless an exporting bundle was already selected, so this is a property of
    // what is on screen RIGHT NOW: one unimportable surface moves the bundle back
    // inline, and that surface going away lets it offload again. A partial set is
    // not an option — the shader would sample whichever stale descriptor was last
    // written for the missing slot. Asked for THIS bundle's ownership mode: a
    // `pipeline` bundle never receives the iced-world panels.
    if cp.requires.textures() {
        // `None` is NOT a refusal — the renderer has simply not collected a set
        // for this world yet, which is every bundle's first frames. Stay inline
        // until one arrives without tripping the latch: that line reports a real
        // stall, and firing it at every load would make it worthless.
        let Some(set) = set else { return false };
        let ok = set.all_importable(cp.owns.into());
        note_textures_unavailable(!ok);
        return ok;
    }
    true
}

/// Stage 4: this bundle's AFTER band runs on the worker, so the compositor keeps
/// drawing the head band inline and presents the worker's decorated result instead
/// of running the after pass itself.
///
/// Distinct from [`worker_can_render`], which asks whether the worker replaces the
/// bundle entirely. Here it replaces neither — it decorates what the compositor
/// composited.
pub fn offloads_after_band(offthread: bool, pipeline: Option<&CompiledPipeline>) -> bool {
    offthread && pipeline.is_some_and(|cp| cp.offload == Offload::AfterBand)
}

static TEXTURES_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

/// Say it ONCE, on each transition.
///
/// Per-transition and never per frame: at 60fps a per-frame line is not a report,
/// it is a way to lose the log. Before this there was no line at all — the
/// background flipped to the inline path mid-session, both ways, with nothing
/// recording it. That is exactly the stall the worker exists to prevent, arriving
/// invisibly.
fn note_textures_unavailable(now: bool) {
    if TEXTURES_UNAVAILABLE.swap(now, Relaxed) == now {
        return;
    }
    if now {
        warn!(
            "background.pipeline: window textures unavailable — a surface on screen \
             cannot be shared with the worker; drawing the background inline"
        );
    } else {
        info!("background.pipeline: window textures available again — back off-thread");
    }
}

/// Whether the active bundle is currently inline because its texture requirement
/// cannot be met. Advisory, not an error: the bundle is correct and still drawing.
pub fn textures_unavailable() -> bool {
    TEXTURES_UNAVAILABLE.load(Relaxed)
}
