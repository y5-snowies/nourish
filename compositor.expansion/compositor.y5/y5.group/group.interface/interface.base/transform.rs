use compositor_y5_group_state_base::state::Group;
use smithay::utils::{self, Logical, Point, Rectangle, Size};
use compositor_orchestration_core_state_base::{Loop, Transform, state::CoordinateTrait};

// Get the transform of group
pub fn get(_loop: &mut Loop, group: &Group) -> Transform {
    let bbox = crate::interface::bbox_padded(_loop, group).into_storage_rect();

    let origin = Point::new(bbox.loc.x, bbox.loc.y);

    let rect: Rectangle<i32, Logical> = match group.Visibility {
        compositor_y5_group_state_base::state::GroupVisibility::Collapse(_) => {
            Rectangle::new(origin, Size::new(bbox.size.w, 250))
        }
        compositor_y5_group_state_base::state::GroupVisibility::Visible(_) => {
            Rectangle::new(origin, Size::new(bbox.size.w, bbox.size.h))
        }
    };

    // Calculate the bbox of all windows
    //

    // let loc_logical: Point<i32, Logical> = Point::new(rect.position.x, rect.position.y);
    // let size_logical: Size<i32, Logical> = Size::new(rect.size.w, rect.size.h);
    (rect, _loop.size_ctx_all()).into()
}
