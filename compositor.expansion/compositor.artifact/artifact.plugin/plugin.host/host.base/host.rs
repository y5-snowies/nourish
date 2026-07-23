//! Host-side plugin proxy: drives a loaded artifact plugin as a y5 `System`.
//! Feeds the plugin WORLD-logical window rects and wraps its returned quads as
//! WORLD-space `ArtifactQuad`s at each quad's declared band — projection to
//! physical happens once, in the frame driver (`scene.frame`), not here and not in
//! plugins. Historical contract versions arrive pre-bridged through their version
//! layer (e.g. `plugin.v1/v1.adapter`), which also owns their `.so` keepalive;
//! current (root-module) loads are kept alive by abi_stable itself.

use abi_stable::library::RootModule;
use abi_stable::std_types::RVec;
use compositor_artifact_draw_quad_base::quad::{ArtifactQuad, ArtifactSpace};
use compositor_artifact_plugin_abi_base::abi::{
    AbiDmabufQuad, AbiRect, ArtifactMod_Ref, ArtifactPlugin, ArtifactPluginBox, DmabufCtx,
    FrameCtx,
};
use compositor_orchestration_smithay_data_base::data::SCREEN;
use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufFlags};
use smithay::backend::allocator::{Fourcc, Modifier};
use std::collections::HashMap;
use compositor_orchestration_draw_platform_base::platform::Platform;
use compositor_support_system_trait_system_base::base::{System, SystemCx, WorldBuilder};
use compositor_support_system_world_frame_base::base::{self as layer, FramePlan};
use compositor_y5_camera_transform_translate::slot;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::utils::CommitCounter;
use std::path::Path;

/// One imported plugin dmabuf, cached by the plugin's quad id so the fd is dup'd +
/// wrapped once, with a stable element id + commit counter for damage tracking.
struct DmaCacheEntry {
    fd: i32,
    width: i32,
    height: i32,
    fourcc: u32,
    modifier: u64,
    stride: u32,
    offset: u32,
    dmabuf: Dmabuf,
    sm_id: Id,
    commit: CommitCounter,
    last_commit: u64,
}

impl DmaCacheEntry {
    fn same_buffer(&self, q: &AbiDmabufQuad) -> bool {
        self.fd == q.fd
            && self.width == q.width
            && self.height == q.height
            && self.fourcc == q.fourcc
            && self.modifier == q.modifier
            && self.stride == q.stride
            && self.offset == q.offset
    }
}

/// Dup the plugin's fd and wrap it as a smithay `Dmabuf` (single-plane v1).
fn import_dmabuf_fd(q: &AbiDmabufQuad) -> Option<Dmabuf> {
    let fourcc = Fourcc::try_from(q.fourcc).ok()?;
    let owned = unsafe { std::os::fd::BorrowedFd::borrow_raw(q.fd) }
        .try_clone_to_owned()
        .ok()?;
    let mut builder =
        Dmabuf::builder((q.width, q.height), fourcc, Modifier::from(q.modifier), DmabufFlags::empty());
    builder.add_plane(owned, 0, q.offset, q.stride);
    builder.build()
}

/// A loaded native plugin driven as a y5 `System` (speaks ONLY the current contract).
pub struct PluginSystem {
    plugin: ArtifactPluginBox,
    /// Previous `draw` instant, for a real per-frame dt. `FrameTick.delta` is
    /// always ZERO (see `CameraSystem::update`), so we self-time like it does.
    last: Option<std::time::Instant>,
    /// Whether the plugin's vtable has the `draw_dmabuf` SUFFIX method (probed on
    /// first use; plugins older than the method simply have none).
    has_dmabuf: Option<bool>,
    /// Imported plugin dmabufs by plugin quad id (evicted when an id leaves the set).
    dma_cache: HashMap<u64, DmaCacheEntry>,
    /// The NEGOTIATED importable format handed to the plugin (computed once from
    /// the renderer's dmabuf formats ∩ the wgpu bridge set — the same recipe the
    /// host's own bevy surfaces allocate with).
    negotiated: Option<(u32, Vec<u64>)>,
}

impl PluginSystem {
    /// Load a plugin `.so` for the manifest-declared contract `y5_api`, routing
    /// through the version-adapter chain. Two gates: an unsupported version is
    /// refused HERE (before any adapter runs); the current path additionally runs
    /// abi_stable's recursive layout verification at load (a mismatch is a clean
    /// error, never UB) — on failure the library is unloaded.
    ///
    /// # Safety
    /// Loads and executes arbitrary native code from `path`; the caller must trust it
    /// (the 1c tier is trusted — see document/EXTENSIONS.md).
    pub unsafe fn load(path: &Path, y5_api: &str) -> Result<Self, String> {
        let plugin = match y5_api {
            // v1: raw-symbol era, no band — loaded + bridged entirely inside its
            // version layer.
            "1" => unsafe { compositor_artifact_plugin_v1_adapter::adapter::load(path) }
                .map_err(|e| format!("v1 load: {e}"))?,
            // Current: root module with recursive layout verification.
            "2" => {
                let module = ArtifactMod_Ref::load_from_file(path)
                    .map_err(|e| format!("layout verification: {e}"))?;
                (module.new())()
            }
            other => return Err(format!("unsupported y5_api `{other}` (supported: 1, 2)")),
        };
        Ok(Self { plugin, last: None, has_dmabuf: None, dma_cache: HashMap::new(), negotiated: None })
    }
}

impl System for PluginSystem {
    fn name(&self) -> &'static str {
        "artifact.plugin"
    }

    fn register(&mut self, _builder: &mut WorldBuilder) {}

    fn draw(&mut self, cx: &mut SystemCx, plan: &mut FramePlan) {
        let windows = world_windows(cx);
        // Real per-frame dt, self-timed (FrameTick.delta is always zero); clamped so
        // a slow/first frame can't jump the animation.
        let now = std::time::Instant::now();
        let dt = self
            .last
            .replace(now)
            .map_or(1.0 / 60.0, |prev| (now - prev).as_secs_f32())
            .clamp(0.0, 0.1);
        let frame = FrameCtx { windows: windows.into(), dt };
        let quads: RVec<_> = self.plugin.draw(&frame);
        for q in quads.iter() {
            // The plugin picks its compositing band (trusted tier; the band table is
            // host-defined — see `abi::AbiQuad::band`). Clamped under the pointer.
            let band = layer::Layer(q.band.min(layer::POINTER.0 - 1));
            plan.push(
                band,
                Box::new(ArtifactQuad::solid_world(
                    q.color,
                    q.rect.x as f64,
                    q.rect.y as f64,
                    q.rect.w as f64,
                    q.rect.h as f64,
                    Id::new(),
                    CommitCounter::default(),
                )),
            );
        }

        // SUFFIX `draw_dmabuf`: plugin-rendered pixels. A plugin built before the
        // method existed lacks the vtable entry — the prefix accessor panics
        // HOST-side (before any plugin code runs), so probe once under
        // catch_unwind and remember the absence.
        if self.has_dmabuf != Some(false) {
            // Negotiate the importable format ONCE (renderer's importable dmabuf
            // formats ∩ the wgpu bridge set — mirror of `BevySurface::allocate`).
            if self.negotiated.is_none() {
                let wgpu_ctx = cx
                    .kernel
                    .try_get(&compositor_background_three_system_base::base::BEVY_CONTEXT)
                    .and_then(|c| c.clone());
                if let Some(wgpu_ctx) = wgpu_ctx {
                    if let Some(gles) = cx
                        .platform
                        .as_deref_mut()
                        .and_then(|p| p.downcast_mut::<Platform>())
                        .and_then(|p| p.renderer())
                    {
                        let fourcc = Fourcc::Argb8888;
                        let mods = compositor_kernel_graphic_bridge_negotiate_base::negotiate::bridge_modifiers(
                            smithay::backend::renderer::ImportDma::dmabuf_formats(gles),
                            wgpu_ctx.importable.clone(),
                            fourcc,
                        );
                        self.negotiated =
                            Some((fourcc as u32, mods.into_iter().map(Into::<u64>::into).collect()));
                    }
                }
            }
            let (fourcc, modifiers) = self
                .negotiated
                .clone()
                .unwrap_or((Fourcc::Argb8888 as u32, Vec::new()));
            let (sw, sh) = {
                let s = cx.kernel.get(&SCREEN);
                (s.size.w as f64 / s.scale, s.size.h as f64 / s.scale)
            };
            let dctx = DmabufCtx { screen_w: sw, screen_h: sh, fourcc, modifiers: modifiers.into() };
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.plugin.draw_dmabuf(&dctx)
            })) {
                Ok(dquads) => {
                    self.has_dmabuf = Some(true);
                    // Evict ids that left the retained set (drops the dup'd fd).
                    let want: std::collections::HashSet<u64> = dquads.iter().map(|q| q.id).collect();
                    self.dma_cache.retain(|id, _| want.contains(id));
                    for q in dquads.iter() {
                        use std::collections::hash_map::Entry;
                        let entry = match self.dma_cache.entry(q.id) {
                            Entry::Occupied(mut o) => {
                                if !o.get().same_buffer(q) {
                                    // The plugin swapped buffers under this id:
                                    // re-import, keep element identity for damage.
                                    match import_dmabuf_fd(q) {
                                        Some(d) => {
                                            let e = o.get_mut();
                                            e.dmabuf = d;
                                            e.fd = q.fd;
                                            e.width = q.width;
                                            e.height = q.height;
                                            e.fourcc = q.fourcc;
                                            e.modifier = q.modifier;
                                            e.stride = q.stride;
                                            e.offset = q.offset;
                                            e.commit.increment();
                                        }
                                        None => {
                                            warn!("artifact dmabuf re-import failed (id {})", q.id);
                                            o.remove();
                                            continue;
                                        }
                                    }
                                }
                                o.into_mut()
                            }
                            Entry::Vacant(v) => match import_dmabuf_fd(q) {
                                Some(d) => v.insert(DmaCacheEntry {
                                    fd: q.fd,
                                    width: q.width,
                                    height: q.height,
                                    fourcc: q.fourcc,
                                    modifier: q.modifier,
                                    stride: q.stride,
                                    offset: q.offset,
                                    dmabuf: d,
                                    sm_id: Id::new(),
                                    commit: CommitCounter::default(),
                                    last_commit: q.commit,
                                }),
                                None => {
                                    warn!("artifact dmabuf import failed (id {})", q.id);
                                    continue;
                                }
                            },
                        };
                        if q.commit != entry.last_commit {
                            entry.commit.increment();
                            entry.last_commit = q.commit;
                        }
                        let space = if q.screen != 0 { ArtifactSpace::Screen } else { ArtifactSpace::World };
                        let band = layer::Layer(q.band.min(layer::POINTER.0 - 1));
                        plan.push(
                            band,
                            Box::new(ArtifactQuad::dmabuf(
                                entry.dmabuf.clone(),
                                space,
                                q.x,
                                q.y,
                                q.w,
                                q.h,
                                entry.sm_id.clone(),
                                entry.commit,
                            )),
                        );
                    }
                }
                Err(_) => self.has_dmabuf = Some(false),
            }
        }
    }
}

/// Every mapped window's rect in WORLD-logical units — what the plugin reasons in.
fn world_windows(cx: &mut SystemCx) -> Vec<AbiRect> {
    let Some(platform) = cx
        .platform
        .as_deref_mut()
        .and_then(|p| p.downcast_mut::<Platform>())
    else {
        return Vec::new();
    };
    let space = platform.space();
    let windows: Vec<smithay::desktop::Window> = space.elements().cloned().collect();
    windows
        .iter()
        .filter_map(|w| {
            let loc = space.element_location(w)?;
            let size = slot::expected_size(w).unwrap_or_else(|| w.geometry().size);
            Some(AbiRect { x: loc.x, y: loc.y, w: size.w, h: size.h })
        })
        .collect()
}
