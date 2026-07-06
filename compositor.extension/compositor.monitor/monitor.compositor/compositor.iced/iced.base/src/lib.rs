//! # compositor_monitor_compositor_iced_base
//!
//! Smithay integration layer for the multi-instance deferred-Iced subsystem.
//!
//! ## Public types
//!
//! - [`IcedRegistry`]: holds all Iced instances. Does **not** store
//!   camera state — render and hit-test calls take `Transform` +
//!   `output_size` as parameters. This mirrors how `window.render_elements`
//!   takes screen position and scale as arguments.
//! - [`IcedItem`]: type-erased wrapper yielded by `iter()`/`iter_mut()`.
//!   Common ops directly; typed access via `is::<U>`/`get::<U>`/`get_mut::<U>`.
//!   Helper geometry methods (`screen_location`, `screen_rect`,
//!   `contains_screen_point`, `local_coords`) take `(transform, output_size)`.
//! - [`IcedInstance<U>`]: concrete per-UI-type access.
//! - [`IcedHandle<U>`]: typed handle.
//! - [`Transform`]: camera transform with centered-origin world↔screen math.
//! - [`IcedSpace`]: per-item space (World or Screen).
//!
//! ## Render flow
//!
//! Each frame, mirroring how window rendering works:
//!
//! ```ignore
//! let elements = registry.render_all(
//!     gles,
//!     Transform { position: cam.position, zoom: cam.zoom },
//!     Size::from((output_w as f64, output_h as f64)),
//! )?;
//! ```
//!
//! ## Input flow
//!
//! ```ignore
//! if let Some(handle) = registry.dispatch_pointer_at(point, &transform, output_size) {
//!     // swallow — pointer is over an iced item
//! }
//! ```
//!
//! ## Hit-testing your way
//!
//! If you want different hit-test semantics (e.g., screen items only,
//! or restrict to certain UI types), iterate yourself:
//!
//! ```ignore
//! let hit = registry.iter()
//!     .rev()
//!     .filter(|item| item.is::<PlaceholderUi>())
//!     .find(|item| item.contains_screen_point(point, &transform, output_size))
//!     .map(|item| item.handle_id());
//! ```

#[macro_use]
extern crate compositor_developer_debug_instance_record;

pub mod element;
pub mod error;
pub mod handle;
pub mod input;
pub mod instance;
pub mod registry;
pub mod space;

pub use element::IcedRenderElement;
pub use error::{CreateError, DispatchError, ResizeError};
pub use handle::{HandleId, IcedHandle};
pub use instance::{IcedInstance, IcedItem};
pub use registry::{IcedRegistry, PaneView};
pub use space::{IcedSpace, Transform};

pub use compositor_support_iced_core_engine_base::{
    DirtyFlags, EngineSettings, IcedEvent, IcedRuntime, IcedUi, MessageHandler, SharedEngine,
    Theme,
};
pub use compositor_monitor_runtime_surface_base::{
    TEXTURE_FORMAT, WgpuVulkanContext, create_wgpu_vulkan_context,
};
