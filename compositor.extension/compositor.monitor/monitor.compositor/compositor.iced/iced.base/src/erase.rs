//! Erase `IcedRuntime<U>`'s UI type so the worker can hold a heterogeneous set.
//!
//! The orphan rules allow this: `AnyUi` is ours, `IcedRuntime` is foreign. The
//! snapshot function is carried as a field rather than required by the trait, so
//! the UIs the compositor never reads back — which is most of them — need no
//! change at all.

use crate::worker::AnyUi;
use compositor_support_iced_core_engine_base::{IcedEvent, IcedRuntime, IcedSnapshot, IcedUi};
use std::any::Any;
use std::sync::Arc;

pub struct Erased<U: IcedUi> {
    runtime: IcedRuntime<U>,
    /// `None` unless the UI opted into [`IcedSnapshot`].
    snap: Option<fn(&U) -> Arc<dyn Any + Send + Sync>>,
}

impl<U: IcedUi> Erased<U> {
    pub fn new(runtime: IcedRuntime<U>) -> Self {
        Self { runtime, snap: None }
    }

    /// For a UI the compositor reads synchronously. `snapshot()` then returns a
    /// `Send` copy each frame, which the compositor reads instead of borrowing.
    pub fn with_snapshot(runtime: IcedRuntime<U>) -> Self
    where
        U: IcedSnapshot,
    {
        Self { runtime, snap: Some(|ui| Arc::new(ui.snapshot()) as Arc<dyn Any + Send + Sync>) }
    }
}

impl<U: IcedUi> AnyUi for Erased<U> {
    fn queue_event(&mut self, event: IcedEvent) {
        self.runtime.queue_event(event);
    }
    fn queue_message(&mut self, message: Box<dyn Any + Send>) {
        match message.downcast::<U::Message>() {
            Ok(m) => self.runtime.queue_message(*m),
            // Routed to the wrong instance — a registry bug. Worth a log, not
            // worth killing the worker thread over.
            Err(_) => compositor_model_debug_instance_record::warn!(
                "iced worker: message type mismatch; dropped"
            ),
        }
    }
    fn tick(&mut self) -> bool {
        self.runtime.tick()
    }
    fn is_dirty(&self) -> bool {
        self.runtime.is_dirty()
    }
    fn render_into(&mut self, view: &wgpu::TextureView) {
        self.runtime.render_into(view);
    }
    fn resize(&mut self, size: (u32, u32), scale: f32) {
        self.runtime.resize(size, scale);
    }
    fn acknowledge_frame(&mut self) {
        self.runtime.acknowledge_frame();
    }
    fn set_handler(&mut self, mut handler: Box<dyn FnMut(&dyn Any) + Send>) {
        self.runtime.set_message_handler(move |m: &U::Message| handler(m as &dyn Any));
    }
    fn snapshot(&self) -> Option<Arc<dyn Any + Send + Sync>> {
        self.snap.map(|f| f(&self.runtime.ui))
    }
}
