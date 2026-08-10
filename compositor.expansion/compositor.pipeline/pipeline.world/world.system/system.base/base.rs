//! The per-world pipeline slot, and the system that maintains it.
//!
//! # Why this exists
//!
//! Everything a shader reads about the desktop — which windows exist, where they
//! are, when each of them last did something, where the pointer is — used to
//! reach the renderer through process-global slots parked in
//! `kernel.graphic/graphic.bridge`. That works exactly as long as there is one
//! world, and y5 has never had one world: the picker is a world, every
//! picker-created scene is a world, and two of them can be running different
//! bundles at the same time. A global slot serves whichever world wrote last.
//!
//! So the state lives here, in the world's own `Storage`, reached by a token.
//! The mechanism is not new — `BG_TWO` has held per-world background state all
//! along; this is the same shape for the facts the pipeline needs.
//!
//! # Liveness
//!
//! A world publishes while it is being LOOKED AT, which is not the same as being
//! active:
//!
//! * The **active** world ticks (`hooks.rs` calls `worlds.active_mut()`), so a
//!   dormant world already costs nothing and needs no gate.
//! * The **spawn target** may be dormant and still on screen — opening the picker
//!   makes the picker active while the session world's background is still drawn
//!   behind it, because the draw path falls back through `spawn_target`. Wiping on
//!   `on_disable` alone would strip the descriptors from a background that is
//!   visibly still running.
//! * The **lock screen** is the exception: nothing of the session world is
//!   visible, so it is wiped deliberately rather than kept warm.
//!
//! [`PipelineState::live`] is that answer, and a pass reads the tables only while
//! it is true. Wiping is not an optimisation — a stale table is a shader
//! animating against timestamps from the last time anyone looked at this world.

use compositor_pipeline_abi_worldset_base::base::Own;
/// Re-exported so the world's slot and the renderer name ONE type.
pub use compositor_pipeline_abi_worldset_base::base::Facts;
use compositor_pipeline_build_pipeline_base::pipeline::CompiledPipeline;
use compositor_support_system_storage_slot_base::base::Storage;
use compositor_support_system_buffer_token_base::y5_buffer;
use compositor_support_system_storage_token_base::y5_storage;
use compositor_support_system_trait_system_base::base::{BufferCx, System, SystemCx, WorldBuilder};
use compositor_support_system_world_frame_base::base::FrameTick;
use std::any::Any;


/// What the renderer produced for ONE world on ONE output, one frame old.
///
/// Mirrors `renderer.core`'s own `OutputResources`: the renderer is the only
/// place this data exists — it comes off that output's draw ops and that output's
/// composite — but it is not the renderer's to own, so the frame driver hands it
/// to the world and the consumers read it from there.
#[derive(Default, Clone)]
pub struct OutputFrame {
    /// The drawables collected for this world on this output.
    pub world_set: Option<std::sync::Arc<compositor_pipeline_abi_worldset_base::base::WorldSet>>,
    /// The warp grid this output's device produced. Only the inline path fills
    /// it; a fully-offloaded bundle's comes back through the worker's readback.
    pub warp_grid: Option<std::sync::Arc<Vec<[f32; 2]>>>,
    /// This output's composited band and window layer, exported for the worker.
    pub content_share:
        Option<std::sync::Arc<compositor_kernel_vulkan_memory_external_base::external::Shared>>,
    pub windows_share:
        Option<std::sync::Arc<compositor_kernel_vulkan_memory_external_base::external::Shared>>,
}

impl OutputFrame {
    /// Whether this holds anything at all — the "still needs clearing" test.
    fn any(&self) -> bool {
        self.world_set.is_some()
            || self.warp_grid.is_some()
            || self.content_share.is_some()
            || self.windows_share.is_some()
    }
}

/// Everything the pipeline knows about ONE world.
///
/// Rows are `vec4` lanes because that is what they become in the shader's
/// uniform blocks — packing them here rather than at the renderer keeps one
/// layout, and `pipeline.abi` owns what the lanes mean.
#[derive(Default)]
pub struct PipelineState {
    /// Whether this world is being looked at. See the module doc.
    ///
    /// False means the tables below are empty AND that nothing should publish
    /// them; the two always move together, which is why there is no separate
    /// "has data" question to get wrong.
    pub live: bool,
    /// The seat pointer, snapshotted per world per frame.
    ///
    /// The pointer itself is per-SEAT and correctly owned by the seat — what was
    /// wrong was the renderer fetching it from a global at draw time, behind the
    /// caller's back and with no world attached. Here it is a value this world
    /// captured on the frame it was drawn.
    ///
    /// `[0]` = `[x, y, buttons, _]` in screen UV; `[1]` = `[down_at, up_at, _, _]`
    /// on the shared clock.
    pub pointer: [[f32; 4]; 2],
    /// Per-drawable timestamps, index-aligned with the world set the renderer
    /// collects. `life` = opened/entered/left, `state` = focused/topmost,
    /// `drag` = resize and move start/end. See `pipeline.abi::descriptor`.
    pub life: Vec<[f32; 4]>,
    pub state: Vec<[f32; 4]>,
    pub drag: Vec<[f32; 4]>,
    /// What this world's bundle asks the engine for. Unlike the tables above this
    /// is NOT gated on `live`: a world still on screen behind an overlay is still
    /// the one whose band the engine is drawing, so its claim has to stand even
    /// while its descriptor tables are wiped.
    pub facts: Facts,
    /// What the renderer produced for this world, PER OUTPUT.
    ///
    /// Per output and not per world, because the geometry is: `collect`
    /// normalises every rect to the pass's own extent, so two monitors describe
    /// the same window in two different UV spaces. Held in one slot they
    /// overwrote each other every frame and the worker sampled whichever
    /// composited last — the same shape as the renderer's own per-output
    /// resources, and fixed the same way.
    pub outputs: std::collections::HashMap<std::sync::Arc<str>, OutputFrame>,
}

impl PipelineState {
    /// Drop everything this world knows, and stop publishing.
    ///
    /// Capacity is released rather than kept: a world that stopped being looked
    /// at may never be looked at again, and holding a table per dormant world is
    /// how a per-world design becomes more expensive than the global it replaced.
    fn wipe(&mut self) {
        self.live = false;
        self.pointer = [[0.0; 4]; 2];
        self.life = Vec::new();
        self.state = Vec::new();
        self.drag = Vec::new();
        self.outputs = Default::default();
        // `facts` deliberately NOT reset here. Wiping is about the descriptor
        // tables going stale; the bundle a world runs has not changed because
        // nobody is looking at it, and clearing the claim would have the world
        // come back with the engine drawing a band its bundle also draws.
    }
}

y5_storage!(pub PIPELINE, PIPELINE_MUT: PipelineState);

/// Restate what the world being DRAWN asks of the engine, into that world's own
/// slot.
///
/// # Why the writer lives here
///
/// `y5_storage!` makes `PIPELINE_MUT` crate-private on purpose, so this is the
/// only place a world's pipeline facts can be written. That is the guarantee the
/// old design lacked: four `pub` atomics that anything could store into, and did.
///
/// # Why it is called per frame, not per selection
///
/// Selection happens once per BUNDLE; these are properties of the WORLD BEING
/// DRAWN. A world returning to the screen with an already-compiled bundle never
/// re-selects, so a selection-time write leaves the engine following whichever
/// world selected last — open the picker, come back, and the session world is
/// drawn under the picker's answers.
///
/// `None` — a world on the built-in shader, or one whose bundle failed to load —
/// states the neutral answers, so the engine does nothing extra for a world that
/// asked for nothing.
pub fn publish_facts(storage: &mut Storage, cp: Option<&CompiledPipeline>) {
    let Some(p) = storage.try_get_mut(&PIPELINE_MUT) else { return };
    // `suppressed` is NOT part of this: it is decided by the frame driver, which
    // knows what is on screen, and a bundle restating its own claim must not
    // clear it. Read-modify-write on the other three only.
    match cp {
        Some(cp) => {
            p.facts.requires = cp.requires.bits();
            p.facts.whole_frame = cp.composes_whole_frame();
            p.facts.owns = cp.owns.into();
        }
        None => {
            p.facts.requires = 0;
            p.facts.whole_frame = false;
            p.facts.owns = Own::None;
        }
    }
}

/// The frame driver: whether something other than this world's live band is on
/// screen. Separate from [`publish_facts`] because it is not the bundle's answer
/// — the picker, the lock and the overview each put a picture up that the live
/// bundle did not draw.
pub fn publish_suppressed(storage: &mut Storage, on: bool) {
    if let Some(p) = storage.try_get_mut(&PIPELINE_MUT) {
        p.facts.suppressed = on;
    }
}

/// Whether a pipeline is ACTIVE for this world — see [`Facts::active`]. THE gate
/// for every per-frame cost the feature adds; false means the engine does exactly
/// what it did before the feature existed.
pub fn active(storage: &Storage) -> bool {
    facts(storage).active()
}

/// Whether this world holds any pipeline state at all — a live bundle, or values
/// left by one that has gone and still need clearing.
///
/// The difference from [`active`] is the frame a bundle unloads: `active` is
/// already false, but the world is still holding a set, a grid and two shares
/// that must be released. A gate written on `active` alone would skip that frame
/// and strand them.
pub fn holds(storage: &Storage) -> bool {
    match storage.try_get(&PIPELINE) {
        None => false,
        Some(p) => {
            p.facts.active() || !p.outputs.is_empty()
        }
    }
}

/// Hand this world what the renderer produced for ONE of its outputs.
///
/// One call rather than four: they are produced together, by one pass, for one
/// output, and splitting them was four chances to key one of them differently.
pub fn publish_frame(storage: &mut Storage, output: &std::sync::Arc<str>, frame: OutputFrame) {
    let Some(p) = storage.try_get_mut(&PIPELINE_MUT) else { return };
    match frame.any() {
        // Removed rather than stored empty, so an output that goes quiet — or a
        // monitor that is unplugged — leaves no entry behind to be read.
        false => {
            p.outputs.remove(output);
        }
        true => {
            p.outputs.insert(std::sync::Arc::clone(output), frame);
        }
    }
}

/// What the renderer produced for this world on `output`. Default — everything
/// absent — for an output this world has not been drawn on.
pub fn frame(storage: &Storage, output: &str) -> OutputFrame {
    storage
        .try_get(&PIPELINE)
        .and_then(|p| p.outputs.get(output).cloned())
        .unwrap_or_default()
}

/// What this world asks of the engine. Neutral for a world with no pipeline slot
/// — a test world, or one built before the system was registered — which is the
/// same answer as "no bundle", and the right one.
pub fn facts(storage: &Storage) -> Facts {
    storage.try_get(&PIPELINE).map(|p| p.facts).unwrap_or_default()
}


/// Mutation intents. `update` holds storage READ-ONLY, so every write is queued
/// here and applied in [`System::buffer`] — the same discipline `TwoSystem`
/// follows, and the reason a system cannot quietly mutate another's slot.
enum PipelineCmd {
    /// This world was drawn: mark it live and take the frame's pointer.
    Observe([[f32; 4]; 2]),
    /// This world stopped being looked at.
    Wipe,
    /// Restate what this world's bundle asks of the engine.
    Facts(Facts),
}
y5_buffer!(PIPELINE_BUF: PipelineCmd);

/// Maintains [`PIPELINE`] for the world it is registered in.
///
/// Descriptor publishing ONLY. Which bundle a world runs, when it compiles and
/// when it reloads stay on `Two`/`TwoSystem` — this system has no opinion about
/// any of that, and deliberately: the two answer different questions and giving
/// them one owner is what made the previous design reach for globals.
#[derive(Default)]
pub struct PipelineSystem;

impl System for PipelineSystem {
    fn name(&self) -> &'static str {
        "pipeline.world"
    }

    fn register(&mut self, builder: &mut WorldBuilder) {
        builder.storage.insert(&PIPELINE, PipelineState::default());
    }

    fn update(&mut self, cx: &mut SystemCx, _tick: &FrameTick) {
        // NOTHING for a world with no bundle. `cx.write` boxes its message, so an
        // unconditional observe was a heap allocation per world per frame paid by
        // every desktop running the stock parallax. One token read replaces it.
        //
        // The facts are a frame old here — systems run before the drawn world
        // states them — so a bundle just selected begins observing one frame late,
        // which is a frame in which nothing samples the tables anyway.
        //
        // Deactivation is SYMMETRIC and costs exactly one frame: a world that was
        // observing and no longer has a bundle wipes once, then answers `live ==
        // false` and does nothing ever after. Without that the tables and the
        // pointer would sit at whatever the last bundle left, holding their
        // capacity for a world that may never run one again.
        let Some(p) = cx.storage.try_get(&PIPELINE) else { return };
        if !p.facts.active() {
            if p.live {
                cx.write(&PIPELINE_BUF, PipelineCmd::Wipe);
            }
            return;
        }
        // The seat's own value, read once per world per frame and captured. The
        // renderer no longer reaches for it: it is handed this snapshot with the
        // rest of the world's state.
        let pointer = compositor_orchestration_seat_pointer_publish::publish::packed(
            compositor_pipeline_abi_clock_base::base::NEVER,
        );
        cx.write(&PIPELINE_BUF, PipelineCmd::Observe(pointer));
    }

    /// The world stopped being the active one.
    ///
    /// Not always a wipe — see the module doc. A world that is still the spawn
    /// target is still on screen, so it keeps publishing; only a world nobody is
    /// looking at drops its tables.
    fn on_disable(&mut self, cx: &mut SystemCx) {
        cx.write(&PIPELINE_BUF, PipelineCmd::Wipe);
    }

    fn buffer(&mut self, cx: &mut BufferCx, message: Box<dyn Any>) {
        let Ok(cmd) = message.downcast::<PipelineCmd>() else { return };
        let Some(p) = cx.storage.try_get_mut(&PIPELINE_MUT) else { return };
        match *cmd {
            PipelineCmd::Observe(pointer) => {
                p.live = true;
                p.pointer = pointer;
            }
            PipelineCmd::Wipe => p.wipe(),
            PipelineCmd::Facts(f) => p.facts = f,
        }
    }
}
