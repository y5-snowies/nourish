//! `AdapterV1` — bridges a v1 plugin to the CURRENT contract, fully encapsulating
//! v1: this crate owns the v1 LOADING path (raw `artifact_plugin_new` symbol — v1
//! predates the root-module export) AND the semantic bridge. Nothing outside
//! `plugin.v1/` touches v1 types; the host proxy only ever sees the current trait.
//!
//! One-time authored judgment (the adapter IS the capability decision): v1 quads
//! carry no `band`, so they default to WORLD_3D (100, under windows) — v1's original
//! behaviour — logged once so "old plugin draws under windows" is diagnosable.

use abi_stable::sabi_trait::TD_Opaque;
use abi_stable::std_types::RVec;
use compositor_artifact_plugin_abi_base::abi as current;
use compositor_artifact_plugin_v1_abi::abi as v1;
use libloading::{Library, Symbol};
use std::path::Path;

/// v1 quads have no band: WORLD_3D (see `world.frame`'s Layer table), v1's
/// original compositing position.
const V1_DEFAULT_BAND: u16 = 100;

/// A v1 plugin wrapped as a current-contract plugin.
struct AdapterV1 {
    inner: v1::ArtifactPluginBox,
    /// Keeps the v1 `.so` mapped for the plugin's lifetime (declared last → drops last).
    _lib: Library,
}

impl current::ArtifactPlugin for AdapterV1 {
    fn draw(&mut self, ctx: &current::FrameCtx) -> RVec<current::AbiQuad> {
        // FrameCtx is layout-identical across v1/v2; convert type-wise.
        let v1_ctx = v1::FrameCtx {
            windows: ctx
                .windows
                .iter()
                .map(|r| v1::AbiRect { x: r.x, y: r.y, w: r.w, h: r.h })
                .collect(),
            dt: ctx.dt,
        };
        self.inner
            .draw(&v1_ctx)
            .into_iter()
            .map(|q| current::AbiQuad {
                rect: current::AbiRect { x: q.rect.x, y: q.rect.y, w: q.rect.w, h: q.rect.h },
                color: q.color,
                band: V1_DEFAULT_BAND,
            })
            .collect()
    }

    /// v1 predates plugin-rendered dmabufs: always the empty set.
    fn draw_dmabuf(&mut self, _ctx: &current::DmabufCtx) -> RVec<current::AbiDmabufQuad> {
        RVec::new()
    }
}

/// Load a v1 plugin `.so` and return it speaking the CURRENT contract.
///
/// # Safety
/// Loads and executes arbitrary native code from `path` (trusted 1c tier).
pub unsafe fn load(path: &Path) -> Result<current::ArtifactPluginBox, libloading::Error> {
    let lib = unsafe { Library::new(path)? };
    let inner = {
        let ctor: Symbol<v1::PluginNewFn> = unsafe { lib.get(v1::PLUGIN_NEW)? };
        let ctor = *ctor;
        ctor()
    };
    info!("v1 compat: plugin bridged via AdapterV1 (band defaults to WORLD_3D)");
    Ok(current::ArtifactPlugin_TO::from_value(AdapterV1 { inner, _lib: lib }, TD_Opaque))
}
