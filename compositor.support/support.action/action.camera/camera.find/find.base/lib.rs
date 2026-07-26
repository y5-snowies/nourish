#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod find {
    pub use compositor_support_action_camera_find_flags::WindowFinderFlags;
    pub use compositor_support_action_camera_find_angle::{Angle, Octant, Snap};
    pub use compositor_support_action_camera_find_direction::Direction;
    pub use compositor_support_action_camera_find_window::{WindowEntry, WindowId, cmp_f64};
    pub use compositor_support_action_camera_find_passes::{BasePass, EndpointPass, PassResult};
    pub use compositor_support_action_camera_find_run::find;
}
