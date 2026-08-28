//! The presenter's storage: its own iced registry, the message on screen, and
//! one pill per output showing it.
//!
//! Owned by the kernel-hosted `NotifySystem`, by no world, so a world switch
//! neither strands nor rebuilds the pills; nothing hit-tests this registry, so
//! they are click-through by construction. Per OUTPUT because each monitor's
//! pass sizes and places its own pill — one surface moved between passes would
//! be re-laid-out every frame and could only ever be right for one of them.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use compositor_monitor_compositor_iced_base::{IcedHandle, IcedRegistry};
use compositor_orchestration_draw_layer_base::base::Layer;
use compositor_support_system_storage_slot_base::base::Storage;
use compositor_y5_notify_view_base::NotifyUi;
use compositor_y5_surface_system_base::base::{ICED_CONTEXT, ICED_ENGINE, ICED_WORKER};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Size};

/// Pill height. Width is estimated, not measured: iced lays out INSIDE the
/// surface, so it only has to avoid clipping — overshoot is invisible behind
/// the shrink-centred pill.
pub const HEIGHT: i32 = 44;
const CHAR_W: i32 = 8;
const PAD_W: i32 = 48;
const MIN_W: i32 = 240;

pub struct Pill {
    pub handle: IcedHandle<NotifyUi>,
    pub width: i32,
}

#[derive(Default)]
pub struct Presenter {
    pub registry: Option<IcedRegistry>,
    /// The message on screen and when it was raised. One at a time.
    pub current: Option<(String, Instant)>,
    /// Its pill on each output that has drawn since it was raised, by output key.
    pub pills: HashMap<Arc<str>, Pill>,
}

impl Presenter {
    /// Take the message down: every output's pill, and the message itself.
    pub fn retire(&mut self) {
        if let Some(reg) = self.registry.as_mut() {
            for (_, pill) in self.pills.drain() {
                reg.destroy(pill.handle);
            }
        }
        self.current = None;
    }
}

/// This output's pill for the current message, created on its first pass —
/// bound to the output so its element carries the key the passes gate on.
pub fn pill<'p>(
    p: &'p mut Presenter,
    message: &str,
    output: &Arc<str>,
    size: Size<i32, Physical>,
    renderer: &mut GlesRenderer,
    node: &str,
) -> Option<&'p Pill> {
    let Presenter { registry, pills, .. } = p;
    if !pills.contains_key(output) {
        let width = ((message.len() as i32 * CHAR_W) + PAD_W).clamp(MIN_W, (size.w - 40).max(MIN_W));
        // Fully below the edge, so frame one starts off-screen not mid-slide.
        let start = Point::from(((size.w - width) / 2, size.h));
        let reg = registry.as_mut()?;
        let handle = reg
            .create_screen(node, NotifyUi::new(message.to_owned()), renderer, start, Size::from((width, HEIGHT)), Layer::GLOBAL_SCREEN.bits())
            .ok()?;
        reg.set_output_affinity_by_id(handle.id, Some(output.to_string()));
        pills.insert(Arc::clone(output), Pill { handle, width });
    }
    pills.get(output)
}

/// The presenter's registry, built on first use from the shared kernel engine —
/// a clone of the one renderer, exactly as every world's registry is. `false`
/// while the engine has not landed yet.
pub fn ensure_registry(p: &mut Presenter, kernel: &Storage) -> bool {
    if p.registry.is_some() {
        return true;
    }
    let wgpu = kernel.try_get(&ICED_CONTEXT).and_then(Clone::clone);
    let engine = kernel.try_get(&ICED_ENGINE).and_then(Clone::clone);
    let (Some(wgpu), Some(engine)) = (wgpu, engine) else { return false };
    let worker = kernel.try_get(&ICED_WORKER).and_then(Clone::clone);
    p.registry = Some(IcedRegistry::new(engine, wgpu, worker));
    true
}
