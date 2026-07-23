//! The stable-ABI boundary between the y5 host and a 1c native artifact plugin
//! (`.so`). ONLY these `#[repr(C)]` / `StableAbi` types and the `#[sabi_trait]`
//! trait object cross the boundary, so a plugin built with a different toolchain /
//! crate versions stays sound (document/EXTENSIONS.md §5). The host does the
//! world→physical projection and hands the plugin already-projected window rects;
//! the plugin returns solid quads to composite. The plugin therefore needs no y5 /
//! smithay types — it is pure effect geometry.

use abi_stable::StableAbi;
use abi_stable::library::RootModule;
use abi_stable::sabi_trait;
use abi_stable::sabi_types::VersionStrings;
use abi_stable::std_types::{RBox, RVec};
use abi_stable::{declare_root_module_statics, package_version_strings};

/// An axis-aligned rectangle in WORLD-LOGICAL units: window rects arrive in world
/// coordinates and returned quads are placed in world space (they pan/zoom with the
/// camera; the host projects to physical). Screen-space quads are future suffix
/// growth.
#[repr(C)]
#[derive(StableAbi, Clone, Copy)]
pub struct AbiRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// A quad the plugin wants drawn: a physical rect + premultiplied-ish RGBA colour +
/// the compositing band.
///
/// `band` is the host's draw band number (see `world.frame`'s `Layer` table):
/// lower draws first. Reference points: 100 = WORLD_3D (under windows),
/// 400 = the window/content band, 401/402 = RESERVED (floating panes),
/// 410 = above all canvas content, 500 = compositor screen UI, 700 = pointer.
/// Contract `y5_api = "2"` (the band field is the v1→v2 change).
#[repr(C)]
#[derive(StableAbi, Clone, Copy)]
pub struct AbiQuad {
    pub rect: AbiRect,
    pub color: [f32; 4],
    pub band: u16,
}

/// Per-frame context the host passes into the plugin.
#[repr(C)]
#[derive(StableAbi)]
pub struct FrameCtx {
    /// Projected physical rects of the mapped windows (host did the projection).
    pub windows: RVec<AbiRect>,
    /// Seconds since the previous frame.
    pub dt: f32,
}

/// A plugin-RENDERED dmabuf quad: the plugin renders with its OWN engine into a
/// dmabuf and hands the fd across. In-process, the fd is just an int — the host
/// `dup()`s it at import, so each side manages its own lifetime; the plugin must
/// keep its fd valid while the `id` remains in the returned set. Single-plane v1;
/// sync is implicit-fence (the same discipline the compositor's own bevy/iced
/// dmabuf sharing relies on).
#[repr(C)]
#[derive(StableAbi, Clone, Copy)]
pub struct AbiDmabufQuad {
    /// Plugin-chosen stable identity (drives host-side import caching + damage).
    pub id: u64,
    /// Bump when the buffer CONTENTS changed since the last frame (damage).
    pub commit: u64,
    /// The dmabuf fd (plugin-owned; host dups).
    pub fd: i32,
    /// Buffer pixel size + DRM fourcc + modifier + plane-0 layout.
    pub width: i32,
    pub height: i32,
    pub fourcc: u32,
    pub modifier: u64,
    pub stride: u32,
    pub offset: u32,
    /// Placement: 0 = WORLD space (world-logical; pans/zooms), 1 = SCREEN space
    /// (logical px; fixed). The rect is the on-screen viewport — the texture
    /// stretches to it (any stretch).
    pub screen: u8,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Compositing band (see `AbiQuad::band`).
    pub band: u16,
}

/// The plugin behaviour. `draw` returns the quads to composite this frame given the
/// window rects + elapsed time.
///
/// ADDITIVE EVOLUTION: methods added in later minor versions go AFTER the
/// `last_prefix_field` marker — old plugins keep loading; absent suffix methods are
/// probed once and skipped by the host. Deliberately MINIMAL: the ONE pixel
/// primitive beyond solid quads is the dmabuf; content vocabularies belong to the
/// bus + channels RW era (EXTENSIONS.md §10), not this trait.
#[sabi_trait]
pub trait ArtifactPlugin {
    #[sabi(last_prefix_field)]
    fn draw(&mut self, ctx: &FrameCtx) -> RVec<AbiQuad>;

    /// SUFFIX (v2-additive): the full retained set of plugin-rendered dmabuf quads
    /// for this frame, given the host's [`DmabufCtx`] (screen dims + the NEGOTIATED
    /// importable format the plugin must allocate with). Absent on old plugins
    /// (host probes once → empty set).
    fn draw_dmabuf(&mut self, ctx: &DmabufCtx) -> RVec<AbiDmabufQuad>;
}

/// Per-frame context for `draw_dmabuf`.
#[repr(C)]
#[derive(StableAbi)]
pub struct DmabufCtx {
    /// Output LOGICAL dimensions, so screen-space quads can self-anchor.
    pub screen_w: f64,
    pub screen_h: f64,
    /// The host-NEGOTIATED importable format (the compositor's renderer ∩ wgpu
    /// bridge set): allocate buffers with this fourcc and EXACTLY these modifiers
    /// (e.g. gbm `with_modifiers2`). An empty list ⇒ the driver's implicit path is
    /// acceptable. Buffers outside this set may fail import — a logged skip, never
    /// a CPU fallback.
    pub fourcc: u32,
    pub modifiers: RVec<u64>,
}

/// The owned trait-object the plugin constructor returns and the host drives.
pub type ArtifactPluginBox = ArtifactPlugin_TO<'static, RBox<()>>;

/// The plugin's exported ROOT MODULE — a prefix type, so it grows add-only: new
/// entries go AFTER `new`; a host reading a plugin that lacks them sees their
/// absence instead of UB, and abi_stable verifies the layout recursively at load
/// (a mismatch is a clean `LibraryError`, never a crash). Export from the plugin
/// with `#[export_root_module]`; load host-side with
/// `ArtifactMod_Ref::load_from_file(path)`.
#[repr(C)]
#[derive(StableAbi)]
#[sabi(kind(Prefix))]
#[sabi(missing_field(panic))]
pub struct ArtifactMod {
    /// Construct the plugin. LAST prefix field — everything after is optional.
    #[sabi(last_prefix_field)]
    pub new: extern "C" fn() -> ArtifactPluginBox,
}

impl RootModule for ArtifactMod_Ref {
    declare_root_module_statics! {ArtifactMod_Ref}
    const BASE_NAME: &'static str = "artifact_plugin";
    const NAME: &'static str = "artifact_plugin";
    const VERSION_STRINGS: VersionStrings = package_version_strings!();
}
