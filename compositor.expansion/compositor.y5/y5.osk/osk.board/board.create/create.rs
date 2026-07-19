//! Per-frame reconciler for the on-screen keyboard. Creates the Screen-space OSK
//! (a bottom bar) in the active world's registry when `OSK.open`, destroys it
//! otherwise, and pushes the live sticky-modifier state to highlight the mod keys.
//! Per-(world,output) keyed + output-bound, mirroring the touch pane / FPS overlay.
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc::Sender;

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Rectangle, Size};

use smithay::input::keyboard::Keycode;
use smithay::wayland::text_input::TextInputSeat;

use compositor_support_world_order_track_base::base::DrawLayer;
use compositor_monitor_compositor_iced_base::{HandleId, IcedHandle, IcedSpace};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_y5_navigator_state_base::state::{NavRequest, State as NavState};
use compositor_y5_navigator_travel_state::state::{Target, Travel};
use compositor_y5_osk_board_state::state::{OSK, OSK_MUT};
use compositor_y5_osk_board_view::view::{char_keycodes, Osk, OskMessage};
use compositor_y5_surface_protocol_base::protocol::{SurfaceMessage, SurfaceMessageType};

/// Compute a label per character keycode for the ACTIVE xkb layout (globe switch
/// re-labels). Level-0 (unshifted) syms, so e.g. a Hebrew layout shows Hebrew letters.
fn compute_labels(state: &mut Loop) -> Vec<(u32, String)> {
    let Some(kb) = state.state.seat.seat.get_keyboard() else {
        return Vec::new();
    };
    kb.with_xkb_state(&mut state.state, |ctx| {
        let xkb = ctx.xkb().lock().unwrap();
        let layout = xkb.active_layout();
        char_keycodes()
            .into_iter()
            .filter_map(|kc| {
                xkb.raw_syms_for_key_in_layout(Keycode::new(kc), layout)
                    .first()
                    .and_then(|s| s.key_char())
                    .map(|c| (kc, c.to_string()))
            })
            .collect()
    })
}

/// The active field's surrounding text (from the client's `set_surrounding_text`,
/// captured in the vendored text-input handler) with a caret marking the cursor —
/// the OSK's live preview line. Empty when no field/text is reported.
fn compute_preview(state: &Loop) -> String {
    let ti = state.state.seat.seat.text_input();
    // Never preview a sensitive field (password/PIN) — from surrounding text OR echo.
    if ti.is_sensitive_field() {
        return String::new();
    }
    match ti.surrounding_text() {
        Some((text, cursor, _anchor)) => {
            let c = cursor as usize;
            if c <= text.len() && text.is_char_boundary(c) {
                let mut s = String::with_capacity(text.len() + 1);
                s.push_str(&text[..c]);
                s.push('|');
                s.push_str(&text[c..]);
                s
            } else {
                text
            }
        }
        // No client surrounding text (an iced field like the launcher search, or a
        // client that doesn't report it) → fall back to the OSK's own local echo.
        None => state.inner.kernel.get(&OSK).echo.clone(),
    }
}

/// A stable identity for the currently focused text sink, tagged by kind so the ranges
/// can't collide. Used to clear the local echo when the focus / IME target changes.
/// - An iced surface (launcher / settings) → its registry keyboard-focus handle.
/// - A client text field → the text-input FIELD GENERATION (bumped on every
///   enable/disable/leave), not the surface id, so a switch between two inputs that
///   share one `WlSurface` (e.g. a web page) is detected too.
fn focus_key(state: &Loop) -> Option<u64> {
    if let Some(reg) = state.inner.surface().registry.as_ref() {
        if let Some(h) = reg.keyboard_focus() {
            return Some(0x1 << 62 | (h.0 as u64));
        }
    }
    let ti = state.state.seat.seat.text_input();
    if ti.focus().is_some() {
        return Some(0x2 << 62 | ti.field_generation());
    }
    None
}

/// A Wayland client text field is active (text-input-v3 enabled on the focused
/// surface) — the same "show a keyboard" signal an IME sees. This is the OSK's
/// unpinned auto-show trigger (combined with the last input being touch/pen).
fn text_input_active(state: &Loop) -> bool {
    let mut active = false;
    state
        .state
        .seat
        .seat
        .text_input()
        .with_active_text_input(|_ti, _surface| active = true);
    active
}

type Mods = (bool, bool, bool, bool);
type Labels = Vec<(u32, String)>;

/// Bookkeeping for a live OSK surface: last dispatched state (mods / labels /
/// field-preview, for de-dup) plus the placement mode (a screen↔world switch
/// recreates the surface).
#[derive(Clone)]
struct Live {
    id: HandleId,
    mods: Mods,
    labels: Labels,
    world: bool,
    preview: String,
    flash: Option<u32>,
}

thread_local! {
    static SURF: RefCell<HashMap<(u128, String), Live>> = RefCell::new(HashMap::new());
}

/// OSK world-mode base size (logical); counter-scaled so it holds constant on-screen.
const OSK_WORLD_W: f64 = 1000.0;
const OSK_WORLD_H: f64 = 380.0;
/// Zoom floor (mirrors the selection overlay's `MIN_ZOOM`) so the world dmabuf can't
/// explode when zoomed far out.
const OSK_MIN_ZOOM: f64 = 0.15;

/// World-mode surface size (base / zoom) so it keeps a constant on-screen size.
fn world_size(zoom: f64, mult: f64) -> Size<i32, Physical> {
    let z = zoom.max(OSK_MIN_ZOOM);
    Size::new(
        (OSK_WORLD_W * mult / z).round().max(1.0) as i32,
        (OSK_WORLD_H * mult / z).round().max(1.0) as i32,
    )
}

/// World-mode iced counter-scale (1 / zoom) so content lays out at native size.
fn world_scale(zoom: f64) -> f32 {
    (1.0 / zoom.max(OSK_MIN_ZOOM)) as f32
}

/// World-mode location (logical × scale, top-left), centred horizontally on the camera
/// and low in the viewport — like a floating keyboard. Scales/pans with the world.
fn world_loc(state: &Loop, wsize: Size<i32, Physical>) -> Point<i32, Physical> {
    let cam = state.inner.camera().transform.position;
    let zoom = state.inner.camera().transform.zoom.max(OSK_MIN_ZOOM);
    let scale = state.size_ctx_all().scale;
    let (_, screen_h) = state.size_ctx_all().screen_size_physical;
    let vp_half_h = (screen_h * 0.5) / (scale * zoom);
    let top_world_y = cam.y + vp_half_h * 0.30;
    Point::new(
        (cam.x * scale - wsize.w as f64 / 2.0).round() as i32,
        (top_world_y * scale).round() as i32,
    )
}

/// Screen bottom bar: full width (small side margin), ~42% of the output height
/// scaled by the user's `osk_size`.
fn rect(size: Size<i32, Physical>, size_mult: f64) -> Rectangle<i32, Physical> {
    let h = ((size.h as f64) * 0.42 * size_mult).round() as i32;
    let margin = ((size.w as f64) * 0.02).round() as i32;
    let w = (size.w - 2 * margin).max(1);
    let y = (size.h - h.min(size.h)).max(0);
    Rectangle::new(Point::from((margin, y)), Size::new(w, h.clamp(1, size.h)))
}

pub fn per_frame(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    if state.inner.surface().registry.is_none() {
        return;
    }
    let active = state.inner.active_output_key();
    let out = state.inner.render_output.clone().unwrap_or_else(|| active.clone());
    let world = state.inner.worlds.spawn_target().as_u128();
    let key = (world, out.clone());
    let on_active = !out.is_empty() && out == active;

    // Recompute visibility ONCE per frame (on the active output's pass): shown when
    // PINNED (touch-menu) or AUTO — a client text field is active, the last input was
    // touch/pen, and the user hasn't dismissed it. Clearing `dismissed` when no field
    // is active lets the next focused field re-summon it.
    if on_active {
        let ti_active = text_input_active(state);
        let direct = state.inner.touch.modality.is_direct();
        if !ti_active {
            state.inner.kernel.get_mut(&OSK_MUT).dismissed = false;
        }
        let (was_open, pinned, dismissed, restore) = {
            let o = state.inner.kernel.get(&OSK);
            (o.open, o.pinned, o.dismissed, o.travel_restore)
        };
        let visible = pinned || (ti_active && direct && !dismissed);
        // World placement applies only to the AUTO-summoned OSK (touch-menu/pinned is
        // always the screen bottom bar). Read live from the setting.
        let world_mode = !pinned && state.inner.preference.osk_world_position;
        state.inner.kernel.get_mut(&OSK_MUT).world = world_mode;

        // Screen-mode AUTO-show camera travel: when the bottom keyboard slides in, lift
        // the world up so the focused field clears it; travel back when it hides. A
        // world-placed OSK floats with the field, so it never travels.
        if visible && !was_open && !pinned && !world_mode {
            let cam = state.inner.camera().transform.position;
            let zoom = state.inner.camera().transform.zoom.max(0.01);
            let scale = state.size_ctx_all().scale.max(0.01);
            let delta = (size.h as f64) * 0.42 * 0.5 / (scale * zoom);
            state.inner.kernel.get_mut(&OSK_MUT).travel_restore = Some((cam.x, cam.y));
            travel_to(state, (cam.x, cam.y + delta));
        } else if !visible && was_open {
            if let Some(p) = restore {
                travel_to(state, p);
            }
            state.inner.kernel.get_mut(&OSK_MUT).travel_restore = None;
        }

        state.inner.kernel.get_mut(&OSK_MUT).open = visible;

        // Advance the tap-flash countdown and pump a redraw while it runs, so the brief
        // key highlight is actually drawn (an OSK tap schedules no redraw on its own).
        if state.inner.kernel.get(&OSK).flash_frames > 0 {
            {
                let st = state.inner.kernel.get_mut(&OSK_MUT);
                st.flash_frames -= 1;
                if st.flash_frames == 0 {
                    st.flash_key = None;
                }
            }
            state.schedule_redraw();
        }

        // Clear the local echo when the focus / IME target changes, so a previous
        // field's typed text doesn't linger on the next field.
        let fk = focus_key(state);
        if state.inner.kernel.get(&OSK).echo_focus != fk {
            let st = state.inner.kernel.get_mut(&OSK_MUT);
            st.echo.clear();
            st.echo_focus = fk;
        }
    }

    let want = on_active && state.inner.kernel.get(&OSK).open;

    let want_world = state.inner.kernel.get(&OSK).world;
    let mut live = SURF.with(|p| p.borrow().get(&key).cloned());
    live = live.filter(|l| {
        state.inner.surface().registry.as_ref().is_some_and(|r| r.contains(l.id))
    });
    // A placement-mode switch (screen ↔ world) means a different surface — tear the old
    // one down so it recreates in the new space.
    if let Some(l) = &live {
        if l.world != want_world {
            if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
                reg.destroy_by_id(l.id);
            }
            SURF.with(|p| { p.borrow_mut().remove(&key); });
            live = None;
        }
    }

    if !want {
        if let Some(l) = live {
            if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
                reg.destroy_by_id(l.id);
            }
            SURF.with(|p| {
                p.borrow_mut().remove(&key);
            });
        }
        return;
    }

    let osk_size = state.inner.preference.osk_size;
    let m = state.inner.kernel.get(&OSK).mods;
    let want_mods: Mods = (m.shift, m.ctrl, m.alt, m.logo);
    let want_labels = compute_labels(state);
    let want_preview = compute_preview(state);
    let want_flash = state.inner.kernel.get(&OSK).flash_key;
    match live {
        None => {
            if let Some(handle) = create(state, renderer, size, want_mods, &out, osk_size, want_world) {
                if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
                    let _ = reg.dispatch_message(
                        IcedHandle::<Osk>::from_id(handle.id),
                        OskMessage::SetLabels(want_labels.clone()),
                    );
                    let _ = reg.dispatch_message(
                        IcedHandle::<Osk>::from_id(handle.id),
                        OskMessage::SetPreview(want_preview.clone()),
                    );
                }
                SURF.with(|p| {
                    p.borrow_mut().insert(key, Live {
                        id: handle.id,
                        mods: want_mods,
                        labels: want_labels,
                        world: want_world,
                        preview: want_preview,
                        flash: None,
                    });
                });
            }
        }
        Some(l) => {
            let mods_changed = l.mods != want_mods;
            let labels_changed = l.labels != want_labels;
            let preview_changed = l.preview != want_preview;
            let flash_changed = l.flash != want_flash;
            if mods_changed || labels_changed || preview_changed || flash_changed {
                if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
                    if mods_changed {
                        let _ = reg.dispatch_message(
                            IcedHandle::<Osk>::from_id(l.id),
                            OskMessage::SetMods {
                                shift: want_mods.0,
                                ctrl: want_mods.1,
                                alt: want_mods.2,
                                logo: want_mods.3,
                            },
                        );
                    }
                    if labels_changed {
                        let _ = reg.dispatch_message(
                            IcedHandle::<Osk>::from_id(l.id),
                            OskMessage::SetLabels(want_labels.clone()),
                        );
                    }
                    if preview_changed {
                        let _ = reg.dispatch_message(
                            IcedHandle::<Osk>::from_id(l.id),
                            OskMessage::SetPreview(want_preview.clone()),
                        );
                    }
                    if flash_changed {
                        let _ = reg.dispatch_message(
                            IcedHandle::<Osk>::from_id(l.id),
                            OskMessage::SetFlash(want_flash),
                        );
                    }
                }
                SURF.with(|p| {
                    if let Some(e) = p.borrow_mut().get_mut(&key) {
                        e.mods = want_mods;
                        e.labels = want_labels;
                        e.preview = want_preview;
                        e.flash = want_flash;
                    }
                });
            }
            // World-mode: hold a constant on-screen size as the camera zoom changes.
            if want_world {
                resize_world(state, l.id, osk_size);
            }
        }
    }
}

/// Per-frame world-mode zoom tracking: recompute the counter-scaled size + re-centre
/// so the world OSK keeps a constant on-screen size across zoom (mirrors the selection
/// overlay's `resize_on_zoom`). Gated on a zoom change via `OSK.prev_zoom`.
fn resize_world(state: &mut Loop, id: HandleId, osk_size: f64) {
    let zoom = state.inner.camera().transform.zoom;
    if state.inner.kernel.get(&OSK).prev_zoom == zoom {
        return;
    }
    let new_size = world_size(zoom, osk_size);
    let scale = world_scale(zoom);
    let loc = world_loc(state, new_size);
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        reg.request_resize_scaled_by_id(id, new_size, scale);
        reg.set_location_by_id(id, loc);
    }
    state.inner.kernel.get_mut(&OSK_MUT).prev_zoom = zoom;
}

/// Issue an eased camera travel to a world position (same mechanism as
/// `navigator.interface::move_direction`/`fit`).
fn travel_to(state: &mut Loop, pos: (f64, f64)) {
    compositor_y5_navigator_state_base::state::request(
        state.inner.focus_channels(),
        NavRequest::Set(NavState::Travel(Travel {
            position: Some(Target { start: None, target: pos }),
            zoom: None,
            duration: None,
            time_start: None,
        })),
    );
}

fn ensure_font() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(compositor_monitor_selection_font_base::font::load);
}

fn create(
    state: &mut Loop,
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
    mods: Mods,
    out: &str,
    osk_size: f64,
    world: bool,
) -> Option<IcedHandle<Osk>> {
    ensure_font();
    let mut ui = Osk::new();
    ui.shift = mods.0;
    ui.ctrl = mods.1;
    ui.alt = mods.2;
    ui.logo = mods.3;

    // Screen mode: a bottom bar sized from the output. World mode: a floating
    // keyboard centred on the camera, low in the viewport, counter-scaled so its
    // on-screen size stays constant across zoom (mirrors the selection overlay).
    let zoom = state.inner.camera().transform.zoom;
    let (rect, space) = if world {
        let wsize = world_size(zoom, osk_size);
        (Rectangle::new(world_loc(state, wsize), wsize), IcedSpace::World)
    } else {
        (rect(size, osk_size), IcedSpace::Screen)
    };

    let handle = compositor_y5_surface_draw_handle::handle::load(
        state,
        renderer,
        ui,
        rect,
        space,
        compositor_orchestration_draw_layer_base::base::Layer::SCENE.bits(),
    );

    // World-space: lift above every window (load registers it at CONTENT) and set
    // the counter-scale iced factor so content lays out at native size.
    if world {
        state.inner.register_drawable(
            uuid::Uuid::from_u128(handle.id.0 as u128),
            DrawLayer::OVERLAY,
        );
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            reg.request_resize_scaled_by_id(handle.id, world_size(zoom, osk_size), world_scale(zoom));
        }
        state.inner.kernel.get_mut(&OSK_MUT).prev_zoom = zoom;
    }

    let tx = state.inner.surface_mut().surface_message_buffer_channel.0.clone();
    let registry = state.inner.surface_mut().registry.as_mut()?;
    if !out.is_empty() {
        registry.set_output_affinity_by_id(handle.id, Some(out.to_string()));
    }
    // A key tap must not move keyboard focus off the text field being typed into.
    registry.set_keyboard_transparent_by_id(handle.id, true);
    registry
        .instance_mut(handle)?
        .runtime_mut()
        .set_message_handler(move |m: &OskMessage| dispatch(m, &tx));
    Some(handle)
}

fn dispatch(message: &OskMessage, tx: &Sender<SurfaceMessage>) {
    // `SetMods`/`SetLabels` are compositor → surface only; the rest are actionable taps.
    if matches!(message, OskMessage::SetMods { .. } | OskMessage::SetLabels(_)) {
        return;
    }
    let _ = tx.send(SurfaceMessage {
        message: SurfaceMessageType::Osk(message.clone()),
    });
}
