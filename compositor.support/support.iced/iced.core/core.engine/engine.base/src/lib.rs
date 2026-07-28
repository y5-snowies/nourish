//! # compositor_support_iced_core_engine_base
//!
//! Iced runtime, stripped of `Application`, async tasks, animations, and
//! window-system entanglement. One `IcedRuntime<U>` per UI instance,
//! sharing wgpu + iced_wgpu resources via `SharedEngine`.
//!
//! ## Decoupling
//!
//! This crate depends on `iced_*` and `wgpu`, nothing else. It knows
//! nothing about Smithay, Wayland, or DMABUF. Callers (typically the
//! `compositor_monitor_compositor_iced_base` integration crate) supply a `wgpu::TextureView`
//! to render into and translated `iced_core::Event` values to dispatch.
//!
//! ## Module layering
//!
//! ```text
//! runtime.rs        IcedRuntime<U>  — per-instance state + tick + render
//! shared.rs         SharedEngine    — process-wide wgpu/renderer holder
//! notifier.rs       DirtyFlags + Notifier impl
//! ui.rs             IcedUi trait    — what UIs implement
//! error.rs          init errors
//! ```
//!
//! ## Minimum example
//!
//! ```ignore
//! // Once:
//! let shared = SharedEngine::new(
//!     &adapter, Arc::new(device), Arc::new(queue),
//!     compositor_monitor_runtime_surface_base::TEXTURE_FORMAT,
//!     EngineSettings::default(),
//! );
//!
//! // Per UI:
//! let mut rt = IcedRuntime::new(my_ui, shared.clone(), (800, 600), 1.0);
//! rt.set_message_handler(|msg: &MyMessage| { /* observe */ });
//!
//! // Per frame:
//! rt.queue_event(iced_event);
//! if rt.tick() || rt.is_dirty() {
//!     rt.render_into(&texture_view);
//! }
//! ```

#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod error;
pub mod notifier;
pub mod runtime;
pub mod shared;
pub mod ui;

pub use error::EngineInitError;
pub use notifier::{DirtyFlags, RuntimeNotifier};
pub use runtime::{IcedRuntime, MessageHandler};
pub use shared::{EngineSettings, SharedEngine};
pub use ui::IcedUi;

// Re-export iced types callers commonly need so they can avoid pulling
// iced_core in directly (especially convenient for the compositor crate).
pub use iced_core::{
    Color, Element, Event as IcedEvent, Pixels, Point as IcedPoint, Size as IcedSize, Theme,
    keyboard, mouse,
};
pub use iced_wgpu::Renderer;

// Full re-export so callers can build custom widgets / construct events without
// taking a direct `iced_core` dependency (keeps the DEPS-lint surface small).
pub use iced_core;
