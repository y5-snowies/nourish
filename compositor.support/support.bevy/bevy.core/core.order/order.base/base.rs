use std::collections::HashMap;
use compositor_support_bevy_core_handle_base::HandleId;
use compositor_support_bevy_core_item_base::BevyItem;

pub fn raise(items: &mut Vec<BevyItem>, index: &mut HashMap<HandleId, usize>, id: HandleId) {
    let Some(&idx) = index.get(&id) else {
        return;
    };
    if idx == items.len() - 1 {
        return;
    }
    let item = items.remove(idx);
    items.push(item);
    for (other_id, i) in index.iter_mut() {
        if *other_id == id {
            *i = items.len() - 1;
        } else if *i > idx {
            *i -= 1;
        }
    }
}

pub fn lower(items: &mut Vec<BevyItem>, index: &mut HashMap<HandleId, usize>, id: HandleId) {
    let Some(&idx) = index.get(&id) else {
        return;
    };
    if idx == 0 {
        return;
    }
    let item = items.remove(idx);
    items.insert(0, item);
    for (other_id, i) in index.iter_mut() {
        if *other_id == id {
            *i = 0;
        } else if *i < idx {
            *i += 1;
        }
    }
}
