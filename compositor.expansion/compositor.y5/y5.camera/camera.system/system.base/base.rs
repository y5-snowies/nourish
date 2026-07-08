use compositor_support_system_buffer_token_base::y5_buffer;
use compositor_support_system_channel_token_base::y5_channel;
use compositor_support_system_input_event_base::base::{InputEvent, InputFlow, PinchPhase};
use compositor_support_system_input_layer_base::base as input_layer;
use compositor_support_system_storage_slot_base::base::Storage;
use compositor_support_system_trait_system_base::base::{BufferCx, System, SystemCx, WorldBuilder};
use compositor_support_system_world_frame_base::base::FrameTick;
use compositor_y5_viewport_state_base::state::{OutputViews, OUTPUT_VIEWS, OUTPUT_VIEWS_MUT};
use compositor_y5_canvas_input_state::state::{ActiveOption, CanvasGrab};
use compositor_y5_canvas_system_base::base::CANVAS;
use compositor_y5_group_state_base::state::{GroupVisibility, GROUP};
use compositor_y5_navigator_state_base::state::{NavRequest, State, NAVIGATOR};
use compositor_y5_surface_interface_core::hit::surface_under_filtered_cx;
use compositor_y5_window_interface_record::window::LoopWindow;
use smithay::desktop::Window;
use smithay::utils::{Logical, Point};
use std::any::Any;

/// Zoom guard (see the legacy camera.draw interface for the rationale: the
/// scene can wedge at extreme zoom until the damage interaction is root-caused).
const MIN_ZOOM: f64 = 0.02;
const MAX_ZOOM: f64 = 50.0;

/// Momentum-pan tuning (touchpad two-finger swipe).
/// - `PAN_LAUNCH_GAIN`: multiplies the swipe velocity at release. >1 makes the
///   fling punchier AND travel farther (the "snappy" knob).
/// - `PAN_FRICTION`: exponential (viscous) decay rate (1/seconds). Proportional to
///   speed, so it shapes the main glide but asymptotes — it alone leaves a long
///   faded tail.
/// - `PAN_DAMPING`: constant (Coulomb) deceleration (world-units/second²). Speed-
///   independent, so it's negligible at speed but dominates at the end, bringing
///   the coast to a firm stop in finite time — this is the knob that kills the
///   faded tail.
/// - `PAN_MIN_SPEED`: world-units/second floor below which the coast snaps to rest.
/// - `PAN_END_IDLE_FRAMES`: fallback lift detection (pan-free frames) for when no
///   terminating axis event arrives; the real touchpad path launches immediately
///   off the libinput 0,0 finger event, so this only backstops odd devices.
const PAN_LAUNCH_GAIN: f64 = 2.5;
const PAN_FRICTION: f64 = 3.6;
const PAN_DAMPING: f64 = 400.0;
const PAN_MIN_SPEED: f64 = 8.0;
const PAN_END_IDLE_FRAMES: u32 = 3;

#[derive(Clone, Copy, Debug)]
pub struct CameraMoved {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct CameraZoomed {
    pub zoom: f64,
}
/// The focused world's ACTIVE viewport pane changed (split/click/detach/remove).
/// Systems subscribe via `builder.receive(&ACTIVE_VIEWPORT_CHANGED, ...)` to
/// react — e.g. republishing the fractional scale for the new pane's zoom.
#[derive(Clone, Copy, Debug)]
pub struct ActiveViewportChanged {
    pub slot: compositor_y5_viewport_state_base::state::SlotId,
}
y5_channel!(pub CAMERA_MOVED, CAMERA_MOVED_TX: CameraMoved);
y5_channel!(pub CAMERA_ZOOMED, CAMERA_ZOOMED_TX: CameraZoomed);
y5_channel!(pub ACTIVE_VIEWPORT_CHANGED, ACTIVE_VIEWPORT_CHANGED_TX: ActiveViewportChanged);

enum CamCmd {
    SetPosition(f64, f64),
    SetZoom(f64),
    /// Canvas PAN step (Hand grab / `position_updating`), migrated from the rim
    /// `canvas.input/input.motion`. Carries the CURRENT physical screen cursor;
    /// the handler reads `position_previous` (the previous screen cursor),
    /// computes the zoom-scaled delta, advances the camera position by it, and
    /// stores the new screen cursor in `position_previous`. Done whole in the
    /// buffer so the screen accumulator (`position_previous`) is NOT clobbered by
    /// `SetPosition` (which writes it to the camera's logical position). The bool
    /// gates the actual camera move (the rim advanced position_previous on every
    /// motion event but only panned when `position_updating`).
    Pan(f64, f64, bool),
    /// Relative canvas pan from a touchpad two-finger scroll. Carries the raw
    /// scroll delta (horizontal, vertical) in screen units; the handler divides
    /// by the current zoom and advances the camera position. Unlike `Pan`, there
    /// is no screen-cursor accumulator — the libinput axis delta is already
    /// relative — so this never touches `position_previous`.
    PanBy(f64, f64),
    /// Per-frame momentum step. Converts the frame's accumulated pan into a
    /// velocity while the swipe is live, then coasts the camera along that
    /// velocity with friction once the fingers lift. Carries the frame delta
    /// (seconds) so the physics is framerate-independent.
    PanInertiaTick(f64),
    /// Touchpad lift-off (terminating 0,0 finger axis): arm an immediate coast
    /// launch on the next tick instead of waiting out the idle-frame fallback.
    PanEnd,
    /// Cancel any momentum/coast (e.g. a navigator travel takes over).
    PanStop,
}
y5_buffer!(CAM_BUF: CamCmd);

/// Owns the camera slot. Pulls the navigator's eased output each tick, applies it
/// via its buffer, announces CAMERA_MOVED / CAMERA_ZOOMED, and — because it OWNS
/// the camera — handles scroll-zoom INPUT directly: a Pass-1 system can only
/// mutate its own slot synchronously, so cursor-anchored zoom (which must
/// accumulate across rapid scroll events) lives here, not in a separate canvas
/// input system that could only defer via a channel.
#[derive(Default)]
pub struct CameraSystem {
    /// Wall-clock of the previous `update()`. `FrameTick.delta` is hardcoded to
    /// ZERO in this compositor (animations run off `Instant`), so the momentum
    /// integrator measures its own per-frame dt here.
    last_update: Option<std::time::Instant>,
    /// Last-seen active viewport slot, to fire `ACTIVE_VIEWPORT_CHANGED` on change.
    last_active: Option<compositor_y5_viewport_state_base::state::SlotId>,
}

static VIEWPORT_DOCS: &[&compositor_support_system_persist_document_entry::base::DocumentEntry] =
    &[&compositor_y5_viewport_persist_doc::base::VIEWPORTS_DOC];

impl System for CameraSystem {
    fn name(&self) -> &'static str {
        "camera"
    }

    fn register(&mut self, builder: &mut WorldBuilder) {
        builder.storage.insert(&OUTPUT_VIEWS, OutputViews::default());
        builder.input(input_layer::WORLD);
    }

    /// Persist this world's viewport layout (splits/floats + per-pane cameras)
    /// into the per-world `world.viewport` table; rehydrated at world build.
    fn documents(
        &self,
    ) -> &'static [&'static compositor_support_system_persist_document_entry::base::DocumentEntry] {
        VIEWPORT_DOCS
    }

    fn input(&mut self, cx: &mut SystemCx, event: &InputEvent) -> InputFlow {
        // Canvas PAN: when a pan is in progress (Hand grab / position_updating),
        // advance the camera by the zoom-scaled screen-cursor delta. The pan path
        // is non-intercepting in the rim (returned `false`), so we Pass — the rim
        // still runs native_motion. Synchronous via our own CAM_BUF so rapid
        // motion accumulates (each step reads the just-written position_previous).
        if let InputEvent::PointerMotion { screen_x, screen_y, .. } = event {
            // The rim updated position_previous on EVERY motion event (so the
            // first pan step has a correct delta), but only moved the camera when
            // position_updating. `do_pan` carries that gate; position_previous is
            // always advanced to the current screen cursor.
            let do_pan = cx.storage.get(&CANVAS).position_updating;
            // A manual pan is direct user intent: drop any active navigator travel
            // so the easing doesn't fight (and immediately overwrite) the drag.
            if do_pan {
                cancel_travel(cx);
            }
            cx.write(&CAM_BUF, CamCmd::Pan(*screen_x, *screen_y, do_pan));
            return InputFlow::Pass;
        }

        // Touchpad pinch: cursor-anchored zoom. Gated like axis-zoom, except a
        // pinch ALWAYS zooms the canvas in hand mode (`canvas_owns_gesture`).
        // When the canvas does not own the gesture (a window under the cursor in
        // tool mode), Pass so the rim forwards the native pinch to the client.
        // The caller latches ownership at `Begin` (this flow) and only routes
        // `Update`s here while canvas-owned.
        if let InputEvent::PointerPinch { phase, scale, x, y } = event {
            let cursor = Point::<f64, Logical>::from((*x, *y));
            if !canvas_owns_gesture(cx, cursor) {
                return InputFlow::Pass;
            }
            if *phase == PinchPhase::Update && *scale != 1.0 {
                cancel_travel(cx);
                apply_zoom(cx, cursor, *scale);
            }
            return InputFlow::Consume;
        }

        let InputEvent::PointerAxis { horizontal, vertical, x, y, finger } = event else {
            return InputFlow::Pass;
        };
        let cursor = Point::<f64, Logical>::from((*x, *y));

        // Touchpad two-finger scroll PANS the canvas (hand mode, Super-held finger
        // tool, or empty space). Over a window otherwise the canvas does not own it
        // — Pass so the rim's native_axis scrolls the client.
        if *finger {
            if !canvas_owns_gesture(cx, cursor) {
                return InputFlow::Pass;
            }
            if *horizontal != 0.0 || *vertical != 0.0 {
                // Direct user intent: drop any navigator travel so the easing
                // doesn't fight the pan.
                cancel_travel(cx);
                cx.write(&CAM_BUF, CamCmd::PanBy(*horizontal, *vertical));
            } else {
                // libinput `Finger` source terminates the scroll with a 0,0 event:
                // fingers lifted → launch the coast immediately (snappy release).
                cx.write(&CAM_BUF, CamCmd::PanEnd);
            }
            return InputFlow::Consume;
        }

        // Mouse wheel: cursor-anchored zoom. Pass over a window unless a hand tool
        // owns it — including the Super-held tool, so Super+wheel zooms anywhere.
        if !canvas_owns_gesture(cx, cursor) {
            return InputFlow::Pass;
        }
        // Canvas zoom, cursor-anchored. Synchronous via our own CAM_BUF (flushed
        // right after this input()), so the next scroll event reads the updated zoom.
        if *vertical != 0.0 {
            // A manual zoom is direct user intent: drop any active navigator travel
            // so the easing doesn't fight (and immediately overwrite) the gesture.
            cancel_travel(cx);
            let old_zoom = *cx.storage.get(&OUTPUT_VIEWS).current_views().focus_camera().transform.zoom();
            let base_step = 0.05 * old_zoom;
            let distance_from_normal = (old_zoom - 1.0).abs();
            let normal_dampener = 0.4 + (0.6 * (distance_from_normal / (distance_from_normal + 1.0)));
            let adjusted_step = base_step * normal_dampener * vertical.abs().min(1.0);
            let new_zoom = if *vertical < 0.0 { old_zoom + adjusted_step } else { (old_zoom - adjusted_step).max(0.01) };
            // Express the wheel step as a scale factor so zoom anchoring lives in
            // one place (shared with pinch).
            apply_zoom(cx, cursor, new_zoom / old_zoom);
        }
        InputFlow::Consume
    }

    fn update(&mut self, cx: &mut SystemCx, _tick: &FrameTick) {
        // Announce active-viewport changes (split/click/detach/remove) so
        // listeners (e.g. the fractional-scale publish) can recompute for the new
        // pane's camera.
        let active = cx.storage.get(&OUTPUT_VIEWS).current_views().active;
        if self.last_active != Some(active) {
            self.last_active = Some(active);
            cx.channels.send(&ACTIVE_VIEWPORT_CHANGED_TX, ActiveViewportChanged { slot: active });
        }

        // Real per-frame dt (FrameTick.delta is always ZERO here). Refreshed every
        // frame so it never goes stale; clamped so a slow/first frame can't fling.
        let now = std::time::Instant::now();
        let dt = self
            .last_update
            .replace(now)
            .map_or(1.0 / 60.0, |prev| (now - prev).as_secs_f64())
            .clamp(0.001, 0.1);

        let output = cx.storage.get(&NAVIGATOR).output;
        if let Some(output) = output {
            if let Some((x, y)) = output.position {
                cx.write(&CAM_BUF, CamCmd::SetPosition(x, y));
            }
            if let Some(zoom) = output.zoom {
                cx.write(&CAM_BUF, CamCmd::SetZoom(zoom));
            }
        }

        // Momentum pan: a navigator travel (eased view move) drives position
        // directly, so cancel any coast while it runs; otherwise step the coast.
        // Only emit when there is pan state to advance, to avoid per-frame churn.
        let nav_driving = output.is_some_and(|o| o.position.is_some());
        let camera = cx.storage.get(&OUTPUT_VIEWS).current_views().focus_camera();
        let pan_active = camera.panning
            || camera.pan_velocity.x != 0.0
            || camera.pan_velocity.y != 0.0
            || camera.pan_accum.x != 0.0
            || camera.pan_accum.y != 0.0;
        if nav_driving {
            if pan_active {
                cx.write(&CAM_BUF, CamCmd::PanStop);
            }
        } else if pan_active {
            cx.write(&CAM_BUF, CamCmd::PanInertiaTick(dt));
        }
    }

    fn buffer(&mut self, cx: &mut BufferCx, message: Box<dyn Any>) {
        let camera = cx.storage.get_mut(&OUTPUT_VIEWS_MUT).current_views_mut().focus_camera_mut();
        match *message.downcast::<CamCmd>().expect("camera buffer type") {
            CamCmd::SetPosition(x, y) => {
                // NOTE: do NOT touch `position_previous` here. It is the canvas-PAN
                // accumulator (the previous physical screen cursor, written by
                // `CamCmd::Pan` + wire.rs on pointer init) and is read ONLY by the
                // Pan arm. SetPosition is driven by zoom + navigator easing; writing
                // the LOGICAL camera position into it clobbered the screen
                // accumulator, so the next pan computed `screen - world_position`
                // and flung the camera to an unknown place (and windows out of view).
                camera.transform.position = Point::from((x, y));
                cx.channels.send(&CAMERA_MOVED_TX, CameraMoved { x, y });
            }
            CamCmd::SetZoom(zoom) => {
                let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
                camera.transform.zoom = zoom;
                // Publish to the kernel side-channel so the Vulkan renderer can
                // zoom-weight the anti-aliasing knobs (world-AA graphics config).
                compositor_developer_stats_registry_base::base::set_world_zoom(zoom);
                cx.channels.send(&CAMERA_ZOOMED_TX, CameraZoomed { zoom });
            }
            CamCmd::Pan(screen_x, screen_y, do_pan) => {
                // Zoom-scaled screen-cursor delta (rim: dx = (screen - prev)/zoom).
                let zoom = *camera.transform.zoom();
                let prev = camera.position_previous;
                let dx = (screen_x - prev.x) / zoom;
                let dy = (screen_y - prev.y) / zoom;
                // Advance position_previous for the next event (rim does this every
                // motion event, whether panning or not).
                camera.position_previous = Point::from((screen_x, screen_y));
                if do_pan {
                    // Rim: cam_position -= (dx, dy). Emit CAMERA_MOVED so the
                    // background parallax follows (the rim's camera_draw::position
                    // updated the background pan inline).
                    let pos = camera.transform.position();
                    let new_x = pos.x - dx;
                    let new_y = pos.y - dy;
                    camera.transform.position = Point::from((new_x, new_y));
                    cx.channels.send(&CAMERA_MOVED_TX, CameraMoved { x: new_x, y: new_y });
                }
            }
            CamCmd::PanBy(dx, dy) => {
                // Touchpad two-finger scroll: advance the camera by the zoom-scaled
                // scroll delta. Direction is set upstream (natural-scroll inversion
                // in the rim's axis handler), so no sign flip here.
                let zoom = *camera.transform.zoom();
                let px = camera.transform.position().x;
                let py = camera.transform.position().y;
                let wdx = dx / zoom;
                let wdy = dy / zoom;
                let nx = px + wdx;
                let ny = py + wdy;
                camera.transform.position = Point::from((nx, ny));
                cx.channels.send(&CAMERA_MOVED_TX, CameraMoved { x: nx, y: ny });
                // Feed momentum: accumulate this frame's world delta and mark the
                // swipe live (velocity is measured per-frame in PanInertiaTick).
                camera.pan_accum = Point::from((camera.pan_accum.x + wdx, camera.pan_accum.y + wdy));
                camera.panning = true;
                camera.pan_idle_frames = 0;
            }
            CamCmd::PanInertiaTick(dt) => {
                let moved = camera.pan_accum.x != 0.0 || camera.pan_accum.y != 0.0;
                if moved {
                    // Live swipe: velocity = this frame's world delta / dt.
                    if dt > 0.0 {
                        camera.pan_velocity = Point::from((camera.pan_accum.x / dt, camera.pan_accum.y / dt));
                    }
                    camera.pan_accum = Point::from((0.0, 0.0));
                    camera.pan_idle_frames = 0;
                } else if camera.panning {
                    // Pan-free frame: count toward the idle-fallback lift detection.
                    camera.pan_idle_frames += 1;
                }
                // Launch the coast when the touchpad signalled lift-off (snappy) or
                // the idle fallback fires. Apply the launch gain ONCE here, AFTER
                // velocity is measured above, so the boost survives.
                if camera.panning && (camera.pan_ending || camera.pan_idle_frames >= PAN_END_IDLE_FRAMES) {
                    camera.panning = false;
                    camera.pan_ending = false;
                    camera.pan_velocity =
                        Point::from((camera.pan_velocity.x * PAN_LAUNCH_GAIN, camera.pan_velocity.y * PAN_LAUNCH_GAIN));
                }
                if !camera.panning && (camera.pan_velocity.x != 0.0 || camera.pan_velocity.y != 0.0) {
                    let px = camera.transform.position().x;
                    let py = camera.transform.position().y;
                    let nx = px + camera.pan_velocity.x * dt;
                    let ny = py + camera.pan_velocity.y * dt;
                    camera.transform.position = Point::from((nx, ny));
                    cx.channels.send(&CAMERA_MOVED_TX, CameraMoved { x: nx, y: ny });
                    // Viscous (exponential) decay shapes the glide...
                    let decay = (-PAN_FRICTION * dt).exp();
                    let mut vx = camera.pan_velocity.x * decay;
                    let mut vy = camera.pan_velocity.y * decay;
                    // ...then a constant deceleration (Coulomb damping) firms up the
                    // end: subtract a fixed speed step along the direction of travel,
                    // so the tail dies in finite time instead of fading out.
                    let speed = vx.hypot(vy);
                    if speed > 0.0 {
                        let scale = (speed - PAN_DAMPING * dt).max(0.0) / speed;
                        vx *= scale;
                        vy *= scale;
                    }
                    camera.pan_velocity = Point::from((vx, vy));
                    if camera.pan_velocity.x.hypot(camera.pan_velocity.y) < PAN_MIN_SPEED {
                        camera.pan_velocity = Point::from((0.0, 0.0));
                    }
                }
            }
            CamCmd::PanEnd => {
                // Fingers lifted: arm the immediate coast launch (the next
                // PanInertiaTick measures the final velocity, then launches). Only
                // while a swipe is live, so a stray terminating event is harmless.
                if camera.panning {
                    camera.pan_ending = true;
                }
            }
            CamCmd::PanStop => {
                camera.pan_velocity = Point::from((0.0, 0.0));
                camera.pan_accum = Point::from((0.0, 0.0));
                camera.panning = false;
                camera.pan_idle_frames = 0;
                camera.pan_ending = false;
            }
        }
    }
}

/// Whether the canvas owns a pointer gesture at `cursor`: always in hand mode,
/// otherwise only when the cursor is NOT over a visible window (a scene-group
/// passthrough ice does not count as a window). When this is false the gesture
/// belongs to the client under the cursor (window scroll / native pinch).
fn canvas_owns_gesture(cx: &mut SystemCx, cursor: Point<f64, Logical>) -> bool {
    let canvas = cx.storage.get(&CANVAS);
    // The persistent hand tool owns every gesture. The momentary Super-held tool
    // owns touchpad pan/pinch AND the mouse wheel (zoom) — the user reserves the
    // mouse CLICK for the Move tool it shares the modifier with, but the wheel is
    // free, so Super+wheel zooms the canvas even over a window.
    let hand = matches!(canvas.Grab, CanvasGrab::Active(ActiveOption::Hand));
    if hand || canvas.finger_pan {
        return true;
    }
    let over_window = surface_under_filtered_cx(cx.storage, cursor, &|hit| {
        if let Some(window) = hit.window() {
            return window_visible(cx.storage, window);
        }
        if let Some(layer) = hit.iced_layer()
            && (layer & compositor_orchestration_draw_layer_base::base::Layer::SCENE_SURFACE_GROUP.bits()) != 0
        {
            return false;
        }
        true
    })
    .is_some();
    !over_window
}

/// Cursor-anchored zoom by a multiplicative `factor` (shared by mouse-wheel and
/// pinch). Position uses the CLAMPED zoom (the buffer clamps the same way) so the
/// cursor stays pinned to the same logical point. Synchronous via CAM_BUF so a
/// rapid gesture accumulates (each step reads the just-written zoom).
fn apply_zoom(cx: &mut SystemCx, cursor: Point<f64, Logical>, factor: f64) {
    let (old_zoom, cam_position) = {
        let camera = cx.storage.get(&OUTPUT_VIEWS).current_views().focus_camera();
        (*camera.transform.zoom(), camera.transform.position())
    };
    let new_zoom = (old_zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    let zoom_ratio = old_zoom / new_zoom;
    let new_x = cursor.x - (cursor.x - cam_position.x) * zoom_ratio;
    let new_y = cursor.y - (cursor.y - cam_position.y) * zoom_ratio;
    cx.write(&CAM_BUF, CamCmd::SetZoom(new_zoom));
    cx.write(&CAM_BUF, CamCmd::SetPosition(new_x, new_y));
}

/// Cancel an in-progress navigator travel when the user pans/zooms by hand.
/// Announced on the focused world's channel (the navigator owns the slot); only
/// fires while a `Travel` is set, so it's a no-op once cancelled. `Machine::set`
/// ignores the request while locked, so this never disturbs the lock state.
fn cancel_travel(cx: &mut SystemCx) {
    if matches!(cx.storage.get(&NAVIGATOR).state(), State::Travel(_)) {
        compositor_y5_navigator_state_base::state::request(cx.channels, NavRequest::Set(State::Idle));
    }
}

/// Window visibility via group state (mirrors `DrawWindow::visible`, reading
/// `cx.storage` instead of `&Loop`): a window in a hidden group is not visible.
fn window_visible(storage: &Storage, window: &Window) -> bool {
    let Some(window_uuid) = window.uuid() else { return true };
    let group_state = storage.get(&GROUP);
    let Some(group_uuid) = group_state.window.get(&window_uuid) else { return true };
    for group in &group_state.group {
        if &group.id != group_uuid.as_ref() {
            continue;
        }
        return matches!(group.Visibility, GroupVisibility::Visible(_));
    }
    false
}
