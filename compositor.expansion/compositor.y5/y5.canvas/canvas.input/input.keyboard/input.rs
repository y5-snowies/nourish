use smithay::backend::input::KeyState;
use smithay::input::keyboard::{KeysymHandle, ModifiersState};
use smithay::input::pointer::CursorIcon;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::export::{ActiveOption, CanvasGrab, TargetOption};
use compositor_support_library_input_keyboard_base::keyboard::combo::KeyCombo;
use compositor_support_library_input_keyboard_base::keyboard::key::Key;

pub fn input_received(
    key_state: KeyState,
    modifiers: &ModifiersState,
    state: &mut Loop,
    key: Option<Key>,
) -> Option<bool> {
    let mut combo_hand = KeyCombo {
        modifiers: ModifiersState {
            logo: true,
            ctrl: true,
            alt: true,
            ..ModifiersState::default()
        },
        key: None,
    };

    let mut combo_scale = KeyCombo {
        modifiers: ModifiersState {
            logo: true,
            shift: true,
            ..ModifiersState::default()
        },
        key: None,
    };

    let mut combo_move = KeyCombo {
        modifiers: ModifiersState {
            logo: true,
            ..ModifiersState::default()
        },
        key: None,
    };

    let mut combo_select = KeyCombo {
        modifiers: ModifiersState {
            logo: true,
            alt: true,
            ..ModifiersState::default()
        },
        key: None,
    };

    let mut combo_select_append = KeyCombo {
        modifiers: ModifiersState {
            logo: true,
            alt: true,
            shift: true,
            ..ModifiersState::default()
        },
        key: None,
    };

    // No nested remap here. Right Ctrl is substituted for Super at the INPUT
    // (`seat.keyboard/keyboard.input::shortcut_modifiers`), so a Super binding
    // already matches in a nested session and a Ctrl binding still means Ctrl.
    // Rewriting `logo` into `ctrl` here is what used to make those two
    // indistinguishable.

    // Momentary finger-only hand tool: Super held alone (same exact-modifier
    // condition as the Move tool, but kept as its own flag — never the Move grab).
    // While set, touchpad pan/pinch own the canvas; the mouse Move tool is
    // untouched, so on a laptop the otherwise-idle two-finger gestures become
    // pan/zoom without colliding with Super+drag window moves.
    state.inner.canvas_mut().finger_pan = combo_move.matches(modifiers, key);

    let (targetting_available, is_active) = match state.inner.canvas_mut().Grab {
        CanvasGrab::None => (true, false),
        CanvasGrab::Target(_) => (true, false),
        CanvasGrab::Active(_) => (false, true),
    };

    // Its it is active, only monitor whether the logo key has been removed.
    if is_active {
        let cancels = match &state.inner.canvas_mut().Grab {
            CanvasGrab::Active(option) => match option {
                ActiveOption::Hand { .. } => {
                    // Specifically for Hand, do not set this. This way, hand tool toggles. works better
                    let cancels = combo_hand.matches(modifiers, key);

                    if cancels {
                        state.inner.canvas_mut().Grab = CanvasGrab::None;
                        // Hand tool released: drop the forced grab cursor so clients
                        // (and the default arrow) take over again.
                        state.state.seat.force_cursor = None;
                    }

                    true
                }
                ActiveOption::Moving { .. } => {
                    // CHECK: Why this rechecks whether it matches rather than whether it is no matching?
                    let cancels = !combo_move.matches(modifiers, key);

                    if cancels {
                        state.inner.canvas_mut().Grab = CanvasGrab::None;
                    }

                    true
                }
                ActiveOption::Scaling { .. } => {
                    let cancels = !combo_scale.matches(modifiers, key);

                    if cancels {
                        state.inner.canvas_mut().Grab = CanvasGrab::None;
                    }

                    true
                }
                ActiveOption::SelectBox { .. } => {
                    let cancels = !combo_select_append.matches(modifiers, key);

                    if cancels {
                        state.inner.canvas_mut().Grab = CanvasGrab::None;
                    }

                    true
                }
            },
            _ => false,
        };

        // All keys are interrupted while grab.
        return None;
    }

    // Only effect when targetting state is available
    if !targetting_available {
        // Since is_active is false, it is safe to cancel the grab state.
        state.inner.canvas_mut().Grab = CanvasGrab::None;
        return Some(true);
    }

    let match_scale = combo_scale.matches(modifiers, key);
    let match_move = combo_move.matches(modifiers, key);
    let match_select = combo_select.matches(modifiers, key);
    let match_hand = combo_hand.matches(modifiers, key);
    let match_select_append = combo_select_append.matches(modifiers, key);

    if match_scale {
        state.inner.canvas_mut().Grab = CanvasGrab::Target(TargetOption::Scale);
    } else if match_move {
        state.inner.canvas_mut().Grab = CanvasGrab::Target(TargetOption::Move);
    } else if match_select {
        state.inner.canvas_mut().Grab = CanvasGrab::Target(TargetOption::Select { Append: false });
    } else if match_select_append {
        state.inner.canvas_mut().Grab = CanvasGrab::Target(TargetOption::Select { Append: true });
    } else if match_hand {
        // Because the hand tool activated, button presses must not be sent.
        state.inner.canvas_mut().Grab = CanvasGrab::Active(ActiveOption::Hand);
        // Show the grab (open hand) cursor while the hand tool is active; the
        // per-frame render forces it over any client-set cursor.
        state.state.seat.force_cursor = Some(CursorIcon::Grab);
        // The hand tool breaks out of any pointer lock/confine the focused client
        // holds (a game's locked pointer must not pin the pan).
        compositor_orchestration_seat_pointer_input::constraint::break_constraint(state);
    } else {
        state.inner.canvas_mut().Grab = CanvasGrab::None;
    }

    // Its targetting move, still the shortcuts can remain functional when targetting move.
    if match_scale || match_move || match_select || match_select_append || match_hand {
        return None;
    }

    Some(true)
}
