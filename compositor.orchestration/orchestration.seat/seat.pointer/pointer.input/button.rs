use crate::native_press;
use smithay::backend::input::{ButtonState, Event, InputBackend, PointerButtonEvent};
use smithay::input::pointer::ButtonEvent;
use smithay::utils::{Physical, Point, SERIAL_COUNTER};
use compositor_orchestration_core_state_base::{Loop, Transform};
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_y5_surface_interface_base::hit::surface_under_filtered;
use compositor_y5_window_interface_draw::visible::DrawWindow;

pub fn button<I: InputBackend>(event: &<I as InputBackend>::PointerButtonEvent, _loop: &mut Loop) {
    // The pointer descriptor, recorded FIRST — before every early return below.
    // A release swallowed by the overview or by a popup grab is still a release,
    // and a bundle left holding a button that was let go would stay in its
    // pressed state until the next click somewhere it happened to be observed.
    compositor_orchestration_seat_pointer_publish::publish::note_button(
        event.button_code(),
        event.state() == ButtonState::Pressed,
        compositor_pipeline_abi_clock_base::base::now(),
    );

    // Overview overlay open → the overview layer handles + swallows the click
    // (menu bar / grid cell / globe); windows never receive it.
    if compositor_y5_overview_input_pointer::pointer::button::<I>(event, _loop) {
        return;
    }

    // A popup grab (a menu) owns the pointer: forward the raw button so smithay's popup
    // grab routes it into the menu chain (or dismisses on an outside press), then STOP —
    // the world bus + native_press must not fight it. Scoped by `in_popup_grab` so the
    // DnD grab keeps its own path; the flag resets once the seat is no longer grabbed.
    {
        let pointer = _loop.state.seat.seat.get_pointer().unwrap();
        if !pointer.is_grabbed() {
            _loop.state.in_popup_grab = false;
        }
        if _loop.state.in_popup_grab && pointer.is_grabbed() {
            let serial = SERIAL_COUNTER.next_serial();
            pointer.button(
                &mut _loop.state,
                &ButtonEvent {
                    button: event.button_code(),
                    state: event.state(),
                    serial,
                    time: event.time_msec(),
                },
            );
            pointer.frame(&mut _loop.state);
            return;
        }
    }

    // Viewport separator drag: a press on a separator bar starts a resize; the
    // matching release ends it. Both consume the event (no window/canvas routing).
    let cursor_world = _loop.state.seat.seat.get_pointer().unwrap().current_location();
    let cursor_phys: Point<f64, Physical> = {
        let t: Transform = ((cursor_world.x, cursor_world.y), _loop.focus_pane_context()).into();
        t.into()
    };
    match event.state() {
        ButtonState::Pressed => {
            // Separator drag hit-tests against the current output's viewport layout, so
            // it needs the output's physical bounds; the drag STATE + math live in
            // `viewport.interaction` (keeps the Orchestrator slim).
            let bounds = {
                let (pw, ph) = _loop.size_ctx_all().screen_size_physical;
                smithay::utils::Rectangle::new(
                    smithay::utils::Point::from((0, 0)),
                    smithay::utils::Size::from((pw.round() as i32, ph.round() as i32)),
                )
            };
            if compositor_y5_viewport_interaction_base::interaction::try_begin_separator(_loop.inner.output_views_mut(), bounds, cursor_phys) {
                return;
            }
            // Floating pane move (Super-drag) / resize (Super+Shift-drag) near an
            // edge. The canvas grab "tool" already encodes the held modifier
            // (incl. the nested-winit Super→Ctrl remap): Move vs Scale.
            use compositor_y5_canvas_input_state::state::{CanvasGrab, TargetOption};
            let tool = match _loop.inner.canvas().Grab {
                CanvasGrab::Target(TargetOption::Move) => Some(false),
                CanvasGrab::Target(TargetOption::Scale) => Some(true),
                _ => None,
            };
            if let Some(resize) = tool {
                if compositor_y5_viewport_interaction_base::interaction::try_begin_floating(_loop.inner.output_views_mut(), cursor_phys, resize) {
                    return;
                }
            }
        }
        ButtonState::Released => {
            if _loop.inner.output_views().separator_drag.is_some() {
                compositor_y5_viewport_interaction_base::interaction::end_separator(_loop.inner.output_views_mut());
                return;
            }
            if _loop.inner.output_views().floating_drag.is_some() {
                compositor_y5_viewport_interaction_base::interaction::end_floating(_loop.inner.output_views_mut());
                return;
            }
        }
    }

    // Click-to-activate: a press makes the pane under the cursor the keyboard
    // shortcut target (`active`). The `pointer` slot was set by the last motion.
    if event.state() == ButtonState::Pressed {
        let under_cursor = _loop.inner.viewports().pointer;
        _loop.inner.viewports_mut().active = under_cursor;
    }

    // The empty-canvas guide popups. Sits above the world bus so a press outside
    // one always dismisses it — the bus below hands presses over windows straight
    // to the client, which would leave the popup stranded on screen. Consumes
    // only the press that SUMMONS the menu (right-click / double-click on bare
    // canvas), which would otherwise also start a canvas pan underneath it.
    if event.state() == ButtonState::Pressed
        && compositor_y5_guide_interface_base::base::on_press(
            _loop,
            event.button_code(),
            event.time_msec(),
        )
    {
        return;
    }

    let pointer = &_loop.state.seat.seat.get_pointer().unwrap();
    {
        // World input bus first (phase 3); Pass falls through to legacy routing.
        let location = pointer.current_location();
        // Read the modality off the tracker rather than taking it as a parameter: the
        // seat delegate sets it from the real device before dispatching, and the touch
        // / pen emulation paths (`touch::emulate`, `tablet::tip`) reach this same
        // function through the `TouchEmu` backend AFTER their own delegate arm has
        // already recorded Touch / Pen. So it is correct for both the real and the
        // synthesized press without threading an argument through every call site.
        let ev = compositor_support_system_input_event_base::base::InputEvent::PointerButton {
            button: event.button_code(),
            pressed: event.state() == ButtonState::Pressed,
            x: location.x,
            y: location.y,
            modality: _loop.inner.touch.modality,
        };
        if compositor_orchestration_input_drive_base::drive::route(_loop, ev)
            == compositor_support_system_input_event_base::base::InputFlow::Consume
        {
            return;
        }
    }
    let event = &event;

    let button_state = event.state();
    let keyboard = &_loop.state.seat.seat.get_keyboard().unwrap();

    // PRESS and RELEASE are both handled by `CanvasSystem::input` on the world bus
    // (route() above). A `Pass` from the bus on PRESS means the cursor is over a
    // window (the system cleared selection + declined to grab), so the click is
    // routed directly to that window here via `native_press`.
    //
    // Grabbed presses take the SAME gate — hit test included — with `grabbed`
    // passed down so each step in `input_received` decides its own mid-grab
    // behavior: focus/raise/iced belong to the grab owner and skip, while the
    // wayland button still delivers THROUGH the grab (smithay's implicit click
    // grab while another button is held, or a client DnD grab). Without that
    // delivery, chorded input lost every button after the first: FPS games'
    // right-click-aim held + left-click-fire never reached the client.
    if ButtonState::Pressed == button_state {
        let grabbed = pointer.is_grabbed();
        // Overview overlay open → presentational: never deliver a click to a window OR a
        // wlr layer surface (the bus already routes menu-bar iced clicks).
        let overview_open = _loop.inner.overview().visible;
        if let Some(hit) = surface_under_filtered(_loop, pointer.current_location(), &|hit| {
            if overview_open && (hit.window().is_some() || hit.is_layer()) {
                return false;
            }
            if let Some(window) = hit.window() {
                return window.visible(_loop);
            };

            true
        }) {
            // It is directly over a window.
            native_press::press::input_received::<I>(
                pointer, event, _loop, hit, keyboard, button_state, grabbed,
            )
        }
    }
}
