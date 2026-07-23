//! FROZEN v1 boundary (`y5_api = "1"`) — never edit. v1 plugins export a raw
//! `artifact_plugin_new` symbol (pre-root-module era) and their quads carry no
//! `band`. The bridge to the current contract lives beside this in
//! `plugin.v1/v1.adapter`; nothing outside `plugin.v1/` should reference v1 types.

use abi_stable::StableAbi;
use abi_stable::sabi_trait;
use abi_stable::std_types::{RBox, RVec};

/// A physical, axis-aligned rectangle (v1).
#[repr(C)]
#[derive(StableAbi, Clone, Copy)]
pub struct AbiRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// A quad (v1): rect + RGBA colour. NO band — v1 predates layer selection.
#[repr(C)]
#[derive(StableAbi, Clone, Copy)]
pub struct AbiQuad {
    pub rect: AbiRect,
    pub color: [f32; 4],
}

/// Per-frame context (v1).
#[repr(C)]
#[derive(StableAbi)]
pub struct FrameCtx {
    pub windows: RVec<AbiRect>,
    pub dt: f32,
}

/// The v1 plugin behaviour.
#[sabi_trait]
pub trait ArtifactPlugin {
    fn draw(&mut self, ctx: &FrameCtx) -> RVec<AbiQuad>;
}

/// The owned v1 trait-object.
pub type ArtifactPluginBox = ArtifactPlugin_TO<'static, RBox<()>>;

/// v1's raw constructor symbol (nul-terminated for `libloading`).
pub const PLUGIN_NEW: &[u8] = b"artifact_plugin_new\0";

/// Type of that exported constructor.
pub type PluginNewFn = extern "C" fn() -> ArtifactPluginBox;
