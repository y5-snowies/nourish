use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_draw_platform_base::platform::Platform;
use compositor_support_system_input_event_base::base::{InputEvent, InputFlow};

/// The input driver: every seat entry point offers the event to the ACTIVE
/// world's input bus FIRST (priority layers, then registration order; systems
/// decide Consume/Pass from state). `Pass` falls through to the legacy
/// routing. Synchronous by design — hit-testing and grabs need same-event
/// resolution.
pub fn route(l: &mut Loop, event: InputEvent) -> InputFlow {
    // Lend the seat (`&mut l.state` = the wayland `Dispatch`) DISJOINTLY from the
    // world (`l.inner`) — possible since `D = Dispatch` is a field, not the whole
    // `Loop` (document/SMITHAY_DECOUPLING.md "P3"). A Pass-1 input system can now
    // perform seat ops synchronously through `cx.seat`.
    let seat: &mut dyn std::any::Any = &mut l.state;
    // Lend the live window Space via the same `Platform` hatch update/draw use, so a
    // Pass-1 system can hit-test synchronously through `cx.platform`. No live renderer
    // exists at input time (events arrive outside a render pass), hence `None`.
    // SAFETY: `platform` is scoped to this `input()` call; the rim holds `&mut`
    // l.inner.space_state for the whole call and touches it only through this hatch.
    let gpu = l.inner.environment.GPU.clone();
    let mut platform = unsafe { Platform::new(None, &mut l.inner.space_state_mut().state, &gpu) };
    let worlds = &mut l.inner.worlds;
    let kernel = &l.inner.kernel;
    worlds.active_mut().input(kernel, &event, Some(&mut platform), Some(seat))
}
