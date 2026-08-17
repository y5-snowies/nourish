use compositor_introspection_extraction_window_handler_traits::traits::AppHandler;
use compositor_introspection_extraction_window_hints_id::handler_id::HandlerId;
use compositor_introspection_extraction_window_hints_inferred::inferred::InferredHints;
use compositor_introspection_extraction_window_hints_source::source::Confidence;
use compositor_introspection_extraction_window_hints_values::values::ToplevelIcon;
use compositor_introspection_extraction_window_meta_types::types::MetaNode;
use std::collections::HashMap;
use std::sync::Arc;

/// Registry of handlers, keyed by `HandlerId`; one handler is marked as the
/// fallback ("Generic" in our default setup), used when no other matches.
pub struct HandlerRegistry {
    handlers: HashMap<HandlerId, Arc<dyn AppHandler>>,
    fallback: Option<HandlerId>,
}

impl HandlerRegistry {
    pub fn new() -> Self {
        Self { handlers: HashMap::new(), fallback: None }
    }

    pub fn register<H: AppHandler + 'static>(&mut self, handler: H) -> &mut Self {
        self.handlers.insert(handler.id(), Arc::new(handler));
        self
    }

    /// Mark a handler as the fallback. Must already be registered.
    pub fn set_fallback(&mut self, id: HandlerId) -> &mut Self {
        debug_assert!(self.handlers.contains_key(&id), "fallback handler must be registered first");
        self.fallback = Some(id);
        self
    }

    pub fn get(&self, id: HandlerId) -> Option<&dyn AppHandler> {
        self.handlers.get(&id).map(|a| a.as_ref())
    }

    pub fn ids(&self) -> impl Iterator<Item = HandlerId> + '_ {
        self.handlers.keys().copied()
    }
    pub fn len(&self) -> usize {
        self.handlers.len()
    }
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    /// Pick the best-matching handler for a window. Falls back to the
    /// registered fallback if nothing matches.
    pub fn detect(&self, node: &MetaNode) -> (HandlerId, Confidence) {
        let mut best: Option<(HandlerId, Confidence)> = None;
        for (id, handler) in &self.handlers {
            if Some(*id) == self.fallback {
                continue;
            }
            let d = handler.detect(node);
            if !d.matches {
                continue;
            }
            let better = match &best {
                None => true,
                Some((_, c)) => d.confidence.rank() > c.rank(),
            };
            if better {
                best = Some((*id, d.confidence));
            }
        }
        match best {
            Some(b) => b,
            None => (
                self.fallback
                    .unwrap_or_else(|| abort!("registry has no fallback handler")),
                Confidence::Low,
            ),
        }
    }

    /// Full hint extraction: base hints, detection (recorded as a hint),
    /// then only the detected handler's own hints. `icon` is the live toplevel's
    /// icon state when the caller has a surface to read it from (see
    /// [`extract_hints_with`]).
    pub fn extract_all_hints(&self, node: &MetaNode, icon: Option<&ToplevelIcon>) -> InferredHints {
        let detected = self.detect(node);
        compositor_introspection_extraction_window_handler_hints::extract::extract_all_hints(node, detected, self.get(detected.0), icon)
    }
}

impl Default for HandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract `InferredHints` from a (possibly stale) `MetaNode`. Pure data +
/// filesystem reads; does NOT touch the window or `/proc`.
///
/// This is the form the background sampler uses — it has no surface, so the
/// toplevel-icon hints are absent from its results.
pub fn extract_hints(meta: &MetaNode, registry: &HandlerRegistry) -> InferredHints {
    registry.extract_all_hints(meta, None)
}

/// [`extract_hints`] plus the extra context only a caller holding a LIVE window
/// can supply: the `xdg_toplevel_icon_v1` icon read off its surface
/// (`window.icon.toplevel::read`). Everything else is identical.
pub fn extract_hints_with(
    meta: &MetaNode,
    registry: &HandlerRegistry,
    icon: Option<&ToplevelIcon>,
) -> InferredHints {
    registry.extract_all_hints(meta, icon)
}
