use compositor_introspection_extraction_window_handler_traits::traits::AppHandler;
use compositor_introspection_extraction_window_hints_attributes_identity::attributes::DetectedHandler;
use compositor_introspection_extraction_window_hints_extract::extract::extract_base_hints;
use compositor_introspection_extraction_window_hints_id::handler_id::HandlerId;
use compositor_introspection_extraction_window_hints_inferred::inferred::InferredHints;
use compositor_introspection_extraction_window_hints_source::source::{Confidence, SourceMethod};
use compositor_introspection_extraction_window_hints_values::values::ToplevelIcon;
use compositor_introspection_extraction_window_meta_types::types::MetaNode;

/// Full hint extraction:
/// 1. Base hints (handler-agnostic).
/// 2. The detection outcome, recorded as a hint.
/// 3. The detected handler's own handler-specific hints.
///
/// Only the detected handler contributes hints, keeping the hints surface
/// aligned with the detection outcome.
pub fn extract_all_hints(
    node: &MetaNode,
    detected: (HandlerId, Confidence),
    handler: Option<&dyn AppHandler>,
    icon: Option<&ToplevelIcon>,
) -> InferredHints {
    let mut hints = extract_base_hints(node, icon);

    let (handler_id, confidence) = detected;
    hints.push::<DetectedHandler>(
        handler_id,
        SourceMethod::WaylandSurface,
        format!("detected via registry: {handler_id}"),
        confidence,
    );

    if let Some(h) = handler {
        h.extract_hints(node, &mut hints);
    }

    hints
}
