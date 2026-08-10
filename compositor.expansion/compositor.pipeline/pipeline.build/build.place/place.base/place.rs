//! Resolve each pass to a site and report how much of the graph the worker can
//! take. A pure function of the manifest + plan — no device, no worker, no GPU,
//! so it is fully unit-testable. Capability gates (dmabuf export, descriptor
//! indexing) belong to the caller, which applies the verdict and logs `notes`.
//! See `document/SHADER_PIPELINE_WORKER.md` stage 1.

use compositor_pipeline_bundle_graph_base::graph::Plan;
use compositor_pipeline_bundle_manifest_base::manifest::{Manifest, Place, When};

/// Where a pass ends up once `Place::Auto` is resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site { Worker, Compositor }

/// How much of the graph the worker can take.
///
/// `Whole` — every pass. `BeforeBand` — a leading worker band publishes the
/// background and the rest runs inline. `AfterBand` — the INVERSE: the compositor
/// composites background and windows, and the worker decorates the result
/// (stage 4). `None` — all inline.
///
/// The two banded shapes are opposite ends of the graph, which is why placement
/// cannot simply be "a contiguous run": the worker takes the head or the tail, and
/// which one changes what the compositor presents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offload { Whole, BeforeBand, AfterBand, None }

/// What the machine and this session can actually offer a graph.
///
/// An argument rather than something read here, so `place()` stays a pure function
/// of its inputs and stays unit-testable without a compositor. The caller resolves
/// it once (`load_multipass`) from the live environment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Env {
    /// The off-thread worker can exist: triple buffering is on AND the compositor
    /// is on Vulkan (`background_base::engaged()`).
    ///
    /// Placement used to be blind to this, which made its verdict a statement
    /// about the manifest rather than about what would happen: with triple
    /// buffering off it still reported `offload=Whole` while nothing was offloaded
    /// and nothing said so.
    pub worker: bool,
}

/// The resolved placement.
///
/// `notes` are advisory — an `auto` pass resolving inline is the design working,
/// and reporting it would make every ordinary bundle look downgraded.
///
/// `denied` is the opposite and is kept separate for exactly that reason: a pass
/// wrote `place: "worker"`, and it is not getting it. A shader cannot dictate
/// something it does not get, so the caller REFUSES the bundle rather than running
/// it in a shape its author did not ask for and cannot see. Structured rather than
/// grepped out of `notes`, so the distinction survives an edit to the wording.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub sites: Vec<Site>,
    pub offload: Offload,
    pub notes: Vec<String>,
    pub denied: Vec<String>,
}

/// Worker-eligible only if every input the pass needs can reach the worker.
///
/// A before-content pass may sample nothing post-composite — there is no
/// composited anything at that point in the frame, on either side.
///
/// An after-content pass is NOT worker-eligible. This is the single line that
/// switches stage 4 — the worker taking a graph's TAIL — off.
///
/// The transport for it exists and works in isolation: `content` and the
/// `windows` layer are exported, `history` is derived worker-side, and
/// `ContentPath` can present the decorated band the worker sends back. What does
/// not work is the contract underneath.
///
/// Every other offload hands the compositor a BACKDROP. Late, stale, missing or
/// torn, the desktop is still drawn — so the ring's design is allowed to be
/// approximate, and it is: an ack handshake that assumes the compositor draws for
/// its own reasons, a rate cap sized for wallpaper, a two-deep ring, and a single
/// `content` image shared across devices with no layout agreement. A stage-4 band
/// is the PICTURE, and each of those approximations becomes a blank or frozen
/// screen instead of a stale backdrop. They were found and fixed one at a time,
/// and each fix surfaced the next — which is the signature of a contract
/// mismatch, not of a queue of bugs.
///
/// Re-enabling means giving the worker a transport designed to carry a FRAME:
/// completion-before-publish on `content` as well as on the band, a ring deep
/// enough for both directions, pacing that is not the wallpaper's, and a
/// compositor that never depends on the worker to have a picture at all. See
/// `SHADER_PIPELINE_WORKER.md`. Until then an after pass runs inline, which is
/// what every bundle did before stage 4 and what they all still do correctly.
fn eligible(m: &Manifest, i: usize) -> bool {
    let p = &m.passes[i];
    if p.when == When::AfterContent {
        return false;
    }
    let post = |t: &String| matches!(t.as_str(), "content" | "windows" | "history");
    // Both window interfaces now cross. Geometry is numbers; textures are client
    // dmabufs the worker imports itself, duping the fd, so neither shares a
    // lifetime with the compositor. What CANNOT cross is a SHM surface — it has no
    // fd — but that is a per-frame property of the live desktop, not of the
    // manifest, so it is decided at dispatch (`worker_can_render`) rather than here.
    !p.inputs.values().any(post)
}

/// Resolve placement for every pass of `m` under `plan`, given what `env` can
/// actually offer.
pub fn place(m: &Manifest, plan: &Plan, env: Env) -> Placement {
    let order: Vec<usize> = plan.before.iter().chain(plan.after.iter()).copied().collect();
    let mut sites = vec![Site::Compositor; m.passes.len()];
    let mut notes = Vec::new();
    let mut denied = Vec::new();

    // No worker in this session: nothing can be placed on one, and a pass that
    // ASKED is being refused something. Reported as a denial rather than a note
    // because the fix is the user's (turn triple buffering on, restart) and
    // because the alternative — running the graph inline while the load line
    // claims it offloaded — is the exact invisibility this split exists to end.
    if !env.worker {
        for &i in &order {
            if m.passes[i].place == Place::Worker {
                let n = &m.passes[i].name;
                denied.push(format!(
                    "pass '{n}': asks for the worker, but background triple buffering is                      off (or this session is not on Vulkan)"
                ));
            }
        }
        return Placement { sites, offload: Offload::None, notes, denied };
    }

    for &i in &order {
        let ok = eligible(m, i);
        // `Auto` never sends an AFTER pass to the worker. Doing so would offload
        // the decorate band by default and leave a heavy background inline, and it
        // would silently reclassify every bundle written before stage 4 existed.
        // The tail is taken only on request.
        let auto_ok = ok && m.passes[i].when == When::BeforeContent;
        sites[i] = match m.passes[i].place {
            Place::Compositor => Site::Compositor,
            Place::Auto => if auto_ok { Site::Worker } else { Site::Compositor },
            Place::Worker if ok => Site::Worker,
            Place::Worker => {
                let n = &m.passes[i].name;
                denied.push(format!(
                    "pass '{n}': asks for the worker, but reads post-composite data                      (`content`/`windows`/`history`) or runs after content"
                ));
                Site::Compositor
            }
        };
    }

    // The worker takes the HEAD or the TAIL, never a middle and never both.
    //
    // Head (`BeforeBand`/`Whole`): it publishes the background and the compositor
    // composites windows over it. Tail (`AfterBand`, stage 4): the compositor
    // composites background AND windows, and the worker decorates the result. The
    // two are opposite ends and imply different things about what the compositor
    // presents, so a graph that wants both would round-trip twice — three-plus
    // frames — and is coalesced to one end instead.
    //
    // Which end is OPT-IN, not inferred. Defaulting to the tail would send a
    // trivial after pass to the worker while leaving a heavy background inline —
    // exactly backwards — and the manifest cannot know which band is expensive.
    // The author can, so the tail is taken only when the after band asks for it
    // with `place: "worker"`. Everything else keeps the head, which is the
    // behaviour every bundle had before stage 4 existed.
    //
    // Unreachable while `eligible` refuses after-content passes (see there). Kept
    // whole rather than deleted: the tail model is correct, and what it waits on
    // is a transport, not a fix here.
    let after_eligible = !plan.after.is_empty()
        && plan.after.iter().all(|&i| {
            sites[i] == Site::Worker && m.passes[i].place == Place::Worker
        });
    if after_eligible {
        for &i in &plan.before {
            if sites[i] == Site::Worker {
                // Only a pass that ASKED for the worker is being denied something.
                // An `auto` head pass moving inline is the resolution working as
                // designed, and noting it would make every stage-4 bundle look
                // like it had been downgraded.
                if m.passes[i].place == Place::Worker {
                    let n = &m.passes[i].name;
                    denied.push(format!(
                        "pass '{n}': asks for the worker, but the worker is taking this \
                         graph's after band — taking both ends would round-trip twice"
                    ));
                }
                sites[i] = Site::Compositor;
            }
        }
    } else {
        let mut inline_seen = false;
        for &i in &order {
            if sites[i] == Site::Compositor {
                inline_seen = true;
            } else if inline_seen {
                let n = &m.passes[i].name;
                let msg = format!(
                    "pass '{n}': demoted — worker passes must run contiguously from the start"
                );
                // Only a pass that ASKED is being denied. An `auto` pass landing
                // inline is the resolution working.
                if m.passes[i].place == Place::Worker { denied.push(msg) } else { notes.push(msg) }
                sites[i] = Site::Compositor;
            }
        }
    }

    // Closure, in whichever direction the split runs. A worker pass may only read
    // targets produced on ITS side, and the band must end by writing `output`,
    // since the worker publishes exactly one image.
    let band: Vec<usize> = order.iter().copied().filter(|&i| sites[i] == Site::Worker).collect();
    if let Some(&last) = band.last() {
        let outs: Vec<&str> = band.iter().map(|&i| m.passes[i].output.as_str()).collect();
        let reason = if m.passes[last].output != "output" {
            Some("worker band never writes `output` — it would have nothing to publish")
        } else if after_eligible {
            // TAIL: the mirror of the head case. An intermediate produced by the
            // inline head lives on the compositor's device, so a worker pass
            // reading one would sample a target that is not there.
            let ins: Vec<&str> = band
                .iter()
                .flat_map(|&i| m.passes[i].inputs.values().map(|t| t.as_str()))
                .collect();
            let from_inline = order.iter().any(|&i| {
                sites[i] == Site::Compositor
                    && ins.contains(&m.passes[i].output.as_str())
                    && m.passes[i].output != "output"
            });
            from_inline.then_some(
                "worker band reads an intermediate the inline passes produce — it never leaves \
                 the compositor's device",
            )
        } else {
            // HEAD: nothing left inline may read what the band kept on its device.
            let leaks = order.iter().any(|&i| {
                sites[i] == Site::Compositor
                    && m.passes[i].inputs.values().any(|t| outs.contains(&t.as_str()))
            });
            leaks.then_some(
                "worker band feeds an inline pass — its targets never leave the worker device",
            )
        };
        if let Some(r) = reason {
            // The whole band goes inline, so anyone in it who asked is denied.
            if band.iter().any(|&i| m.passes[i].place == Place::Worker) {
                denied.push(r.to_string());
            } else {
                notes.push(r.to_string());
            }
            band.iter().for_each(|&i| sites[i] = Site::Compositor);
        }
    }

    let after_eligible = after_eligible && plan.after.iter().all(|&i| sites[i] == Site::Worker);
    let workers = sites.iter().filter(|s| **s == Site::Worker).count();
    let offload = if workers == 0 {
        Offload::None
    } else if workers == m.passes.len() {
        Offload::Whole
    } else if after_eligible {
        Offload::AfterBand
    } else {
        Offload::BeforeBand
    };
    Placement { sites, offload, notes, denied }
}
