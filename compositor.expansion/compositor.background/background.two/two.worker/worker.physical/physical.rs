//! Which GPU the background worker runs on.
//!
//! It must be the one the COMPOSITOR renders on, not simply the first device
//! Vulkan enumerates. The worker's whole output is dmabufs the compositor
//! imports into its own renderer, and on a hybrid machine the first enumerated
//! device is routinely the other GPU — every buffer then crosses devices, which
//! is a copy at best and a per-pane, per-frame import failure at worst.
//!
//! There is NO FALLBACK. If the configured node has no Vulkan device the worker
//! fails to start and the background goes absent — which is loud, and correct.
//! Falling back to "some other GPU" would produce a background that renders
//! perfectly and then imports slowly or not at all on the compositor's device,
//! and the failure would surface as a mysteriously blank or stuttering backdrop
//! rather than as the configuration error it is.

use smithay::backend::drm::DrmNode;
use smithay::backend::vulkan::{Instance, PhysicalDevice};

pub fn select(instance: &Instance) -> Result<PhysicalDevice, String> {
    let node = compositor_model_environment_config_base::base::get().render_node.clone();
    let drm = DrmNode::from_path(&node)
        .map_err(|e| format!("worker: render node {node} is not a DRM node ({e})"))?;
    let phd = compositor_kernel_vulkan_instance_physical_base::physical::for_node(instance, drm)
        .map_err(|e| format!("worker physical: {e}"))?
        .ok_or_else(|| format!("worker: no vulkan device for the selected render node {node}"))?;
    compositor_kernel_graphic_bridge_negotiate_report::report::node("background shader worker", &node);
    Ok(phd)
}
