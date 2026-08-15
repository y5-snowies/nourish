use std::collections::HashMap;
use compositor_support_bevy_core_error_base::DispatchError;
use compositor_support_bevy_core_handle_base::{BevyHandle, HandleId};
use compositor_support_bevy_core_item_base::BevyItem;
use compositor_support_bevy_core_scene_base::BevyScene;
use smithay::utils::{Physical, Point, Size};

pub fn set_instance_scale(items: &mut [BevyItem], scale: f32) {
    for item in items.iter_mut() {
        item.request_resize(item.size(), scale);
    }
}

pub fn set_location_by_id(
    items: &mut [BevyItem],
    index: &HashMap<HandleId, usize>,
    id: HandleId,
    location: Point<i32, Physical>,
) -> bool {
    match index.get(&id).and_then(|&idx| items.get_mut(idx)) {
        Some(item) => {
            item.set_location(location);
            true
        }
        None => false,
    }
}

pub fn request_resize_by_id(
    items: &mut [BevyItem],
    index: &HashMap<HandleId, usize>,
    id: HandleId,
    new_size: Size<i32, Physical>,
    instance_scale: f32,
) -> bool {
    match index.get(&id).and_then(|&idx| items.get_mut(idx)) {
        Some(item) => {
            item.request_resize(new_size, instance_scale);
            true
        }
        None => false,
    }
}

pub fn dispatch_command<S: BevyScene>(
    items: &mut [BevyItem],
    index: &HashMap<HandleId, usize>,
    handle: BevyHandle<S>,
    command: S::Command,
) -> Result<(), DispatchError> {
    let item = index
        .get(&handle.id)
        .and_then(|&idx| items.get_mut(idx))
        .ok_or(DispatchError::UnknownHandle(handle.id))?;
    let typed = item.get_mut::<S>().ok_or(DispatchError::TypeMismatch)?;
    typed.runtime_mut().queue_command(command);
    Ok(())
}

pub fn process_frame(items: &mut [BevyItem]) {
    for item in items.iter_mut() {
        item.tick();
    }
}
