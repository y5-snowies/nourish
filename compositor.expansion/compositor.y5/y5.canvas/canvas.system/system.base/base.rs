use compositor_support_system_buffer_token_base::y5_buffer;
use compositor_support_system_input_event_base::base::{InputEvent, InputFlow};
use compositor_support_system_input_layer_base::base as input_layer;
use compositor_support_system_storage_token_base::base::{Token, TokenMut};
use compositor_support_system_trait_system_base::base::{BufferCx, System, SystemCx, WorldBuilder};
use compositor_support_smithay_dispatch_state_base::state::Dispatch;
use compositor_y5_canvas_input_state::state::{ActiveOption, ActiveTransformCandidate, CanvasGrab, TargetOption};
use compositor_y5_canvas_state_base::state::CanvasState;
use compositor_y5_camera_transform_translate::slot;
use compositor_y5_surface_system_base::base::announce_iced_button;
use smithay::backend::input::ButtonState;
use smithay::desktop::Window;
use smithay::input::pointer::ButtonEvent;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::utils::SERIAL_COUNTER;
use std::any::Any;
use std::time::{SystemTime, UNIX_EPOCH};

pub static CANVAS: Token<CanvasState> = Token::new();
/// TRANSITIONAL pub: legacy call sites still write this slot directly until
/// their logic moves into systems/events (pass 2 of phase 4).
pub static CANVAS_MUT: TokenMut<CanvasState> = TokenMut::new(&CANVAS);

pub(crate) enum CanvasCmd {
    SetGrab(CanvasGrab),
    PanUpdating(bool),
    /// Update the in-progress SelectBox's `current_cursor` (the rim wrote it on
    /// each motion event). Grab state is our OWN slot, so this goes via our buffer.
    SetSelectBoxCursor(smithay::utils::Point<f64, smithay::utils::Logical>),
    /// Raise a window to the top of the world's draw-order authority (the actual
    /// top-level z). Goes via the buffer because the `Platform` hatch handed to
    /// the press handler only exposes the smithay `Space`, not DRAW_ORDER.
    RaiseDrawable(uuid::Uuid),
}
y5_buffer!(CANVAS_BUF: CanvasCmd);

/// Owns the canvas slot and the canvas-direct pointer handlers. `input()`
/// handles pointer PRESS (`press.rs`) and RELEASE here:
/// - RELEASE: end the active grab (Moving/Scaling -> the matching Target;
///   SelectBox -> Select; Hand -> stays), flush any resize, clear the pan flag,
///   and (unless a hand pan) send the wayland pointer button via `cx.seat` +
///   announce the iced button-up.
/// - PRESS: hit-test (`surface_under_filtered_cx` over `cx.storage`), set up the
///   grab (Scale/Move/Select/SelectBox/pan) via this system's own buffer, route
///   selection via the SELECT_REQUEST channel, drop wayland + iced keyboard focus
///   and forward the button via `cx.seat`/the surface channels, and deactivate
///   windows via `cx.platform.space()`. Returns Pass over a window (rim
///   native_press routes the click) and Consume otherwise.
#[derive(Default)]
pub struct CanvasSystem;

impl System for CanvasSystem {
    fn name(&self) -> &'static str {
        "canvas"
    }

    fn register(&mut self, builder: &mut WorldBuilder) {
        builder.storage.insert(&CANVAS, CanvasState::new());
        // Generic teleport-suppression lock (refcount) lives in world storage so any
        // system can acquire/release it; seeded to 0 here (canvas is its first client).
        builder.storage.insert(&compositor_orchestration_driver_output_base::base::TELEPORT_SUPPRESS, 0u32);
        builder.input(input_layer::WORLD);
    }

    fn input(&mut self, cx: &mut SystemCx, event: &InputEvent) -> InputFlow {
        // MOTION transforms (MOVE/SCALE/SELECTBOX) — `motion.rs`. Consumes only
        // when an active non-Hand grab is in progress; otherwise Pass (PAN is
        // CameraSystem, native motion is the rim).
        if let InputEvent::PointerMotion { x, y, screen_x, screen_y, .. } = event {
            return crate::motion::motion(cx, *x, *y, *screen_x, *screen_y);
        }

        let InputEvent::PointerButton { button, pressed, x, y } = event else { return InputFlow::Pass };
        if *pressed {
            return crate::press::press(cx, *button, *x, *y);
        }

        // End the active grab, collecting any windows whose resize must be flushed.
        let mut finish: Vec<Window> = Vec::new();
        let mut hand = false;
        let next = match &cx.storage.get(&CANVAS).Grab {
            CanvasGrab::Active(opt) => match opt {
                ActiveOption::Moving { .. } => Some(CanvasGrab::Target(TargetOption::Move)),
                ActiveOption::Scaling { candidates, .. } => {
                    if let ActiveTransformCandidate::Window(list) = candidates {
                        finish = list.iter().map(|(w, _)| w.clone()).collect();
                    }
                    Some(CanvasGrab::Target(TargetOption::Scale))
                }
                ActiveOption::SelectBox { .. } => Some(CanvasGrab::Target(TargetOption::Select { Append: true })),
                ActiveOption::Hand => {
                    hand = true;
                    None
                }
            },
            _ => None,
        };

        if let Some(grab) = next {
            cx.write(&CANVAS_BUF, CanvasCmd::SetGrab(grab));
        }
        cx.write(&CANVAS_BUF, CanvasCmd::PanUpdating(false));
        for window in finish {
            finish_resize(window);
        }

        if !hand {
            let serial = SERIAL_COUNTER.next_serial();
            let time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u32)
                .unwrap_or(0);
            if let Some(dispatch) = cx.seat.as_deref_mut().and_then(|s| s.downcast_mut::<Dispatch>())
                && let Some(pointer) = dispatch.seat.seat.get_pointer()
            {
                pointer.button(
                    dispatch,
                    &ButtonEvent { button: *button, state: ButtonState::Released, serial, time },
                );
                pointer.frame(dispatch);
            }
            // Iced button-up routes through the surface system's slot (we can't touch it).
            announce_iced_button(cx.channels, *button, false);
        }

        InputFlow::Consume
    }

    fn buffer(&mut self, cx: &mut BufferCx, message: Box<dyn Any>) {
        match *message.downcast::<CanvasCmd>().expect("canvas buffer type") {
            CanvasCmd::SetGrab(grab) => cx.storage.get_mut(&CANVAS_MUT).Grab = grab,
            CanvasCmd::PanUpdating(value) => {
                let canvas = cx.storage.get_mut(&CANVAS_MUT);
                let was = canvas.position_updating;
                canvas.position_updating = value;
                // Pan is one client of the teleport-suppression lock: acquire on pan
                // START (edge false→true), release on END (true→false). Edge-detected so
                // the unconditional PanUpdating(false) on every button release doesn't
                // decrement another system's lock when no pan was active.
                if value != was {
                    let lock = cx.storage.get_mut(&compositor_orchestration_driver_output_base::base::TELEPORT_SUPPRESS_MUT);
                    *lock = if value { lock.saturating_add(1) } else { lock.saturating_sub(1) };
                }
            }
            CanvasCmd::SetSelectBoxCursor(c) => {
                let canvas = cx.storage.get_mut(&CANVAS_MUT);
                if let CanvasGrab::Active(ActiveOption::SelectBox { current_cursor, .. }) = &mut canvas.Grab {
                    current_cursor.x = c.x;
                    current_cursor.y = c.y;
                }
            }
            CanvasCmd::RaiseDrawable(uuid) => {
                cx.storage
                    .get_mut(&compositor_support_world_order_track_base::base::DRAW_ORDER_MUT)
                    .raise(compositor_support_world_order_track_base::base::ComponentId(uuid));
            }
        }
    }
}

/// End of an interactive resize: send the final exact size now (the per-motion
/// throttle may have skipped it) and clear the Resizing state so the client
/// renders one crisp final buffer. Loop-free (smithay + `slot` only) so it is
/// callable from a Pass-1 system; the rim's window.lifecycle copy was the only
/// other caller and is gone with the rim release branch.
fn finish_resize(window: Window) {
    let Some(toplevel) = window.toplevel() else { return };
    let Some(size) = slot::expected_size(&window) else { return };
    toplevel.with_pending_state(|state| {
        state.states.unset(xdg_toplevel::State::Resizing);
        state.size = Some(size);
    });
    let _ = slot::note_resize(&window, size);
    slot::mark_resize_settling(&window);
    toplevel.send_configure();
}
