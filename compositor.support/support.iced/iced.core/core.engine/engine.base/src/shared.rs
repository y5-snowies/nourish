//! Process-wide shared WGPU + iced_wgpu resources.
//!
//! One `SharedEngine` per process for the Iced subsystem. All `IcedRuntime`
//! instances borrow it. This is the "shared renderer" choice from the
//! design discussion: cheaper memory (one font atlas, one shape cache, one
//! pipeline cache), at the cost of `Renderer` being a `RefCell`-style
//! borrow contention point — except it isn't, because all rendering is
//! single-threaded on the compositor's render thread.

use std::cell::RefCell;
use std::sync::Arc;

use iced_core::Font;
use iced_core::Pixels;
use iced_core::renderer::Settings;
use iced_graphics::shell::Shell;
use iced_wgpu::{Engine, Renderer};

use crate::notifier::{DirtyFlags, RuntimeNotifier};

/// Shared WGPU + iced_wgpu resources.
///
/// Constructed once at compositor startup, then handed to each `IcedRuntime`
/// at creation time. Cloning is cheap (`Arc`-wrapped internally).
///
/// ## Why a single shared `Renderer`?
/// `iced_wgpu::Renderer` is a frontend over the `Engine`. It holds caching
/// state (text atlas, glyph cache, shape primitives). When rendering N
/// instances, sharing one `Renderer` is dramatically cheaper than allocating
/// N separate text atlases.
///
/// `Renderer` is not `Send`/`Sync`, but that's fine — the compositor renders
/// on one thread. The `RefCell` here makes the borrow rules explicit: callers
/// that go through `with_renderer` get an exclusive borrow for the duration
/// of the closure.
///
/// ## Why dirty flags here too?
/// One `DirtyFlags` per instance, but iced_wgpu's `Shell` (passed to the
/// `Engine`) only accepts one `Notifier`. We need a different `Notifier`
/// per instance, which means a different `Engine` and thus a different
/// `Renderer`... or, we accept that the "tick" / "request_redraw" notifications
/// are *global* across the shared renderer, and route them per-instance at
/// the runtime layer. We take the second route: one `Notifier` here, plumbed
/// to a process-wide `DirtyFlags`. Per-instance dirty is tracked separately
/// in each `IcedRuntime` based on its own event/message activity.
///
/// The trade-off: a redraw signal from one widget will redraw *every* dirty
/// instance, not just that widget's instance. In practice this is fine —
/// redraws are cheap when nothing changed (Iced's diffing skips uninteresting
/// work), and our "many small UIs" use case rarely sees the renderer-internal
/// `tick`/`request_redraw` paths fire at all (those are for animations).
pub struct SharedEngine {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    /// Format used for the iced_wgpu Engine. Must match the format of the
    /// `wgpu::TextureView` later passed to `Renderer::present`.
    pub target_format: wgpu::TextureFormat,
    /// Process-wide dirty flags; signals reach every instance.
    pub global_dirty: DirtyFlags,
    /// The shared renderer. Borrow exclusively via `with_renderer`.
    renderer: Arc<RefCell<Renderer>>,
}

impl std::fmt::Debug for SharedEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedEngine")
            .field("target_format", &self.target_format)
            .finish()
    }
}

/// How to configure the iced default text rendering.
#[derive(Debug, Clone)]
pub struct EngineSettings {
    pub default_font: Font,
    pub default_text_size: Pixels,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            default_font: Font::default(),
            default_text_size: Pixels::from(16),
        }
    }
}

impl SharedEngine {
    /// Create the shared engine. Takes already-created wgpu device/queue/adapter
    /// (typically from `compositor_monitor_runtime_surface_base::WgpuVulkanContext`).
    ///
    /// `target_format` should match the textures rendered into. Use
    /// `compositor_monitor_runtime_surface_base::TEXTURE_FORMAT` for consistency with the
    /// DMABUF round-trip path.
    pub fn new(
        adapter: &wgpu::Adapter,
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
        settings: EngineSettings,
    ) -> Self {
        let global_dirty = DirtyFlags::new();
        let notifier = RuntimeNotifier::new(global_dirty.clone());
        let shell = Shell::new(notifier);

        let engine = Engine::new(
            adapter,
            (*device).clone(),
            (*queue).clone(),
            target_format,
            None,
            shell,
        );

        info!("[DBGVUL] Create renderer");
        let renderer = Renderer::new(
            engine,
            Settings {
                default_font: settings.default_font,
                default_text_size: settings.default_text_size,
                // New in iced master; `..Default::default()` so later additions to
                // this struct don't break the build again.
                ..Default::default()
            },
        );

        Self {
            device,
            queue,
            target_format,
            global_dirty,
            renderer: Arc::new(RefCell::new(renderer)),
        }
    }

    /// Borrow the shared renderer for the duration of the closure.
    ///
    /// Panics if called recursively (i.e., from inside another `with_renderer`
    /// callback). Don't do that.
    pub fn with_renderer<R>(&self, f: impl FnOnce(&mut Renderer) -> R) -> R {
        let mut r = self.renderer.borrow_mut();
        f(&mut r)
    }

    /// Borrow the renderer mutably for an extended call. Returns a guard
    /// that releases the borrow on drop. Useful for the runtime's
    /// `UserInterface::build/update/draw` sequence, which needs the renderer
    /// for multiple consecutive calls.
    pub fn renderer_borrow(&self) -> std::cell::RefMut<'_, Renderer> {
        self.renderer.borrow_mut()
    }
}

impl Clone for SharedEngine {
    fn clone(&self) -> Self {
        Self {
            device: self.device.clone(),
            queue: self.queue.clone(),
            target_format: self.target_format,
            global_dirty: self.global_dirty.clone(),
            renderer: self.renderer.clone(),
        }
    }
}
