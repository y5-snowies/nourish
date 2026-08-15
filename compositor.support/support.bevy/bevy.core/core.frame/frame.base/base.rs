use compositor_support_bevy_core_context_base::WgpuVulkanContext;
use compositor_support_bevy_core_element_base::BevyRenderElement;
use compositor_support_bevy_core_error_base::ResizeError;
use compositor_support_bevy_core_handle_base::HandleId;
use compositor_support_bevy_core_item_base::BevyItem;
use compositor_support_bevy_core_space_base::{BevySpace, Transform};
use compositor_model_debug_instance_record::warn;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Size};

pub fn apply_pending_resizes(
    items: &mut [BevyItem],
    wgpu_ctx: &WgpuVulkanContext,
    render_node: &str,
    gles: &mut GlesRenderer,
) -> Result<usize, ResizeError> {
    let mut applied = 0;
    // Read ONCE for the whole pass, not once per item: the value cannot change
    // mid-frame, and the global is behind an `RwLock`.
    let want = compositor_model_environment_interface_base::base::get().depth();
    for item in items.iter_mut() {
        match item.apply_pending_resize(render_node, wgpu_ctx, gles) {
            Ok(true) => applied += 1,
            Ok(false) => {}
            Err(e) => warn!("resize failed handle={:?} error={e:?}", item.handle_id()),
        }
        // Ring depth tracks the live setting. After the resize, so a frame that
        // does both reallocates once at the new size rather than twice.
        if let Err(e) = item.sync_depth(render_node, wgpu_ctx, gles, want) {
            warn!("ring resize failed handle={:?} error={e:?}", item.handle_id());
        }
    }
    Ok(applied)
}

/// Cache the camera transform; bump commits of World items when it changed.
pub fn cache_camera_and_bump(
    items: &mut [BevyItem],
    last_transform: &mut Transform,
    last_output_size: &mut Size<f64, Physical>,
    transform: Transform,
    output_size: Size<f64, Physical>,
) {
    let changed = *last_transform != transform || *last_output_size != output_size;
    *last_transform = transform;
    *last_output_size = output_size;
    if changed {
        for item in items.iter_mut() {
            if item.space() == BevySpace::World {
                item.bump_commit();
            }
        }
    }
}

pub fn hit_test(
    items: &[BevyItem],
    point: Point<f64, Physical>,
    transform: &Transform,
    output_size: Size<f64, Physical>,
) -> Option<HandleId> {
    for item in items.iter().rev() {
        if item.contains_screen_point(point, transform, output_size) {
            return Some(item.handle_id());
        }
    }
    None
}

pub fn elements(
    items: &[BevyItem],
    transform: &Transform,
    output_size: Size<f64, Physical>,
    layer: u64,
) -> Vec<BevyRenderElement> {
    items
        .iter()
        .rev()
        .filter_map(|i| {
            let in_layer = (i.layer & layer) != 0;
            if !in_layer {
                return None;
            }

            i.element_in(transform, output_size)
        })
        .collect()
}
