//! `NotifySystem` — the notification pill as a KERNEL system.
//!
//! Hosted by `WorldManager::kernel`, not by any world: it ticks every frame
//! whatever world is active — the main world, a picker-created one, the picker
//! (never the lock screen: its pass does not tick, so the queue waits there) —
//! and owns the pill outright (the presenter is a field), so a world switch
//! mid-slide is seamless and no world carries a copy.
//!
//! The queue is the `NOTIFY` slot: raised over `NOTIFY_REQUEST`, appended in
//! `buffer()`, read in `update()` — which renders through the platform hatch,
//! the one place a system holds the GLES renderer — and popped through the
//! buffer again once the presenter has raised it. `draw()` emits the result at
//! `layer::NOTIFY`, above everything else in the frame, pointer included.

use std::any::Any;

use compositor_orchestration_draw_platform_base::platform::Platform;
use compositor_support_system_buffer_token_base::y5_buffer;
use compositor_support_system_trait_system_base::base::{BufferCx, System, SystemCx, WorldBuilder};
use compositor_support_system_world_frame_base::base::{self as layer, FramePlan, FrameTick};
use compositor_y5_notify_present_base::base::{self as present, NotifyFrame};
use compositor_y5_notify_present_registry::registry::Presenter;
use compositor_y5_notify_state_base::base::{NotifyState, NOTIFY, NOTIFY_MUT, NOTIFY_REQUEST};

enum NotifyCmd {
    Queue(String),
    /// The presenter raised the front message; drop it from the queue.
    Taken,
}
y5_buffer!(NOTIFY_BUF: NotifyCmd);

#[derive(Default)]
pub struct NotifySystem {
    presenter: Presenter,
    /// This output's elements (the frame context is per output), `update()` → `draw()`.
    frame: Option<NotifyFrame>,
}

impl System for NotifySystem {
    fn name(&self) -> &'static str {
        "notify"
    }

    fn register(&mut self, builder: &mut WorldBuilder) {
        builder.storage.insert(&NOTIFY, NotifyState::default());
        builder.receive(&NOTIFY_REQUEST, Self::on_request);
    }

    fn update(&mut self, cx: &mut SystemCx, _tick: &FrameTick) {
        let screen = cx.kernel.get(&compositor_orchestration_smithay_data_base::data::SCREEN).clone();
        let next = cx.storage.get(&NOTIFY).queue.front().cloned();
        let (kernel, presenter) = (cx.kernel, &mut self.presenter);
        let (frame, taken) = cx
            .platform
            .as_deref_mut()
            .and_then(|p| p.downcast_mut::<Platform>())
            .map(|p| {
                let node = p.render_node().to_owned();
                p.renderer().map(|r| present::tick(presenter, next, kernel, r, &node, &screen))
            })
            .flatten()
            .unwrap_or((None, false));
        self.frame = frame;
        if taken {
            cx.write(&NOTIFY_BUF, NotifyCmd::Taken);
        }
    }

    fn draw(&mut self, _cx: &mut SystemCx, plan: &mut FramePlan) {
        if let Some(frame) = self.frame.take() {
            plan.push(layer::NOTIFY, Box::new(frame));
        }
    }

    fn buffer(&mut self, cx: &mut BufferCx, message: Box<dyn Any>) {
        let queue = &mut cx.storage.get_mut(&NOTIFY_MUT).queue;
        match *message.downcast::<NotifyCmd>().expect("notify buffer type") {
            NotifyCmd::Queue(m) => queue.push_back(m),
            NotifyCmd::Taken => { queue.pop_front(); }
        }
    }
}

impl NotifySystem {
    fn on_request(&mut self, cx: &mut SystemCx, message: &String) {
        cx.write(&NOTIFY_BUF, NotifyCmd::Queue(message.clone()));
    }
}
