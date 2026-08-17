use compositor_orchestration_core_state_base::Loop;
use compositor_remote_message_client_base::bind::selection;
use compositor_remote_message_client_base::bind::selection::{Layout, LayoutResponse};
use compositor_y5_action_canvas_layout::LayoutFlags;

pub fn layout(request: Layout, state: &mut Loop) -> LayoutResponse {
    // ALIGN enables alignment; ALIGN_CLOSE uses common points to minimize occupancy.
    let base_align_flag = LayoutFlags::NONE | LayoutFlags::ALIGN | LayoutFlags::ALIGN_CLOSE;
    let base_distribute_flag = LayoutFlags::NONE;

    let mut flags: LayoutFlags = LayoutFlags::empty();

    let mut actions: Vec<Vec<LayoutFlags>> = vec![];

    for item in request.action {
        if (item.action.is_none()) {
            continue;
        }

        let action = item.action.unwrap();
        match action {
            selection::action::Action::Align(ac) => match ac.action.unwrap() {
                selection::align::Action::Left(modifier) => {
                    flags = flags | base_align_flag | LayoutFlags::ALIGN_LEFT;
                    if modifier.stretch {
                        flags = flags | LayoutFlags::ALIGN_STRETCH_LEFT;
                    }
                }
                selection::align::Action::CenterHorizontal(modifier) => {
                    flags = flags | base_align_flag | LayoutFlags::ALIGN_CENTER_HORIZONTAL;
                }
                selection::align::Action::CenterVertical(modifier) => {
                    flags = flags | base_align_flag | LayoutFlags::ALIGN_CENTER_VERTICAL;
                }
                selection::align::Action::Right(modifier) => {
                    flags = flags | base_align_flag | LayoutFlags::ALIGN_RIGHT;
                    if modifier.stretch {
                        flags = flags | LayoutFlags::ALIGN_STRETCH_RIGHT;
                    }
                }
                selection::align::Action::Top(modifier) => {
                    flags = flags | base_align_flag | LayoutFlags::ALIGN_TOP;
                    if modifier.stretch {
                        flags = flags | LayoutFlags::ALIGN_STRETCH_TOP;
                    }
                }
                selection::align::Action::Bottom(modifier) => {
                    flags = flags | base_align_flag | LayoutFlags::ALIGN_BOTTOM;
                    if modifier.stretch {
                        flags = flags | LayoutFlags::ALIGN_STRETCH_BOTTOM;
                    }
                }
            },
            selection::action::Action::Distribute(ac) => match ac.action.unwrap() {
                selection::distribute::Action::Horizontal(modifier) => {
                    flags = flags | base_distribute_flag | LayoutFlags::DISTRIBUTE_HORIZONTALLY;
                    if modifier.start {
                        flags = flags | LayoutFlags::DISTRIBUTE_TARGET_H_START
                    } else {
                        flags = flags | LayoutFlags::DISTRIBUTE_TARGET_H_AXIS
                    }
                }
                selection::distribute::Action::Vertical(modifier) => {
                    flags = flags | base_distribute_flag | LayoutFlags::DISTRIBUTE_VERTICALLY;
                    if modifier.start {
                        flags = flags | LayoutFlags::DISTRIBUTE_TARGET_V_START
                    } else {
                        flags = flags | LayoutFlags::DISTRIBUTE_TARGET_V_AXIS
                    }
                }
            },
            selection::action::Action::Stack(ac) => match ac.action.unwrap() {
                // These operate individually.
                selection::stack::Action::Horizontal(stack) => {
                    actions.push(compositor_remote_client_handle_stack::horizontal())
                }
                selection::stack::Action::Vertical(stack) => {
                    actions.push(compositor_remote_client_handle_stack::vertical())
                }
            },
        }
    }

    if !flags.is_empty() {
        actions.push(vec![flags])
    }

    if !actions.is_empty() {
        let collected: Vec<&[LayoutFlags]> = actions.iter().map(|f| f.as_slice()).collect();
        for action in collected {
            compositor_y5_action_canvas_layout::commit_all(state, action);
        }
    }

    LayoutResponse {}
}
