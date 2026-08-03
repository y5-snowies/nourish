//! Bridge registry: entries that swap placeholder `GpuImage`s for
//! dmabuf-backed ones (installed by `BridgeRegistryPlugin`).

use std::sync::{Arc, Mutex};

use bevy::asset::Handle;
use bevy::image::Image;
use bevy::prelude::Resource;
use compositor_model_debug_instance_record::trace;

/// Label of the entry carrying the instance's render target. Named once because
/// two crates must agree on it: `boot.base` registers it, and the surface ring
/// re-points it at a different slot as it rotates.
pub const OUTPUT_LABEL: &str = "bevy_output";

#[derive(Clone)]
pub struct BridgeEntry {
    pub texture: Arc<wgpu::Texture>,
    pub handle: Handle<Image>,
    pub installed: Arc<Mutex<bool>>,
    pub direction: BridgeDirection,
    pub label: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub enum BridgeDirection {
    Output,
    Input,
}

#[derive(Resource, Default, Clone)]
pub struct BridgeRegistry {
    pub entries: Arc<Mutex<Vec<BridgeEntry>>>,
}

impl BridgeRegistry {
    pub fn push(&self, entry: BridgeEntry) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.push(entry);
        }
    }

    /// Replace the texture of an existing bridge entry, identified by label.
    /// Resets the `installed` flag so the bridge runs again on the next
    /// render-world Prepare phase, swapping in the new texture.
    ///
    /// Returns true if an entry with the given label was found and updated.
    pub fn replace_texture(&self, label: &str, new_texture: Arc<wgpu::Texture>) -> bool {
        let Ok(mut entries) = self.entries.lock() else { return false };
        for entry in entries.iter_mut() {
            if entry.label == label {
                entry.texture = new_texture;
                if let Ok(mut installed) = entry.installed.lock() {
                    *installed = false;
                }
                trace!("bridge texture replaced for '{}'", label);
                return true;
            }
        }
        false
    }
}
