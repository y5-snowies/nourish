use compositor_orchestration_core_state_base::Loop;
use compositor_remote_message_client_base::bind::navigator::travel::Action;
use compositor_remote_message_client_base::bind::navigator::view_direction::Diagonal;
use compositor_remote_message_client_base::bind::navigator::{Travel, TravelResponse};
use compositor_support_action_camera_find_base::find::{Angle, Snap};

pub fn travel(request: Travel, state: &mut Loop) -> TravelResponse {
    match request.action {
        None => {}
        Some(a) => {
            match a {
                Action::Reset(a) => {
                    let _ = a;
                    compositor_y5_navigator_interface_base::interface::fit(state, false, false);
                }
                Action::ViewDirection(a) => {
                    let Some(direction) = a.direction else {
                        return TravelResponse {};
                    };
                    let Some(direction) = direction.direction else {
                        return TravelResponse {};
                    };

                    let direction = match direction {
                        compositor_remote_message_client_base::bind::navigator::view_direction::direction::Direction::Left(_) => compositor_support_action_camera_find_base::find::Direction::Left,
                        compositor_remote_message_client_base::bind::navigator::view_direction::direction::Direction::Right(_) => compositor_support_action_camera_find_base::find::Direction::Right,
                        compositor_remote_message_client_base::bind::navigator::view_direction::direction::Direction::Up(_) => compositor_support_action_camera_find_base::find::Direction::Up,
                        compositor_remote_message_client_base::bind::navigator::view_direction::direction::Direction::Down(_) => compositor_support_action_camera_find_base::find::Direction::Down,
                        compositor_remote_message_client_base::bind::navigator::view_direction::direction::Direction::Angle(Diagonal) => compositor_support_action_camera_find_base::find::Direction::Diagonal((Angle(Diagonal.angle as f64)), (Snap::Sixteenth)),
                    };

                    compositor_y5_navigator_interface_base::interface::move_direction(
                        state,
                        direction,
                        a.alternative,
                    );
                }
            }
        }
    }

    TravelResponse {}
}
