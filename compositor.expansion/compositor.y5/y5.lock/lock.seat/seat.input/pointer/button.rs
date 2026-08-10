use smithay::{
    backend::input::{ButtonState, InputBackend, PointerButtonEvent},
    utils::{Physical, Point, Rectangle, SERIAL_COUNTER, Size},
};
use compositor_y5_camera_transform_translate::translate;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_surface_interface_base::hit::{self, SurfaceHit};

pub fn button<I: InputBackend>(event: &<I as InputBackend>::PointerButtonEvent, _loop: &mut Loop) {
    let Some(pointer) = _loop.state.seat.seat.get_pointer() else {
        return;
    };

    let pointer_position = pointer.current_location();

    let under = compositor_y5_surface_interface_base::hit::surface_under_filtered(
        _loop,
        pointer_position,
        &|hit| {
            let Some(iced_layer) = hit.iced_layer() else {
                return false;
            };
            (iced_layer & compositor_orchestration_draw_layer_base::base::Layer::LOCK_SCENE.bits()) != 0
        },
    );

    let button = event.button_code();

    let iced_focus = match &under {
        Some(SurfaceHit::Iced { handle, .. }) => Some(*handle),
        _ => None,
    };

    let button_state = event.state();

    let pressed = ButtonState::Pressed == button_state;

    // Iced pointer-button target.
    let iced_button_target = iced_focus;
    if let Some(registry) = compositor_y5_lock_system_base::base::registry(&mut _loop.inner.worlds) {
        registry.set_keyboard_focus(iced_focus);
        registry.dispatch_button(iced_button_target, button, pressed);
    }
}
