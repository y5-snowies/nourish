use compositor_y5_group_protocol_base::protocol::{GroupBufferMessageBBOX, GroupCmd};
use compositor_y5_group_state_base::state::Group;
use smithay::{
    backend::renderer::gles::GlesRenderer,
    desktop::Window,
    utils::{Logical, Point, Rectangle, Size},
};
use uuid::Uuid;
use compositor_orchestration_core_state_base::{Loop, Transform, state::CoordinateTrait};
use compositor_y5_window_interface_record::window::LoopWindow;

// The selection mutators are now TRIGGERS: they announce a GroupCmd on the main
// world's GROUP_REQUEST channel (owned by group.protocol). The GroupSystem owns
// the slot and applies the mutation through its buffer, then lowers the
// invalidation to surface messages. Grouping is no longer mutated from the rim.
fn request(_loop: &mut Loop, cmd: GroupCmd) {
    compositor_y5_group_protocol_base::protocol::emit(
        _loop.inner.focus_channels(),
        cmd,
    );
}

/// Group the current selection (single window -> ungroup).
pub fn selection_set(_loop: &mut Loop) {
    request(_loop, GroupCmd::SelectionSet);
}

/// Join the current selection into one group.
pub fn selection_set_join(_loop: &mut Loop) {
    request(_loop, GroupCmd::SelectionSetJoin);
}

/// Drop a destroyed window from its group.
pub fn window_destroy(_loop: &mut Loop, window_uuid: Uuid) {
    request(_loop, GroupCmd::WindowDestroy(window_uuid));
}

// pub fn visibility_toggle(_loop: &mut Loop, group_uuid: Uuid, set: Option<bool>) {
//     let group = _loop
//         .inner
//         .canvas
//         .Group
//         .get_mut(group_uuid)
//         .expect("group to exist");

//     let set = if let Some(set) = set {
//         set
//     } else {
//         !matches!(group.Visibility, GroupVisibility::Visible(_))
//     };

//     let existing_handle_id = group.Visibility.id();

//     let Some(iced_registry) = &mut _loop.inner.surface_mut().registry else {
//         return;
//     };

//     group.Visibility = match set {
//         true => GroupVisibility::Visible(None),
//         false => GroupVisibility::Collapse(None),
//     };

//     if let Some(existing_handle_id) = existing_handle_id {
//         iced_registry.destroy_by_id(existing_handle_id.clone());
//     }

//     let mut buffer = GroupBufferMessageHandle { new_handle: vec![] };

//     let group_ui = match set {
//         true => GroupUi::new(compositor_y5_group_surface_base::mode::Mode::Show),
//         false => GroupUi::new(compositor_y5_group_surface_base::mode::Mode::Collapse),
//     };

//     buffer.new_handle.push((group_uuid.clone(), group_ui));

//     if buffer.new_handle.len() > 0 {
//         _loop
//             .inner
//             .surface
//             .surface_message_buffer_channel
//             .0
//             .send(SurfaceMessage {
//                 message: SurfaceMessageType::Group(GroupBufferMessage::Handle(buffer)),
//             });
//     }
// }

pub fn invalidate_bbox(_loop: &mut Loop, window_uuid: Uuid) {
    let Some(window_group) = _loop.inner.group().window.get(&window_uuid) else {
        return;
    };

    let window_group = window_group.as_ref();

    crate::protocol::handle_bbox(
        _loop,
        GroupBufferMessageBBOX {
            group_id: window_group.clone(),
        },
    );
}

pub fn windows(_loop: &mut Loop, group: &Group) -> Vec<Window> {
    _loop
        .inner.space_state()
        .state
        .elements()
        .filter_map(|f| {
            let f = f.clone();
            let uuid = f.uuid();
            if uuid.is_none() {
                return None;
            }
            let uuid = uuid.unwrap();

            if !group.window.contains(&uuid) {
                return None;
            }

            return Some(f);
        })
        .collect()
}

// BBOX with pad
/// A window's box for the group bbox = its compositor-decided **slot** (`element_location` +
/// `expected_size`) — i.e. what is actually rendered and framed by the decoration — NOT
/// `element_bbox`, which is the raw client surface extent (it includes CSD shadow / oversized
/// buffers and so overshoots the rendered window, leaving the group container bigger than the
/// window). Falls back to `element_bbox` when the compositor hasn't decided a size yet.
fn window_box(_loop: &Loop, w: &Window) -> Rectangle<i32, Logical> {
    // A fullscreen window fills the whole group, but it contributes its PRE-fullscreen rect to the
    // group bbox — otherwise the group would grow to the fullscreen (group-filling) size, which is
    // the very thing it's being sized to (a feedback loop). Uses the existing `WindowFullscreen`
    // restore rect (`window.interface.record`).
    if let Some(fs) = w.fullscreen() {
        return Rectangle::new(fs.restore_loc, fs.restore_size);
    }
    match compositor_y5_camera_transform_translate::slot::expected_size(w)
        .filter(|s| s.w > 0 && s.h > 0)
    {
        Some(size) => {
            let loc = _loop.inner.space_state().state.element_location(w).unwrap_or_default();
            Rectangle::new(loc, size)
        }
        None => _loop.inner.space_state().state.element_bbox(w).unwrap_or_default(),
    }
}

/// The group's INNER bbox — the merge of its windows' boxes, WITHOUT the padding. This is the area
/// a window fills when fullscreened inside the group, so the group keeps its margin around it.
pub fn bbox_inner(_loop: &mut Loop, group: &Group) -> Transform {
    let windows = windows(_loop, group);

    let first = window_box(_loop, &windows[0]);

    let bbox = windows
        .iter()
        .map(|w| window_box(_loop, w))
        .fold(first, |acc, b| acc.merge(b));

    (bbox, _loop.size_ctx_all()).into()
}

pub fn bbox_padded(_loop: &mut Loop, group: &Group) -> Transform {
    let windows = windows(_loop, group);

    let first = window_box(_loop, &windows[0]);

    let bbox = windows
        .iter()
        .map(|w| window_box(_loop, w))
        .fold(first, |acc, b| acc.merge(b))
        .pad(125)
        .pad_y(125);

    (bbox, _loop.size_ctx_all()).into()
}

pub trait PadExt {
    fn pad(&self, amount: i32) -> Self;
    fn pad_y(&self, amount: i32) -> Self;
}

impl PadExt for Rectangle<i32, Logical> {
    fn pad(&self, amount: i32) -> Self {
        Rectangle {
            loc: Point::from((self.loc.x - amount, self.loc.y - amount)),
            size: Size::from((self.size.w + (amount * 2), self.size.h + (amount * 2))),
        }
    }

    fn pad_y(&self, amount: i32) -> Self {
        Rectangle {
            loc: Point::from((self.loc.x, self.loc.y - amount)),
            size: Size::from((self.size.w, self.size.h + (amount))),
        }
    }
}
