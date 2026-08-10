//! The world set: every on-screen WORLD drawable's geometry AND its source
//! buffer, as one index-aligned band in composite order.
//!
//! "World", not "window": the set carries client windows AND the iced-world
//! surfaces that interleave with them (placeholders, groups, and whatever the
//! world grows next). They are one band in one order — `drawable_order()` — and a
//! shader handed only the window half cannot reconstruct that order, because the
//! half it did not get is drawn by the ENGINE, after the pipeline's output pass,
//! and therefore lands on top of everything the shader drew. Carrying the whole
//! band with a [`Kind`] per entry is what makes composite order expressible.
//!
//! ONE structure, not two. Geometry alone would sit happily in `model.stats`, but
//! the buffers cannot — a model-layer crate must not take a graphics dependency —
//! and splitting them apart is precisely how `rects[i]` and `texture[i]` drift.
//! That bug has already cost this feature once (every window showing another
//! window's content), so they travel together, index-aligned by construction.
//!
//! # Where this lives, and why it moved
//!
//! It used to be `kernel.graphic/graphic.bridge/bridge.window/window.set`, a
//! process-global slot, on the reasoning that the bridge is "where cross-thread
//! graphics handoff already lives and both ends can reach it". Reachability is
//! the wrong reason to place a crate: nothing in the kernel defines what a world
//! drawable means to a shader — that is this feature's ABI — and parking it below
//! everything so anyone could touch it without declaring a dependency is the
//! crate-level form of the same mistake the global slot was.
//!
//! So these are TYPES now, and only types. The renderer collects a set and hands
//! it on; nothing publishes one to a place other code goes looking.
//!
//! # Lifetime
//!
//! `Dmabuf` is `Arc`-backed and the worker DUPS the fd when it imports, so a
//! consumer's copy stays valid after the client releases the buffer. Carrying one
//! costs a refcount and creates no cross-device lifetime coupling.
//!
//! What it does NOT carry is a fence. A client may be redrawing a buffer while the
//! worker samples it, so an off-thread reader can catch a torn frame. That is a
//! visual artefact on a background effect, not a fault, and it is the trade for
//! not making the compositor mediate per-client acquire points.

use smithay::backend::allocator::dmabuf::Dmabuf;
use std::sync::Arc;

/// Matches the shader-side `Windows` UBO capacity (`pipeline.execute/graph`).
///
/// A cap, not a budget: it is the fixed length of the WGSL uniform arrays, which
/// the language requires to be a compile-time constant, so it cannot simply be
/// removed. Overflow is reported by [`WorldSet::truncate_to_max`] rather than
/// silently dropped.
pub const MAX: usize = 256;

/// What an entry is. The engine draws every kind; a bundle that OWNS the band
/// has to draw them all too, and this is how it tells them apart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    /// A client window — the only kind that existed before the world set did, and
    /// still the only kind a `windows: "pipeline"` bundle is handed.
    #[default]
    Window = 0,
    /// An iced-world surface: placeholders, groups, and future world content.
    /// World-space and interleaved with windows by `drawable_order()`, but not a
    /// client window — glass/window-glow must not treat one as one.
    Panel = 1,
}

/// How much of the world band a bundle takes over — i.e. how much the engine must
/// leave out of its own draw.
///
/// This is a property of the BUNDLE, and therefore of the world running it. It
/// used to be a process-global atomic published at selection, which is what made
/// a world returning to the screen inherit whichever world selected last.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Own {
    /// The engine draws the whole band, as it always has.
    #[default]
    None = 0,
    /// `windows: "pipeline"` — the bundle draws client windows; the engine still
    /// draws iced-world panels, which therefore land ON TOP of them. Kept for the
    /// bundles written against it; [`Own::World`] is the ordering-correct form.
    Windows = 1,
    /// `windows: "world"` — the bundle draws the entire world band in published
    /// order, and the engine draws none of it.
    World = 2,
}

impl Own {
    /// Whether a bundle owning this much draws THIS kind of drawable itself, so
    /// the engine must neither blit it nor treat it as covering what is behind it.
    pub fn covers(self, is_window: bool) -> bool {
        match self {
            Own::None => false,
            Own::Windows => is_window,
            Own::World => true,
        }
    }
}

impl From<u8> for Own {
    fn from(v: u8) -> Self {
        match v {
            1 => Own::Windows,
            2 => Own::World,
            _ => Own::None,
        }
    }
}

/// A drawable's pixels, in whichever form a second device can import them.
///
/// Both arms exist because the two client types arrive differently and neither
/// can be converted to the other cheaply: a dmabuf client already has an fd worth
/// passing on, and a SHM client has none, so the compositor shares the image it
/// uploaded into instead. `Unavailable` is the honest third case — an export that
/// failed — rather than a silent gap in the array.
#[derive(Clone)]
pub enum Source {
    /// The client's own buffer. The importer dups the fd.
    Client(Dmabuf),
    /// The compositor's uploaded image, shared as `OPAQUE_FD`. Valid only between
    /// logical devices on the SAME physical device, which is how the worker is
    /// configured.
    Shared(Arc<compositor_kernel_vulkan_memory_external_base::external::Shared>),
    Unavailable,
}

/// One frame's on-screen world drawables, **back to front** — the same order the
/// engine would have drawn them, so a shader iterating `0..count` composites in
/// composite order with no sort key to carry. All vectors are the same length and
/// index-aligned: entry `i` is one drawable.
#[derive(Clone, Default)]
pub struct WorldSet {
    /// Screen-space rect in UV: xy = origin, zw = size.
    pub rects: Vec<[f32; 4]>,
    /// Texture-crop UV within that drawable's own buffer: xy = origin, zw = size.
    pub srcs: Vec<[f32; 4]>,
    /// What each entry is, and how opaque.
    pub kinds: Vec<Kind>,
    pub alphas: Vec<f32>,
    /// What the compositor knows about each entry's window — focus, stacking,
    /// fullscreen, resize. Raw bits; the meaning lives in `pipeline.abi/descriptor`.
    pub flags: Vec<u32>,
    /// When each entry's window last opened, entered, left, took focus, became
    /// topmost and was resized — `pipeline.abi::descriptor::Times`, on the shared
    /// clock.
    ///
    /// EMPTY unless the active bundle declared `window_times`, which is the whole
    /// of what that requirement gates on this side. Index-aligned with the rest
    /// when it is populated; a consumer must check the length rather than the
    /// count, because "no bundle asked for these" and "no drawables" are different
    /// answers.
    pub times: Vec<[f32; 12]>,
    /// How a second device can get at each drawable's pixels.
    pub sources: Vec<Source>,
}

impl WorldSet {
    /// Whether every entry `own` would hand the shader carries a buffer a second
    /// device could import. A single SHM surface makes this false, which is the
    /// condition a consumer should downgrade on rather than render a partial set.
    ///
    /// Scoped to `own` rather than the whole set on purpose: a bundle that only
    /// receives client windows must not be forced back inline by a panel it will
    /// never be handed.
    pub fn all_importable(&self, own: Own) -> bool {
        self.owned(own).into_iter().all(|i| !matches!(self.sources[i], Source::Unavailable))
    }

    /// The indices this `own` mode hands to the shader, in draw order.
    ///
    /// The SAME predicate the engine suppresses on — that identity is the whole
    /// invariant. A shader given a set the engine did not fully suppress draws
    /// under the leftovers; a set wider than what it was given leaves holes. Both
    /// are ordering bugs, and both are prevented by there being one function.
    pub fn owned(&self, own: Own) -> Vec<usize> {
        (0..self.kinds.len())
            .filter(|&i| match own {
                Own::None | Own::Windows => self.kinds[i] == Kind::Window,
                Own::World => true,
            })
            .collect()
    }

    /// Clamp to [`MAX`], saying so when anything is lost.
    ///
    /// The tail is dropped and the loss is logged, because the alternative — the
    /// silent truncation this had once — reads on screen as windows that simply
    /// stopped existing, with nothing anywhere to say why. Going truly unbounded
    /// means moving the geometry to a storage buffer AND giving the bindless
    /// texture array a variable descriptor count, which is an ABI break across
    /// every bundle that declares `struct Windows`.
    pub fn truncate_to_max(&mut self) {
        let len = self.kinds.len();
        if len <= MAX {
            return;
        }
        warn!(
            "world set: {len} drawables exceeds the {MAX}-entry shader array; {} dropped — \
             they will not be drawn by a bundle that owns the world band",
            len - MAX
        );
        self.rects.truncate(MAX);
        self.srcs.truncate(MAX);
        self.kinds.truncate(MAX);
        self.alphas.truncate(MAX);
        self.flags.truncate(MAX);
        // `times` is empty whenever the bundle did not ask for it, so it is
        // truncated only if it is actually populated — folding an intentionally
        // empty lane into the length would be a different bug.
        self.times.truncate(MAX.min(self.times.len()));
        self.sources.truncate(MAX);
    }
}

/// What one world's bundle asks the engine to do differently.
///
/// These three answer "what changes because of the bundle on screen": collect the
/// world set (`requires`), leave the world band to the shader (`owns`), redraw
/// whole rather than by damage (`whole_frame`). Plus the one thing that can
/// override the claim from outside the bundle (`suppressed`).
///
/// They were four process-global atomics in `bridge.window/window.set`, written
/// once per bundle SELECTION. That is the multi-world bug in its purest form: a
/// bundle belongs to a world, selection happens once, and a world returning to
/// the screen with an already-compiled bundle never restated anything — so the
/// engine went on following whichever world selected last. Here they are a field
/// of the world that owns them, restated by that world on every frame it is
/// drawn, and a frame after a switch is indistinguishable from any other frame.
#[derive(Clone, Copy, Default)]
pub struct Facts {
    /// The bundle's requirement bits — THE gate for every per-frame cost the
    /// pipeline adds. Zero (no bundle, or one that asks for nothing) means the
    /// engine does none of it, which is what makes a background-only multipass
    /// bundle cost what a single-pass shader costs.
    ///
    /// Raw bits rather than `Requires`: the type lives in `bundle.require` and
    /// this slot is transport. Both ends go through `Requires::{bits, from_bits}`.
    pub requires: u16,
    /// Whether the bundle makes the compositor clear and recompose the WHOLE
    /// target, and therefore needs the whole element list to recompose it from.
    pub whole_frame: bool,
    /// How much of the world band the bundle takes over.
    pub owns: Own,
    /// Whether something OTHER than this world's live band is on screen, so the
    /// claim must not apply to it.
    ///
    /// The picker and the lock are their own worlds with their own backgrounds;
    /// the overview sits on a frozen backdrop capture rather than the live
    /// bundle's output. In all three the engine is the only thing drawing world
    /// content, and honouring a claim against a band the bundle is not producing
    /// suppresses that content into a blank screen.
    pub suppressed: bool,
}

impl Facts {
    /// **Whether a pipeline is ACTIVE for this world** — the one predicate every
    /// engine-side gate asks.
    ///
    /// A world with no bundle, or one whose bundle asks for nothing, must cost the
    /// engine exactly what it cost before this feature existed. That is not a
    /// nice-to-have: the overwhelmingly common desktop runs the stock parallax,
    /// and any per-frame work that survives here is paid by every user who never
    /// loads a bundle at all.
    ///
    /// Concrete and in one place ON PURPOSE. The alternative — each call site
    /// spelling out its own `requires != 0 || owns != None || …` — is how one gate
    /// ends up disagreeing with another about whether the feature is on, and the
    /// disagreement shows up as work done for a bundle that is not there or,
    /// worse, skipped for one that is.
    ///
    /// `suppressed` is deliberately NOT part of this. It says another surface is
    /// on screen right now, not that the bundle went away — a suppressed frame
    /// still has a live bundle whose state must keep flowing.
    pub fn active(&self) -> bool {
        self.requires != 0 || self.owns != Own::None || self.whole_frame
    }

    /// The claim as it applies THIS frame — `Own::None` while suppressed.
    pub fn owns_band(&self) -> Own {
        match self.suppressed {
            true => Own::None,
            false => self.owns,
        }
    }

    /// Whether the bundle draws THIS kind of drawable itself, so the engine must
    /// neither blit it nor treat it as covering what is behind it.
    pub fn covers(&self, is_window: bool) -> bool {
        self.owns_band().covers(is_window)
    }
}
