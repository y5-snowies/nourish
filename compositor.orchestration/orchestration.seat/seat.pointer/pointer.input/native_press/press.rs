use smithay::backend::input::{Axis, AxisSource, ButtonState, Event, InputBackend, PointerAxisEvent, PointerButtonEvent};
use smithay::desktop::{Window, WindowSurfaceType, layer_map_for_output};
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::{AxisFrame, ButtonEvent, PointerHandle};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, Layer};
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

/// The `keyboard_interactivity` a mapped layer surface committed, found by locating the
/// surface in each output's layer map. `None` (the return) means it is not a currently
/// mapped layer surface.
fn layer_keyboard_interactivity(_loop: &Loop, surface: &WlSurface) -> Option<KeyboardInteractivity> {
    for output in _loop.inner.space_state().state.outputs() {
        let map = layer_map_for_output(output);
        if let Some(ls) = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL) {
            return Some(ls.cached_state().keyboard_interactivity);
        }
    }
    None
}

/// The topmost mapped Top/Overlay layer surface that requested `Exclusive` keyboard
/// interactivity, if any. While one exists it owns the keyboard (wlr layer-shell), so a click
/// elsewhere must not steal focus from it.
fn exclusive_layer(_loop: &Loop) -> Option<WlSurface> {
    for output in _loop.inner.space_state().state.outputs() {
        let map = layer_map_for_output(output);
        for band in [Layer::Overlay, Layer::Top] {
            for layer in map.layers_on(band).rev() {
                if layer.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive {
                    return Some(layer.wl_surface().clone());
                }
            }
        }
    }
    None
}

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

    // Surface to focus the keyboard on. For windows this is the toplevel
    // wl_surface; for layer surfaces it's the layer surface itself
    // (subject to keyboard_interactivity).
    let focus_surface: Option<WlSurface> = match &hit {
        SurfaceHit::Window { window, .. } => {
            // Window-specific: raise, activate, configure.
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
            // window (outside its bounds) is clicked and raised. Re-raise it
            // without stealing keyboard activation from the clicked window.
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
        SurfaceHit::Layer { surface, .. } => {
            // Honor the client's keyboard_interactivity on ANY layer — Background/Bottom
            // included (wlr-layer-shell allows `on_demand` on any layer, e.g. interactive
            // wallpapers). `None` never takes keyboard focus even on a click; `OnDemand` /
            // `Exclusive` are focusable on click.
            match layer_keyboard_interactivity(_loop, &surface) {
                Some(KeyboardInteractivity::None) | None => None,
                _ => Some(surface.clone()),
            }
        }
        SurfaceHit::Iced { handle, .. } => {
            // iced raise logic: bring this surface to the top of the (renderer-
            // agnostic) draw order on click; lazily registers it. Layers still
            // keep iced below/above windows — cross-layer interleaving + draw
            // consumption is the deferred ("decided later") step.
            _loop.inner.raise_drawable(uuid::Uuid::from_u128(handle.0 as u128));

            // Clears activation
            for window in _loop.inner.space_state().state.elements() {
                window.set_activated(false);
                if let Some(toplevel) = window.toplevel() {
                    toplevel.send_pending_configure();
                }
            }

            None
        }
    };

    // wlr exclusive keyboard: while a mapped Top/Overlay layer surface requested `Exclusive`
    // interactivity, it HOLDS the keyboard — a click on a window (or any other surface) must
    // NOT switch focus away from it. We disallow the switch here rather than re-asserting
    // focus every frame. This runs only when no compositor modal (overview / lock / launcher)
    // swallowed the click first, so those still own the keyboard while active.
    let focus_surface = exclusive_layer(_loop).or(focus_surface);

    // CHECK: I want to keep these calls.
    keyboard.set_focus(&mut _loop.state, focus_surface, serial);

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

    // Iced keyboard-focus target (None for non-iced hits).
    let iced_focus = match &hit {
        SurfaceHit::Iced { handle, .. } => Some(*handle),
        _ => None,
    };

    // Iced pointer-button target.
    let iced_button_target = iced_focus;
    if let Some(registry) = _loop.inner.surface_mut().registry.as_mut() {
        registry.set_keyboard_focus(iced_focus);
        registry.dispatch_button(iced_button_target, button, true);
    }

    // CHECK: I want to add iced registry calls here with the target.
    // It needs to manage that internally.
}