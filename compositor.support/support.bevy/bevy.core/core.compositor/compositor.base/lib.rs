//! # compositor_support_bevy_core_compositor_base
//!
//! Facade: the smithay integration layer for multi-instance Bevy scenes now
//! lives in flat sibling crates; every public path this crate historically
//! exposed keeps resolving through the re-exports below.

pub mod element {
    pub use compositor_support_bevy_core_element_base::BevyRenderElement;
}
pub mod error {
    pub use compositor_support_bevy_core_error_base::{CreateError, DispatchError, ResizeError};
}
pub mod handle {
    pub use compositor_support_bevy_core_handle_base::{BevyHandle, HandleId};
}
pub mod instance {
    pub use compositor_support_bevy_core_instance_base::{BevyInstance, BevyInstanceAny};
    pub use compositor_support_bevy_core_item_base::BevyItem;
}
pub mod registry {
    pub use compositor_support_bevy_core_registry_base::BevyRegistry;
}
pub mod space {
    pub use compositor_support_bevy_core_space_base::{BevySpace, Transform};
}

pub use element::BevyRenderElement;
pub use error::{CreateError, DispatchError, ResizeError};
pub use handle::{BevyHandle, HandleId};
pub use instance::{BevyInstance, BevyItem};
pub use registry::BevyRegistry;
pub use space::{BevySpace, Transform};

pub use compositor_support_bevy_core_engine_base::{
    BevyRuntime, BevyScene, BridgeDirection, BridgeEntry, BridgeRegistry, CommandHandler,
    SharedContext, create_input_placeholder, create_output_placeholder,
};
pub use compositor_support_bevy_core_runtime_base::{
    BevySurface, texture_format, WgpuVulkanContext, create_wgpu_vulkan_context,
    import_dmabuf_to_wgpu,
};
