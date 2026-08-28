//! When a notification is on screen and where it sits, one output of one frame
//! at a time.
//!
//! World-free: the pills live in the presenter's own registry
//! (`present.registry`, owned by the kernel-hosted `NotifySystem`), so a world
//! switch — or the picker — neither strands nor rebuilds them; every frame's
//! pass takes this output's elements from the kernel host's plan and draws them
//! on top.
//!
//! One message at a time, shown on EVERY output: the system hands in the FRONT
//! of its queue, and the presenter reports when it has raised it — only once the
//! previous message has fully slid out everywhere — so the system pops it.

use std::time::{Duration, Instant};

use compositor_monitor_compositor_iced_base::{IcedRenderElement, Transform};
use compositor_orchestration_draw_layer_base::base::Layer;
use compositor_orchestration_smithay_data_base::data::ScreenContext;
use compositor_support_system_storage_slot_base::base::Storage;
use compositor_y5_notify_present_registry::registry::{self as registry, Presenter};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::Point;

/// How long a message rests on screen, and the slide in/out duration.
pub const TTL: Duration = Duration::from_secs(5);
const SLIDE: f32 = 0.22;
/// The gap between the resting pill and the bottom edge.
const MARGIN: i32 = 36;

/// What the presenter hands a frame: this output's pill elements, and whether
/// it is still moving (the driver keeps frames coming while it is).
pub struct NotifyFrame {
    pub elements: Vec<IcedRenderElement>,
    pub animating: bool,
}

/// 1.0 rests above the bottom margin, 0.0 is hidden below the edge. Ease-out
/// cubic in, the same curve reversed out.
fn shown(elapsed: f32, ttl: f32) -> f32 {
    let ease = |t: f32| 1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3);
    match elapsed {
        e if e < SLIDE => ease(e / SLIDE),
        e if e < ttl => 1.0,
        e => 1.0 - ease((e - ttl) / SLIDE),
    }
}

/// Advance the message for one output of one frame: retire a finished one,
/// raise `next` (the queue front) if the slot is free, place this output's pill
/// and render it. Returns the frame (`None` while nothing is on screen — the
/// common case, and a no-op) and whether `next` was raised this call.
pub fn tick(
    p: &mut Presenter,
    next: Option<String>,
    kernel: &Storage,
    renderer: &mut GlesRenderer,
    render_node: &str,
    screen: &ScreenContext,
) -> (Option<NotifyFrame>, bool) {
    let ttl = TTL.as_secs_f32();
    if p.current.as_ref().is_some_and(|(_, raised)| raised.elapsed().as_secs_f32() > ttl + SLIDE) {
        p.retire();
    }
    let mut taken = false;
    if p.current.is_none() {
        // Registry before the message: a failed build must not eat the queue.
        let Some(message) = next.filter(|_| registry::ensure_registry(p, kernel)) else { return (None, false) };
        p.current = Some((message, Instant::now()));
        taken = true;
    }
    let (message, raised) = p.current.clone().expect("raised above");
    let Some(pill) = registry::pill(p, &message, &screen.output, screen.size, renderer, render_node) else { return (None, taken) };
    let (handle, width) = (pill.handle, pill.width);
    let size = screen.size;
    let travel = (registry::HEIGHT + MARGIN) as f32 * shown(raised.elapsed().as_secs_f32(), ttl);
    let Some(reg) = p.registry.as_mut() else { return (None, taken) };
    reg.set_location(handle, Point::from(((size.w - width) / 2, size.h - travel.round() as i32)));
    // Every pill renders (the registry is one), but only THIS output's is handed back.
    let elements = reg
        .render_all(render_node, renderer, Transform { zoom: 1.0, position: Point::new(0.0, 0.0) }, size.to_f64(), Layer::GLOBAL_SCREEN.bits())
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.output.as_deref() == Some(&*screen.output))
        .collect();
    (Some(NotifyFrame { elements, animating: true }), taken)
}
