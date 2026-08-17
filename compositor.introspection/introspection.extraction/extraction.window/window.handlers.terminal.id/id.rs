use compositor_introspection_extraction_window_hints_id::handler_id::HandlerId;

/// Marker type for [`HandlerId::of`].
pub struct Terminal;

pub fn id() -> HandlerId {
    HandlerId::of::<Terminal>()
}
