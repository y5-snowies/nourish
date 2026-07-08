//! Winit event dispatch -> compositor lifecycle. The Focus(false) modifier
//! hack now routes through the shared `compositor_kernel_graphic_seat_modifier_clear` entry
//! (the same problem exists on native TTY switch).

use compositor_kernel_winit_scene_compose_base::compose::WinitRenderContext;
use smithay::backend::winit::WinitEvent;
use compositor_orchestration_core_state_base::Loop;

pub fn route(event: &WinitEvent, state: &mut Loop, context: &mut WinitRenderContext) {
    match event {
        WinitEvent::Resized { size, scale_factor } => {
            // Root entry for the host's window size + fractional scale. Gated on the renderer:
            if context.vulkan_mode {
                // Vulkan: the present path stretches the logical render up to the physical
                // framebuffer, so give the compositor a STATIC, scale-1 *logical* output
                // (physical ÷ host scale) — invariant to host DPI, and the single source both
                // the compositor and compose read (compose.rs `monitor_size`). Scale is left
                // unset (Integer(1) default).
                let logical = smithay::utils::Size::<i32, smithay::utils::Physical>::from((
                    (size.w as f64 / *scale_factor).round() as i32,
                    (size.h as f64 / *scale_factor).round() as i32,
                ));
                info!("winit(vulkan): resized {size:?} host-scale {scale_factor} -> output {logical:?} @ scale 1");
                compositor_orchestration_draw_state_lifecycle::lifecycle::resize(
                    context.output.clone(),
                    logical,
                    None,
                );
            } else {
                // GLES: no stretch in the present path, so keep the ORIGINAL physical output +
                // fractional scale. This fills the host window, but the output size/scale DO
                // track host DPI — intentionally a stress-test candidate for the output-size /
                // DPI-unaware code paths.
                info!("winit(gles): resized {size:?} host-scale {scale_factor} (physical output, fractional scale)");
                compositor_orchestration_draw_state_lifecycle::lifecycle::resize(
                    context.output.clone(),
                    *size,
                    Some(smithay::output::Scale::Fractional(*scale_factor)),
                );
            }
            state.schedule_redraw();
        }
        WinitEvent::Input(input_event) => {
            // Per-event logging omitted: input is a high-frequency path.
            compositor_orchestration_draw_state_lifecycle::lifecycle::input(state, input_event);
            // (Lock engage is drained in the control-plane ping source — the lock
            // keybinding calls `ping_control()` — not here, so it never depends on
            // input arriving and doesn't poll per frame.)
        }
        WinitEvent::Focus(focused) => {
            info!("winit: focus={focused}");
            if !focused {
                info!("winit: focus lost — clearing held modifiers");
                compositor_kernel_graphic_seat_modifier_clear::clear::clear_held_modifiers(state);
            }
        }
        WinitEvent::Redraw => {
            compositor_kernel_winit_scene_compose_base::compose::draw(state, context);
        }
        WinitEvent::CloseRequested => {
            info!("winit: close requested — stopping compositor");
            compositor_orchestration_draw_state_lifecycle::lifecycle::stop(state);
        }
        _ => (),
    }
}
