use std::sync::mpsc::Sender;

use compositor_y5_group_protocol_base::protocol::{
    GroupBufferMessage, GroupBufferMessageBBOX, GroupBufferMessageHandle, GroupBufferMessageSurface,
};
use compositor_y5_group_surface_base::message::GroupMessage;
use smithay::{backend::renderer::gles::GlesRenderer, utils::Rectangle};
use uuid::Uuid;
use compositor_orchestration_core_state_base::{Loop, Transform, state::CoordinateTrait};
use compositor_y5_surface_protocol_base::protocol::{SurfaceMessage, SurfaceMessageType};

pub fn handle(_loop: &mut Loop, renderer: &mut GlesRenderer, message: GroupBufferMessage) {
    match message {
        GroupBufferMessage::Handle(group_buffer_message_handle) => {
            handle_ice(_loop, renderer, group_buffer_message_handle);
        }
        GroupBufferMessage::BBOX(group_buffer_message_bbox) => {
            handle_bbox(_loop, group_buffer_message_bbox);
        }
        GroupBufferMessage::Surface(group_buffer_message_surface) => {
            handle_surface(_loop, group_buffer_message_surface);
        }
        GroupBufferMessage::Destroy(destroy) => {
            if let Some(registry) = _loop.inner.surface_mut().registry.as_mut() {
                for handle in &destroy.handles {
                    registry.destroy_by_id(*handle);
                }
            }
            // DrawOrder GC: drop the destroyed group surfaces (world-space, so
            // they were registered) from the draw-order authority.
            for handle in &destroy.handles {
                _loop.inner.remove_drawable(uuid::Uuid::from_u128(handle.0 as u128));
            }
        }
    }
}

fn handle_ice(_loop: &mut Loop, renderer: &mut GlesRenderer, message: GroupBufferMessageHandle) {
    for (_, (uuid, ui)) in message.new_handle.into_iter().enumerate() {
        // Due to how IcedRegistry behave(differntaly from Space<Window>) the location is not set upon render.
        // The position and size of the iced UI should always stick to windows geometry.
        // This means that either skip the visible UI - rendering nothing
        // Or debounce texture allocation in IcedRegistry for request_resize with immediate=false
        //
        let group = _loop.inner.group_mut()
            
            .get_mut(uuid.clone())
            .unwrap_or_else(|| abort!("group_exist"))
            .clone();

        let transform = crate::transform::get(_loop, &group);

        let handle = compositor_y5_surface_draw_handle::handle::load(
            _loop,
            renderer,
            ui,
            transform.into_storage_rect_physical(),
            compositor_y5_surface_draw_handle::handle::IcedSpace::World,
            compositor_orchestration_draw_layer_base::base::Layer::SCENE.bits()
                | compositor_orchestration_draw_layer_base::base::Layer::SCENE_SURFACE_GROUP.bits(),
        );

        // Scope the group (canvas slot) borrow before touching the surface slot.
        let group_id = {
            let group = _loop.inner.group_mut()
                
                .get_mut(uuid.clone())
                .unwrap_or_else(|| abort!("group requested surface but cease to exist"));
            group.Visibility = group.Visibility.with_handle(handle);
            group.id.clone()
        };

        let tx = _loop.inner.surface_mut().surface_message_buffer_channel.0.clone();
        _loop.inner.surface_mut()
            .registry
            .as_mut()
            .unwrap()
            .set_message_handler(handle, move |message: &GroupMessage| __dispatch(group_id, message, &tx));
    }
}

fn __dispatch(group_id: Uuid, p1: &GroupMessage, p2: &Sender<SurfaceMessage>) {
    match p1 {
        GroupMessage::Renamed(name) => {
            p2.send(SurfaceMessage {
                message: SurfaceMessageType::Group(GroupBufferMessage::Surface(
                    GroupBufferMessageSurface {
                        group_id,
                        name: Some(name.clone()),
                        visibility: None,
                    },
                )),
            });
        }
        GroupMessage::Collapse {} => {
            p2.send(SurfaceMessage {
                message: SurfaceMessageType::Group(GroupBufferMessage::Surface(
                    GroupBufferMessageSurface {
                        group_id,
                        name: None,
                        visibility: Some(false),
                    },
                )),
            });
        }
        GroupMessage::Show {} => {
            p2.send(SurfaceMessage {
                message: SurfaceMessageType::Group(GroupBufferMessage::Surface(
                    GroupBufferMessageSurface {
                        group_id,
                        name: None,
                        visibility: Some(true),
                    },
                )),
            });
        }
        _ => {}
    }
}

pub fn handle_bbox(_loop: &mut Loop, message: GroupBufferMessageBBOX) {
    let Some(group) = _loop.inner.group_mut().get_mut(message.group_id).cloned() else {
        return;
    };
    // let bbox = crate::interface::bbox_padded(_loop, &group).into_storage_rect_physical();
    let bbox = crate::transform::get(_loop, &group).into_storage_rect_physical();

    let Some(registry) = &mut _loop.inner.surface_mut().registry else {
        return;
    };

    // Must destroy previous handle
    let Some(handle) = group.Visibility.id() else {
        return;
    };

    registry.request_resize_by_id(handle, smithay::utils::Size::new(bbox.size.w, bbox.size.h));
    registry.set_location_by_id(handle, smithay::utils::Point::new(bbox.loc.x, bbox.loc.y));
}

/// What other modules may react to. The group module is the ONLY sender;
/// listeners (selection, future systems) subscribe and decide for themselves.
pub mod event {
    #[derive(Clone, Debug)]
    pub struct GroupEvent {
        pub group_id: uuid::Uuid,
    }

    /// Which announcement an invalidation entry maps to (internal helper).
    pub(crate) enum Kind {
        Added,
        Dismissed,
    }
    compositor_support_system_channel_token_base::y5_channel!(pub GROUP_ADDED, GROUP_ADDED_TX: GroupEvent);
    compositor_support_system_channel_token_base::y5_channel!(pub GROUP_UPDATED, GROUP_UPDATED_TX: GroupEvent);
    // No dismissal flow exists yet; the event is announced here so listeners
    // can subscribe before the first sender lands.
    compositor_support_system_channel_token_base::y5_channel!(pub GROUP_DISMISSED, GROUP_DISMISSED_TX: GroupEvent);
}

pub fn handle_surface(_loop: &mut Loop, message: GroupBufferMessageSurface) {
    // Announce the update; listeners (e.g. selection) react on the next drain.
    _loop.inner.bus.send(
        &event::GROUP_UPDATED_TX,
        event::GroupEvent { group_id: message.group_id },
    );

    // Mutate the group (canvas slot) first; dispatch to the surface slot after.
    let mut rename = None;
    {
        let Some(group) = _loop.inner.group_mut().get_mut(message.group_id) else {
            return;
        };

        if let Some(name) = message.name {
            group.name = name.clone();
            if let Some(handle) = group.Visibility.to_owned().handle() {
                rename = Some((handle, name));
            }
        }

        if let Some(visibility) = message.visibility {
            if let Some(handle) = group.Visibility.to_owned().handle() {
                let v = match visibility {
                    true => compositor_y5_group_state_base::state::GroupVisibility::Visible(None),
                    false => compositor_y5_group_state_base::state::GroupVisibility::Collapse(None),
                };

                group.Visibility = v.with_handle(handle);
            }
        }
    }

    if let Some((handle, name)) = rename {
        let registry = _loop.inner.surface_mut().registry.as_mut().unwrap();
        registry.dispatch_message(handle, GroupMessage::SetName(name));
    }

    // visibility update, so set it then call bbox above
    handle_bbox(
        _loop,
        GroupBufferMessageBBOX {
            group_id: message.group_id,
        },
    );
}
