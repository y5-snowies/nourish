use compositor_introspection_extraction_window_hints_id::handler_id::HandlerId;

/// Marker type for [`HandlerId::of`].
pub struct Chrome;

pub fn id() -> HandlerId {
    HandlerId::of::<Chrome>()
}
