use smithay::backend::input::{Axis, AxisSource, ButtonState, Event, InputBackend, PointerAxisEvent, PointerButtonEvent};
use smithay::desktop::Window;
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::{AxisFrame, ButtonEvent, PointerHandle};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{SERIAL_COUNTER, Serial};
use smithay::wayland::shell::wlr_layer::Layer;
use compositor_orchestration_core_state_base::Loop;
use compositor_support_smithay_dispatch_state_base::state::Dispatch;
use compositor_y5_surface_interface_base::hit::SurfaceHit;
use compositor_y5_window_interface_record::window::LoopWindow;

// This is only called on presses when there was a surface hit.
// Currently, I cancel wayland focus on a different place. so please provide a snippet on how to invoke the de-activation.
// This is how i do it on the other place: ( which has priority over this function )
//         _loop.inner.space_state().state.elements().for_each(|window| {
//             window.set_activated(false);
//             window.toplevel().unwrap().send_pending_configure();
//         });
//
//         // Deactivate keyboard focus
//         keyboard.set_focus(&mut _loop.state, Option::<WlSurface>::None, serial);
//         pointer.button(
//             _loop,
//             &ButtonEvent {
//                 button,
//                 state: button_state,
//                 serial,
//                 time: event.time_msec(),
//             },
//         );
//         pointer.frame(&mut _loop.state);

pub fn input_received<I: InputBackend>(
    pointer: &PointerHandle<Dispatch>,
    event: &I::PointerButtonEvent,
    _loop: &mut Loop,
    hit: SurfaceHit,
    keyboard: &KeyboardHandle<Dispatch>,
    button_state: ButtonState,
) {
    let serial = SERIAL_COUNTER.next_serial();
    let button = event.button_code();

    // Raise / activate / keyboard-focus what was hit. Shared with touch-down so a
    // tap focuses a window exactly like a click does.
    apply_focus(_loop, &hit, keyboard, serial);

    pointer.button(
        &mut _loop.state,
        &ButtonEvent {
            button,
            state: button_state,
            serial,
            time: event.time_msec(),
        },
    );
    pointer.frame(&mut _loop.state);

    // Iced pointer-button target (None for non-iced hits).
    let iced_button_target = match &hit {
        SurfaceHit::Iced { handle, .. } => Some(*handle),
        _ => None,
    };
    if let Some(registry) = _loop.inner.surface_mut().registry.as_mut() {
        registry.dispatch_button(iced_button_target, button, true);
    }
}

/// Raise, activate and move keyboard focus to the surface a press/tap landed on:
/// windows → their toplevel (raised, activated, others deactivated, fullscreen
/// re-raised); Top/Overlay layers → the layer surface; iced → raise the drawable
/// and clear window activation. Does NOT deliver a button — callers that need one
/// (pointer clicks) send it separately; touch delivers `wl_touch` instead.
pub fn apply_focus(
    _loop: &mut Loop,
    hit: &SurfaceHit,
    keyboard: &KeyboardHandle<Dispatch>,
    serial: Serial,
) {
    let focus_surface: Option<WlSurface> = match hit {
        SurfaceHit::Window { window, .. } => {
            _loop.inner.space_state_mut().state.raise_element(window, true);
            if let Some(uuid) = window.uuid() {
                _loop.inner.raise_drawable(uuid);
            }

            for w in _loop.inner.space_state().state.elements() {
                w.set_activated(w == window);
                if let Some(toplevel) = w.toplevel() {
                    toplevel.send_pending_configure();
                }
            }

            // A fullscreen window must stay above its peers even when another
            // window (outside its bounds) is raised.
            let fullscreen = _loop
                .inner.space_state()
                .state
                .elements()
                .find(|w| w.is_fullscreen() && *w != window)
                .cloned();
            if let Some(fullscreen) = fullscreen {
                _loop.inner.space_state_mut().state.raise_element(&fullscreen, false);
                if let Some(uuid) = fullscreen.uuid() {
                    _loop.inner.raise_drawable(uuid);
                }
            }

            window.toplevel().map(|t| t.wl_surface().clone())
        }
        SurfaceHit::Layer { layer, surface, .. } => match layer {
            Layer::Top | Layer::Overlay => Some(surface.clone()),
            Layer::Background | Layer::Bottom => None,
        },
        SurfaceHit::Iced { handle, .. } => {
            _loop.inner.raise_drawable(uuid::Uuid::from_u128(handle.0 as u128));
            for window in _loop.inner.space_state().state.elements() {
                window.set_activated(false);
                if let Some(toplevel) = window.toplevel() {
                    toplevel.send_pending_configure();
                }
            }
            None
        }
    };

    keyboard.set_focus(&mut _loop.state, focus_surface, serial);

    let iced_focus = match hit {
        SurfaceHit::Iced { handle, .. } => Some(*handle),
        _ => None,
    };
    if let Some(registry) = _loop.inner.surface_mut().registry.as_mut() {
        registry.set_keyboard_focus(iced_focus);
    }
}