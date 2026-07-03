//! The runtime glue tying the capture session to the compositor: spawning the
//! overlay UIs, driving transitions, projecting the capture region each frame,
//! and the video keep-alive.
//!
//! Coordinate spaces (see document/TRANSFORM.md):
//! - window geometries are y5-world `Logical`;
//! - the registry crop + the indicator overlays are screen `Physical`.
//! The overlay instances are full-screen screen-space at the output's physical
//! size (assumed 1:1 with output pixels, i.e. scale 1.0 — the region rect the
//! user draws is treated as physical directly).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use compositor_orchestration_draw_layer_base::base::Layer;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};
use uuid::Uuid;

use compositor_y5_camera_transform_translate::slot;
use compositor_y5_camera_transform_translate::transform::Transform;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_y5_graphic_capture_encode::{
    AsyncReadback, OptimizedCodec, ReencodeJob, ReencodeStatus, ffmpeg_format, partial_path,
    readback, reencode_detached, save_fallback, save_png,
};
use compositor_y5_graphic_capture_vaapi::{
    Backend, CaptureEncoder, EncoderThread, NvencCudaEncoder, VaapiEncoder, backend_from_config,
    capture_background_auto, capture_cfr, capture_codec_name, capture_codecs, capture_cq,
    capture_fps, capture_nvenc_readback_fallback, capture_render_node,
};
use compositor_y5_graphic_capture_registry::{CaptureSource, OutputId};
use compositor_y5_graphic_capture_session::message::{
    CaptureMedia, CaptureMessage, OverlayRect, TargetKind,
};
use compositor_y5_graphic_capture_session::session::{
    ActiveState, CapturePhase, CaptureTarget, PendingSave,
};
use compositor_y5_surface_draw_capture::border::RegionBorder;
use compositor_y5_surface_draw_capture::dialog::ContinueDialog;
use compositor_y5_surface_draw_capture::dim::RegionDim;
use compositor_y5_surface_draw_capture::hud::StopHud;
use compositor_y5_surface_draw_capture::encodialog::EncodingDialog;
use compositor_y5_surface_draw_capture::errordialog::ErrorDialog;
use compositor_y5_surface_draw_capture::savedialog::SaveDialog;
use compositor_y5_surface_draw_capture::setup::SetupOverlay;
use compositor_y5_surface_draw_handle::handle::{IcedSpace, load};
use compositor_y5_surface_protocol_base::protocol::{SurfaceMessage, SurfaceMessageType};
use compositor_y5_window_interface_record::window::LoopWindow;
use compositor_monitor_compositor_iced_base::{HandleId, IcedHandle};
use compositor_support_iced_core_engine_base::IcedUi;

/// Time a video capture runs before the "still capturing?" prompt appears.
const KEEPALIVE: Duration = Duration::from_secs(300);
/// Grace window to click Continue before the capture auto-stops.
const GRACE: Duration = Duration::from_secs(30);

/// Max width/height (px) of a render-based capture entry. Bounds the dmabuf
/// within GPU / wgpu (`maxTextureDimension2D` 8192) / NVENC H.264 (4096) limits;
/// oversized targets are fit-scaled down into this.
const CAPTURE_MAX_DIM: i32 = 4096;

const STOP_W: i32 = 150;
const STOP_H: i32 = 52;
const DIALOG_W: i32 = 460;
const DIALOG_H: i32 = 170;

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// The Super+S keybinding asks for setup; the per-frame hook (which has a
/// renderer) spawns the overlay.
pub fn request_setup(state: &mut Loop) {
    if state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).is_idle() {
        state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).pending_setup = true;
    }
}

/// Per-frame hook (runs on both backends via the scene `hooks`). Drains a
/// pending setup request, keeps the capture region projected to the current
/// camera, and services the video keep-alive.
pub fn per_frame(state: &mut Loop, renderer: &mut GlesRenderer, _size: Size<i32, Physical>) {
    if state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).pending_setup {
        state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).pending_setup = false;
        if state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).is_idle() {
            start_setup(state, renderer);
        }
    }
    if state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).is_active() {
        // Screenshot: wait a couple of frames for the capture tap to fill the
        // entry, then read it back and go straight to the Save dialog.
        let shot = match &mut state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase {
            CapturePhase::Active(a) => a.shot_wait.as_mut().map(|w| {
                *w = w.saturating_add(1);
                *w
            }),
            _ => None,
        };
        if let Some(frames) = shot {
            // Wait a few frames for the capture tap to actually fill the entry
            // before reading it back (fullscreen/blit can occasionally lag a
            // frame → intermittent white).
            if frames >= 3 {
                begin_saving(state, renderer);
            }
            return;
        }
        update_crop(state, renderer);
        video_frame(state);
        video_keepalive(state, renderer);
    } else if state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).is_saving() {
        poll_saveas(state, renderer);
    } else if state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).is_encoding() {
        poll_encoding(state, renderer);
    }
}

/// Per-frame pump for an active VIDEO capture while an OVERLAY world (e.g. the
/// world picker) owns the frame, so the scene `per_frame` hook does not run.
///
/// Advances ONLY the encoder: the capture tap has already refreshed the entry
/// dmabuf in the backend's picker pass, but `per_frame` (which would encode it)
/// is wired into the scene hooks, which are skipped while the overlay is active —
/// so without this the recording stalls on the last desktop frame and the picker
/// never reaches the video. It deliberately skips `update_crop` (the overlay's
/// camera is not the captured world, so it would mis-project region indicators)
/// and `video_keepalive` (its dialog renders on the scene layer, hidden beneath
/// the overlay).
pub fn overlay_per_frame(state: &mut Loop) {
    if state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).is_active() {
        video_frame(state);
    }
}

/// Dispatch a capture overlay message (routed here from the surface message
/// pump). Selection/drag messages are handled inside the overlay UI itself;
/// only the lifecycle ones act here.
pub fn handle(state: &mut Loop, renderer: &mut GlesRenderer, msg: CaptureMessage) {
    match msg {
        CaptureMessage::Confirm => begin_active(state, renderer),
        CaptureMessage::Cancel => teardown(state),
        CaptureMessage::Stop | CaptureMessage::StopFromDialog => stop(state, renderer),
        CaptureMessage::ContinueCapture => on_continue(state),
        CaptureMessage::SaveDefault => save_default(state, renderer),
        CaptureMessage::SaveAs => save_as(state),
        CaptureMessage::Discard => discard_saving(state),
        _ => {}
    }
}

/// Stop the active capture: finalize the artifact and show the Save dialog.
pub fn stop(state: &mut Loop, renderer: &mut GlesRenderer) {
    if state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).is_active() {
        begin_saving(state, renderer);
    } else {
        teardown(state);
    }
}

/// Stop AND discard — same effect (nothing is saved). Used by the lock and
/// seat-deactivate edge cases.
pub fn stop_and_discard(state: &mut Loop) {
    teardown(state);
}

/// Recompute the tracked window bbox + force-render set. Call on window
/// move/resize/destroy while a capture is active (event-driven, not per-frame).
pub fn on_window_geometry_changed(state: &mut Loop) {
    if !state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).is_active() {
        return;
    }
    let target = match &state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase {
        CapturePhase::Active(a) => a.target.clone(),
        _ => return,
    };
    if let CaptureTarget::Windows(ids) = &target {
        state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).windows_bbox = windows_bbox_world_of(state, ids);
    }
    compute_force_set(state, &target);
}

// ---------------------------------------------------------------------------
// Transitions
// ---------------------------------------------------------------------------

fn start_setup(state: &mut Loop, renderer: &mut GlesRenderer) {
    let (sw, sh) = output_size(state);
    if sw < 1 || sh < 1 {
        return;
    }
    // Snapshot the canvas window selection NOW (Super+S). The capture uses this
    // snapshot, not the live canvas — so switching media / interacting with the
    // overlay can't silently drop the selection (→ fullscreen fallback).
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).setup_selection = selected_uuids(state);

    // Pre-seed the overlay with the snapshot's bbox (if any), so the "Windows"
    // target is preselected with its hole already drawn.
    let preselect = windows_bbox_world_of(state, &state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE).setup_selection)
        .map(|w| world_to_phys(state, w))
        .and_then(|p| clamp_rect(p, sw, sh))
        .map(to_overlay);
    let kind = if preselect.is_some() {
        TargetKind::Windows
    } else {
        TargetKind::FullScreen
    };

    let overlay = SetupOverlay::new(sw, sh, CaptureMedia::Screenshot, kind, preselect);
    let handle = load(
        state,
        renderer,
        overlay,
        full_rect(sw, sh),
        IcedSpace::Screen,
        Layer::SCENE.bits(),
    );
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).setup_id = Some(handle.untyped());
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase = CapturePhase::Setup;
    forward(state, handle);
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        reg.set_keyboard_focus(Some(handle.untyped()));
    }
    info!("capture setup started ({sw}x{sh})");
}

fn begin_active(state: &mut Loop, renderer: &mut GlesRenderer) {
    if !state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).is_setup() {
        return;
    }
    let Some(setup_id) = state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).setup_id else {
        return;
    };

    // Read the authoritative selection off the overlay instance.
    let (kind, media, draft, no_background) = {
        let Some(reg) = state.inner.surface_mut().registry.as_ref() else {
            return;
        };
        let Some(inst) = reg.instance::<SetupOverlay>(IcedHandle::from_id(setup_id)) else {
            return;
        };
        (
            inst.ui().kind(),
            inst.ui().media(),
            inst.ui().draft(),
            inst.ui().no_background(),
        )
    };

    let target = match kind {
        // Use the snapshot taken at Super+S, not the live canvas selection.
        TargetKind::Windows => CaptureTarget::Windows(state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).setup_selection.clone()),
        TargetKind::ScreenRegion => {
            let Some(d) = draft else {
                teardown(state);
                return;
            };
            CaptureTarget::ScreenRegion(from_overlay(d))
        }
        TargetKind::WorldRegion => {
            let Some(d) = draft else {
                teardown(state);
                return;
            };
            CaptureTarget::WorldRegion(phys_to_world(state, from_overlay(d)))
        }
        TargetKind::FullScreen => CaptureTarget::FullScreen,
    };

    // Tear down the setup overlay + its exclusive input.
    destroy(state, Some(setup_id));
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).setup_id = None;
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        reg.set_keyboard_focus(None);
    }

    let (sw, sh) = output_size(state);

    // Window bbox + force-render set (before computing the crop, which needs
    // the bbox for the Windows target).
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).windows_bbox = match &target {
        CaptureTarget::Windows(ids) => windows_bbox_world_of(state, ids),
        _ => None,
    };
    compute_force_set(state, &target);

    // Fixed y5-world origin of the capture region (for the per-element render,
    // so the encoder's dmabuf stays a constant size as windows move).
    let region_origin: Point<i32, Logical> = match &target {
        CaptureTarget::Windows(_) => state
            .inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT)
            .windows_bbox
            .map(|b| b.loc)
            .unwrap_or_default(),
        CaptureTarget::WorldRegion(r) => r.loc,
        _ => Point::from((0, 0)),
    };

    // Entry source. Render-based (Windows/WorldRegion) get a FIXED
    // native-resolution entry (`world size × scale`, camera-zoom-independent);
    // the per-element render fills it, so the screen position/clamp is
    // irrelevant. Blit-based (ScreenRegion) gets the on-screen crop; FullScreen
    // taps the whole framebuffer.
    // Capture targets the ACTIVE monitor (the one the user is on), keyed by its
    // stable EDID-derived id — the same id the render loop tags that output's
    // capture entries with, so a capture from a secondary monitor reads that
    // monitor's framebuffer instead of always the primary's (`OutputId(0)`).
    let output_id = OutputId::from_key(&state.inner.active_output_key());
    let source = match &target {
        CaptureTarget::Windows(_) | CaptureTarget::WorldRegion(_) => {
            let Some(sz) = render_entry_size(state, &target) else {
                teardown(state);
                return;
            };
            CaptureSource::Region {
                output: output_id,
                rect: Rectangle::new(Point::from((0, 0)), sz),
            }
        }
        CaptureTarget::ScreenRegion(_) => match target_crop_physical(state, &target, sw, sh) {
            Some(rect) => CaptureSource::Region {
                output: output_id,
                rect,
            },
            None => {
                teardown(state);
                return;
            }
        },
        CaptureTarget::FullScreen => CaptureSource::OutputFramebuffer(output_id),
    };

    let gpu = state.inner.environment.GPU.clone();
    let req = state
        .inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY_MUT)
        .as_ref()
        .map(|reg| reg.request(&gpu, renderer, source));
    let capture = match req {
        Some(Ok(h)) => h,
        other => {
            warn!("capture registry request failed: {other:?}");
            teardown(state);
            return;
        }
    };

    // A screenshot needs no on-screen chrome (it would otherwise be baked into
    // the captured frame): no indicators, no stop button. We just let the
    // capture tap fill the entry for a couple of frames, then read it back and
    // jump straight to the Save dialog (`video_frame`/`per_frame` drive this
    // via `shot_wait`).
    if media == CaptureMedia::Screenshot {
        state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase = CapturePhase::Active(ActiveState {
            media,
            target,
            capture,
            no_background,
            encoder: None,
            readback: None,
            region_origin,
            last_frame: None,
            video_start: None,
            shot_wait: Some(0),
            last_crop: None,
            keepalive_anchor: Instant::now(),
            dialog_deadline: None,
        });
        info!("screenshot capturing (no chrome)");
        return;
    }

    // Video: start the hardware encoder FIRST, so we can abort cleanly (error
    // dialog) instead of "recording" nothing when no encoder is available — and
    // so we don't spawn indicator chrome we'd have to tear back down.
    let fps = capture_fps();
    let cq = capture_cq();
    let (encoder, video_readback) = match backend_from_config() {
        Backend::Nvenc => {
            // Zero-copy on every backend, including winit/nested — there the
            // result is vertically flipped (the CUDA path can't flip), which is
            // accepted rather than dropping to the readback path.
            let zc = capture.dmabuf().and_then(|d| {
                capture_codecs()
                    .into_iter()
                    .find_map(|c| NvencCudaEncoder::start(&d, fps, c, cq))
            });
            match zc {
                Some(z) => (Some(CaptureEncoder::NvencCuda(z)), None),
                // The readback path is only used as a fallback when explicitly
                // allowed (`nvenc_allow_readback_fallback`); otherwise a failed
                // zero-copy aborts the capture with an error dialog.
                None if capture_nvenc_readback_fallback() => {
                    warn!("nvenc zero-copy unavailable — using readback fallback");
                    let enc = capture.size().and_then(|s| {
                        EncoderThread::spawn_nvenc(s.w.max(0) as u32, s.h.max(0) as u32, fps, cq)
                    });
                    (enc.map(CaptureEncoder::Nvenc), Some(AsyncReadback::new()))
                }
                None => {
                    warn!("nvenc zero-copy unavailable; readback fallback disabled");
                    (None, None)
                }
            }
        }
        Backend::Vaapi => {
            // Same codec preference + fallback as NVENC (`av1 → hevc → h264`):
            // try each VAAPI encoder until one opens. GPUs without AV1/HEVC encode
            // (e.g. Kaby Lake) land on h264_vaapi.
            let enc = capture.dmabuf().and_then(|dmabuf| {
                capture_codecs()
                    .into_iter()
                    .find_map(|c| VaapiEncoder::start(&dmabuf, fps, c))
            });
            (enc.map(CaptureEncoder::Vaapi), None)
        }
    };
    if encoder.is_none() {
        warn!("hardware video encoder unavailable — capture aborted");
        drop(capture); // free the registry entry (no Active state will hold it)
        show_capture_error(
            state,
            renderer,
            "Recording unavailable: no working hardware video encoder.",
        );
        return;
    }

    // Video: indicator overlays. The dim sits below windows (its own layer);
    // the border sits above everything (screen-space). Both are click-through
    // (CAPTURE_PASSTHROUGH → hit-test-transparent) so the whole capture region
    // passes the pointer (motion + clicks) through to the windows while
    // recording. The Stop HUD has no passthrough bit, so it stays clickable.
    // Indicator rect is UNCLAMPED (the on-screen projection of the region/bbox),
    // so the teal border crops at the screen edge as the region pans off rather
    // than shrinking.
    // Origin monitor: the one the capture started on. The region dim + border are
    // BOUND to it (output affinity) so they dim/outline THAT monitor at its own
    // size, instead of painting every monitor at the origin's dimensions.
    let origin_key = state.inner.active_output_key();

    let crop_ov = indicator_rect(state, &target, sw, sh).unwrap_or(OverlayRect {
        x: 0,
        y: 0,
        w: sw,
        h: sh,
    });

    let dim = load(
        state,
        renderer,
        RegionDim::new(sw, sh, crop_ov),
        full_rect(sw, sh),
        IcedSpace::Screen,
        (Layer::CAPTURE_DIM | Layer::CAPTURE_PASSTHROUGH).bits(),
    );
    let dim_id = dim.untyped();
    bind_output(state, dim_id, &origin_key);
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).dim_id = Some(dim_id);

    let border = load(
        state,
        renderer,
        RegionBorder::new(sw, sh, crop_ov),
        full_rect(sw, sh),
        IcedSpace::Screen,
        (Layer::SCENE | Layer::CAPTURE_PASSTHROUGH).bits(),
    );
    let border_id = border.untyped();
    bind_output(state, border_id, &origin_key);
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).border_id = Some(border_id);

    // One Stop button per monitor: anchored to each monitor's own top-right and
    // bound to that output, so the control shows on every screen and is clickable
    // on whichever monitor the cursor is on — not just the origin.
    for (key, (ow, _oh)) in output_sizes(state) {
        let stop_rect = Rectangle::new(
            Point::from((ow - STOP_W - 12, 12)),
            Size::from((STOP_W, STOP_H)),
        );
        let stop = load(
            state,
            renderer,
            StopHud,
            stop_rect,
            IcedSpace::Screen,
            Layer::SCENE.bits(),
        );
        let stop_id = stop.untyped();
        bind_output(state, stop_id, &key);
        forward(state, stop);
        state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).stop_hud_ids.push(stop_id);
    }

    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase = CapturePhase::Active(ActiveState {
        media,
        target,
        capture,
        no_background,
        encoder,
        readback: video_readback,
        region_origin,
        last_frame: None,
            video_start: None,
        shot_wait: None,
        last_crop: None,
        keepalive_anchor: Instant::now(),
        dialog_deadline: None,
    });
    info!("capture active (media={media:?})");
}

fn on_continue(state: &mut Loop) {
    destroy(state, state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE).continue_dialog_id);
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).continue_dialog_id = None;
    if let CapturePhase::Active(a) = &mut state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase {
        a.keepalive_anchor = Instant::now();
        a.dialog_deadline = None;
    }
}

/// Tear down every capture overlay and drop all capture resources, returning
/// to Idle. Used by Cancel and the lock/seat-deactivate discard. Discards any
/// in-flight encoder and any unsaved temp video — nothing is saved.
fn teardown(state: &mut Loop) {
    // Clean encoder / temp file before the phase (and its `CaptureHandle`) drops.
    let phase = std::mem::replace(&mut state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase, CapturePhase::Idle);
    match phase {
        CapturePhase::Active(a) => {
            if let Some(e) = a.encoder {
                e.discard();
            }
        }
        CapturePhase::Saving {
            pending: PendingSave::Video(temp),
            ..
        } => {
            let _ = std::fs::remove_file(temp);
        }
        CapturePhase::Encoding { job, lossless, .. } => {
            job.cancel(); // kills ffmpeg + deletes the partial output
            let _ = std::fs::remove_file(lossless);
        }
        _ => {}
    }

    let ids: Vec<HandleId> = {
        let c = &state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT);
        [
            c.setup_id,
            c.border_id,
            c.dim_id,
            c.continue_dialog_id,
            c.save_dialog_id,
        ]
        .into_iter()
        .flatten()
        .chain(c.stop_hud_ids.iter().copied())
        .collect()
    };
    {
        let c = &mut state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT);
        c.pending_setup = false;
        c.setup_id = None;
        c.border_id = None;
        c.dim_id = None;
        c.stop_hud_ids.clear();
        c.continue_dialog_id = None;
        c.save_dialog_id = None;
        c.force_set.clear();
        c.windows_bbox = None;
        c.setup_selection.clear();
    }
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        for id in ids {
            reg.destroy_by_id(id);
        }
        reg.set_keyboard_focus(None);
    }
}

// ---------------------------------------------------------------------------
// Video frame pump + Saving phase
// ---------------------------------------------------------------------------

/// Per-frame video pump, throttled to `VIDEO_FPS`.
/// - VAAPI: zero-copy — encode the capture dmabuf directly (`encode()`).
/// - NVENC: consume a completed GPU→CPU readback into the encoder, then submit
///   the next one (non-blocking).
fn video_frame(state: &mut Loop) {
    let now = Instant::now();
    let interval = Duration::from_millis(1000 / capture_fps() as u64);
    let ctx = state
        .inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY_MUT)
        .as_ref()
        .map(|r| r.wgpu_ctx().clone());
    let flip = state.inner.storage.nested;
    let CapturePhase::Active(a) = &mut state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase else {
        return;
    };
    let due = a
        .last_frame
        .map(|t| now.duration_since(t) >= interval)
        .unwrap_or(true);
    // Capture-time PTS in the encoder timebase (1/fps), anchored at the first
    // frame. Using real elapsed time (not a frame counter) makes the recorded
    // timeline match wall-clock regardless of how the achieved rate compares to
    // the configured max — so `capture_refresh_rate_max` is a true cap, not an
    // assumed rate (which otherwise sped videos up when the compositor drew
    // fewer frames/s than the cap).
    let fps = capture_fps();
    let start = *a.video_start.get_or_insert(now);
    let pts = (now.saturating_duration_since(start).as_secs_f64() * fps as f64).round() as i64;
    match a.encoder.as_mut() {
        Some(CaptureEncoder::Vaapi(v)) => {
            if !due {
                return;
            }
            v.encode(pts);
            a.last_frame = Some(now);
        }
        Some(CaptureEncoder::NvencCuda(z)) => {
            // Zero-copy: the dmabuf already holds this frame (capture blit ran
            // earlier in the frame). Just signal the encoder thread to map +
            // CUDA-copy + encode it. No readback.
            if !due {
                return;
            }
            // PTS for the zero-copy path is computed on the encoder worker at
            // encode time (so it matches the frame actually read); we only signal.
            z.tick();
            a.last_frame = Some(now);
        }
        Some(CaptureEncoder::Nvenc(n)) => {
            let (Some(ctx), Some(rb)) = (ctx.as_ref(), a.readback.as_mut()) else {
                return;
            };
            // 1. Hand a completed readback to the encoder thread (non-blocking,
            //    unbounded → never drops a frame). The vertical flip + encode
            //    run on the worker, off the render thread.
            if let Some(frame) = rb.poll(ctx) {
                n.send(frame.bgra, frame.width, frame.height, flip, pts);
            }
            // 2. Submit a fresh readback if due and the slot is free.
            if due && !rb.inflight() {
                if let Some(tex) = a.capture.wgpu_texture() {
                    rb.submit(ctx, &tex);
                    a.last_frame = Some(now);
                }
            }
        }
        None => {}
    }
}

/// Active → Saving: finalize the artifact (read back the screenshot or finish
/// the video) and raise the Save dialog.
fn begin_saving(state: &mut Loop, renderer: &mut GlesRenderer) {
    let phase = std::mem::replace(&mut state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase, CapturePhase::Idle);
    let CapturePhase::Active(active) = phase else {
        return;
    };
    let media = active.media;

    // Drop the live indicators; we keep no force-render set once stopped.
    destroy_indicators(state);
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).force_set.clear();
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).windows_bbox = None;

    let pending = match media {
        CaptureMedia::Video => match active.encoder.and_then(|e| e.finish()) {
            Some(path) => PendingSave::Video(path),
            None => {
                warn!("video produced no file — nothing to save");
                teardown(state);
                return;
            }
        },
        CaptureMedia::Screenshot => {
            let flip = state.inner.storage.nested;
            let frame = active.capture.wgpu_texture().and_then(|t| {
                state
                    .inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY_MUT)
                    .as_ref()
                    .and_then(|reg| readback(reg.wgpu_ctx(), &t))
            });
            match frame {
                Some(mut f) => {
                    if flip {
                        f.flip_vertical();
                    }
                    PendingSave::Image(f)
                }
                None => {
                    warn!("screenshot readback failed — nothing to save");
                    teardown(state);
                    return;
                }
            }
        }
    };
    // `active` (and its CaptureHandle) drops here → registry entry freed.

    let (sw, sh) = output_size(state);
    let rect = Rectangle::new(
        Point::from(((sw - DIALOG_W) / 2, (sh - DIALOG_H) / 2)),
        Size::from((DIALOG_W, DIALOG_H)),
    );
    let label = match media {
        CaptureMedia::Video => "Video",
        CaptureMedia::Screenshot => "Screenshot",
    };
    let dlg = load(
        state,
        renderer,
        // Offer the "Optimized encoding" checkbox only in manual mode (auto runs
        // it in the background with no checkbox).
        SaveDialog::new(label, !capture_background_auto()),
        rect,
        IcedSpace::Screen,
        Layer::SCENE.bits(),
    );
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).save_dialog_id = Some(dlg.untyped());
    forward(state, dlg);
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase = CapturePhase::Saving {
        media,
        pending,
        saveas: None,
    };
    info!("capture stopped — save dialog up (media={media:?})");
}

fn save_default(state: &mut Loop, renderer: &mut GlesRenderer) {
    let video = match &state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase {
        CapturePhase::Saving { media, .. } => *media == CaptureMedia::Video,
        _ => return,
    };
    let target = compositor_y5_graphic_capture_encode::default_path(video);
    commit_save(state, renderer, target);
}

/// Spawn the XDG portal "Save As" dialog on a background thread; the result is
/// drained by `poll_saveas`. No-op if one is already in flight.
fn save_as(state: &mut Loop) {
    let CapturePhase::Saving { media, saveas, .. } = &mut state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase else {
        return;
    };
    if saveas.is_some() {
        return;
    }
    let suggested = if *media == CaptureMedia::Video {
        "capture.mp4"
    } else {
        "capture.png"
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result =
            compositor_y5_graphic_capture_encode::portal::save_file_dialog("Save capture", suggested);
        let _ = tx.send(result);
    });
    *saveas = Some(rx);
}

/// Drain the in-flight Save As result (called each frame during Saving).
fn poll_saveas(state: &mut Loop, renderer: &mut GlesRenderer) {
    use std::sync::mpsc::TryRecvError;
    // `None` → portal still running; `Some(v)` → it returned (`v` is `None` if
    // cancelled/failed).
    let resolved: Option<Option<PathBuf>> = {
        let CapturePhase::Saving { saveas, .. } = &state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase else {
            return;
        };
        let Some(rx) = saveas else { return };
        match rx.try_recv() {
            Ok(v) => Some(v),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(None),
        }
    };
    let Some(outcome) = resolved else {
        // Result not in yet. The portal runs on a background thread and generates
        // no compositor events, so keep the loop awake to catch the result the
        // instant it arrives (same reason as the encoding poll). Without this, a
        // result landing while the compositor is idle isn't picked up until some
        // unrelated event happens to wake it.
        state.schedule_redraw();
        return;
    };
    // Clear the in-flight receiver either way.
    if let CapturePhase::Saving { saveas, .. } = &mut state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase {
        *saveas = None;
    }
    match outcome {
        Some(path) => commit_save(state, renderer, path),
        // Portal failed/cancelled: keep the Save dialog up (no-op).
        None => {}
    }
}

fn discard_saving(state: &mut Loop) {
    teardown(state);
}

/// Abort a capture before it starts and show a modal error dialog (e.g. no
/// working hardware video encoder). The dialog's OK button emits `Discard`,
/// which routes to `teardown` and dismisses it.
fn show_capture_error(state: &mut Loop, renderer: &mut GlesRenderer, message: &str) {
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase = CapturePhase::Idle;
    let (sw, sh) = output_size(state);
    let rect = Rectangle::new(
        Point::from(((sw - DIALOG_W) / 2, (sh - DIALOG_H) / 2)),
        Size::from((DIALOG_W, DIALOG_H)),
    );
    let dlg = load(
        state,
        renderer,
        ErrorDialog::new(message),
        rect,
        IcedSpace::Screen,
        Layer::SCENE.bits(),
    );
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).save_dialog_id = Some(dlg.untyped());
    forward(state, dlg);
    warn!("capture aborted — {message}");
}

/// Commit the chosen `target`: save the screenshot, or for video route to the
/// plain move / background re-encode / manual encoding-with-progress paths.
fn commit_save(state: &mut Loop, renderer: &mut GlesRenderer, target: PathBuf) {
    // Read the checkbox before the dialog is torn down.
    let optimize_checked = save_dialog_optimized(state);
    let auto = capture_background_auto();
    // CFR (constant frame rate) can only be produced by re-encoding, so it forces
    // a re-encode even when the user didn't opt into "Optimized encoding".
    let cfr = capture_cfr();

    let phase = std::mem::replace(&mut state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase, CapturePhase::Idle);
    let CapturePhase::Saving { pending, .. } = phase else {
        return;
    };
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match pending {
        PendingSave::Image(frame) => {
            match save_png(&frame, &target) {
                Ok(()) => info!("screenshot saved: {}", target.display()),
                Err(e) => warn!("screenshot save failed: {e}"),
            }
            close_save_dialog(state);
        }
        PendingSave::Video(temp) => {
            if auto || optimize_checked || cfr {
                let codec = optimized_codec();
                if auto {
                    // Background, no progress UI: writes a `.y5-encoding` partial
                    // and renames to `target` when done (lossless fallback on fail).
                    reencode_detached(temp, target, codec, cfr, capture_render_node());
                    close_save_dialog(state);
                } else {
                    begin_encoding(state, renderer, temp, target, codec, cfr);
                }
            } else {
                save_fallback(&temp, &target);
                info!("video saved: {}", target.display());
                close_save_dialog(state);
            }
        }
    }
}

/// Manual "Optimized encoding": kick off the re-encode and show the progress
/// dialog. Falls back to saving the lossless temp if ffmpeg can't start.
fn begin_encoding(
    state: &mut Loop,
    renderer: &mut GlesRenderer,
    temp: PathBuf,
    target: PathBuf,
    codec: OptimizedCodec,
    cfr: bool,
) {
    let partial = partial_path(&target);
    let job = match ReencodeJob::spawn(
        &temp,
        partial.clone(),
        codec,
        cfr,
        ffmpeg_format(&target),
        &capture_render_node(),
    ) {
        Some(j) => j,
        None => {
            warn!("optimized encode could not start — saving lossless");
            save_fallback(&temp, &target);
            close_save_dialog(state);
            return;
        }
    };
    // Swap the save dialog for the progress dialog (reusing `save_dialog_id`).
    close_save_dialog(state);
    let (sw, sh) = output_size(state);
    let rect = Rectangle::new(
        Point::from(((sw - DIALOG_W) / 2, (sh - DIALOG_H) / 2)),
        Size::from((DIALOG_W, DIALOG_H)),
    );
    let dlg = load(
        state,
        renderer,
        EncodingDialog::new(),
        rect,
        IcedSpace::Screen,
        Layer::SCENE.bits(),
    );
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).save_dialog_id = Some(dlg.untyped());
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase = CapturePhase::Encoding {
        job,
        target,
        output: partial,
        lossless: temp,
    };
    info!("optimized encoding started");
    // Kick the first poll; the Running branch keeps the loop alive thereafter.
    state.schedule_redraw();
}

/// Per-frame poll of the manual re-encode: update the progress bar; on success
/// place the optimized file; on failure revert to the save dialog (lossless kept).
fn poll_encoding(state: &mut Loop, renderer: &mut GlesRenderer) {
    let status = match &mut state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase {
        CapturePhase::Encoding { job, .. } => job.poll(),
        _ => return,
    };
    match status {
        ReencodeStatus::Running(f) => {
            set_encode_progress(state, (f * 100.0) as u32);
            // Keep the render loop awake. Unlike active capture (POST_SCENE tap)
            // or the Save portal (window events), a background ffmpeg produces no
            // events, so without this the compositor idles and never polls again —
            // the encode finishes but its `.tmp` is never renamed and the dialog
            // never closes. Self-sustains: each Running poll schedules the next
            // frame; Done/Failed stops scheduling and the loop idles normally.
            state.schedule_redraw();
        }
        ReencodeStatus::Done(output) => {
            let phase = std::mem::replace(&mut state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase, CapturePhase::Idle);
            if let CapturePhase::Encoding { target, lossless, .. } = phase {
                save_fallback(&output, &target); // place the finished optimized file
                let _ = std::fs::remove_file(&lossless);
                info!("optimized video saved: {}", target.display());
            }
            close_save_dialog(state);
        }
        ReencodeStatus::Failed => {
            let phase = std::mem::replace(&mut state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase, CapturePhase::Idle);
            if let CapturePhase::Encoding { lossless, output, .. } = phase {
                let _ = std::fs::remove_file(&output);
                warn!("optimized encode failed — reverting to save (lossless kept)");
                close_save_dialog(state);
                respawn_save_dialog(state, renderer, lossless);
            }
        }
    }
}

/// Re-enter the Saving phase with the lossless temp + a fresh save dialog (used
/// when the optimized re-encode fails, so the recording can still be saved).
fn respawn_save_dialog(state: &mut Loop, renderer: &mut GlesRenderer, lossless: PathBuf) {
    let (sw, sh) = output_size(state);
    let rect = Rectangle::new(
        Point::from(((sw - DIALOG_W) / 2, (sh - DIALOG_H) / 2)),
        Size::from((DIALOG_W, DIALOG_H)),
    );
    let dlg = load(
        state,
        renderer,
        SaveDialog::new("Video", !capture_background_auto()),
        rect,
        IcedSpace::Screen,
        Layer::SCENE.bits(),
    );
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).save_dialog_id = Some(dlg.untyped());
    forward(state, dlg);
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase = CapturePhase::Saving {
        media: CaptureMedia::Video,
        pending: PendingSave::Video(lossless),
        saveas: None,
    };
}

/// Push the latest re-encode progress (0..=100) to the encoding dialog.
fn set_encode_progress(state: &mut Loop, percent: u32) {
    let Some(id) = state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).save_dialog_id else {
        return;
    };
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        let _ = reg.dispatch_message(
            IcedHandle::<EncodingDialog>::from_id(id),
            CaptureMessage::SetEncodeProgress(percent),
        );
    }
}

/// Read the save dialog's "Optimized encoding" checkbox state (false if absent).
fn save_dialog_optimized(state: &mut Loop) -> bool {
    let Some(id) = state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE).save_dialog_id else {
        return false;
    };
    let Some(reg) = state.inner.surface_mut().registry.as_ref() else {
        return false;
    };
    reg.instance::<SaveDialog>(IcedHandle::from_id(id))
        .map(|inst| inst.ui().optimized())
        .unwrap_or(false)
}

/// Destroy the save/encoding dialog and clear its handle. Leaves the phase as-is.
fn close_save_dialog(state: &mut Loop) {
    destroy(state, state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE).save_dialog_id);
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).save_dialog_id = None;
}

/// Map the configured codec to the software re-encode codec.
fn optimized_codec() -> OptimizedCodec {
    match capture_codec_name() {
        "h264" => OptimizedCodec::H264,
        "h265" => OptimizedCodec::H265,
        _ => OptimizedCodec::Av1,
    }
}

fn destroy_indicators(state: &mut Loop) {
    let ids: Vec<HandleId> = {
        let c = &state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT);
        [c.border_id, c.dim_id, c.continue_dialog_id]
            .into_iter()
            .flatten()
            .chain(c.stop_hud_ids.iter().copied())
            .collect()
    };
    {
        let c = &mut state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT);
        c.border_id = None;
        c.dim_id = None;
        c.stop_hud_ids.clear();
        c.continue_dialog_id = None;
    }
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        for id in ids {
            reg.destroy_by_id(id);
        }
    }
}

// ---------------------------------------------------------------------------
// Per-frame region projection
// ---------------------------------------------------------------------------

fn update_crop(state: &mut Loop, renderer: &mut GlesRenderer) {
    let (sw, sh) = output_size(state);
    let (entry, target, last) = {
        let CapturePhase::Active(a) = &state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase else {
            return;
        };
        (a.capture.entry_id(), a.target.clone(), a.last_crop)
    };

    // The UNCLAMPED on-screen rect to outline (so the border crops at the screen
    // edge as the region pans off). WorldRegion = the fixed world rect (origin +
    // native entry size) projected; Windows = the LIVE window bbox projected;
    // ScreenRegion = the fixed screen rect.
    let indicator: Option<Rectangle<i32, Physical>> = match &target {
        CaptureTarget::FullScreen => None,
        CaptureTarget::ScreenRegion(r) => Some(*r),
        // The actual world rect (the entry may be capped/fit-scaled, so deriving
        // it from `entry_size/scale` would mis-size the indicator).
        CaptureTarget::WorldRegion(r) => Some(world_to_phys(state, *r)),
        CaptureTarget::Windows(_) => live_windows_bbox(state).map(|b| world_to_phys(state, b)),
    };
    // FullScreen has no indicator (OutputFramebuffer source) — nothing to update.
    let Some(indicator) = indicator else {
        return;
    };
    // Skip when unchanged (re-rendering the full-screen border/dim every frame
    // is what made region video lag).
    if last == Some(indicator) {
        return;
    }

    // Only blit-based (ScreenRegion) entries resize — to the clamped on-screen
    // crop. Render-based entries stay fixed (the per-element render fills them).
    if matches!(target, CaptureTarget::ScreenRegion(_)) {
        if let Some(crop) = clamp_rect(indicator, sw, sh) {
            let gpu = state.inner.environment.GPU.clone();
            if let Some(reg) = state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY_MUT).as_ref() {
                if let Err(e) = reg.set_region(&gpu, renderer, entry, crop) {
                    warn!("capture set_region failed: {e:?}");
                }
            }
        }
    }

    let ov = to_overlay(indicator);
    set_region_msg::<RegionBorder>(state, state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE).border_id, ov);
    set_region_msg::<RegionDim>(state, state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE).dim_id, ov);

    if let CapturePhase::Active(a) = &mut state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase {
        a.last_crop = Some(indicator);
    }
}

fn video_keepalive(state: &mut Loop, renderer: &mut GlesRenderer) {
    let now = Instant::now();
    let (is_video, anchor, deadline) = match &state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase {
        CapturePhase::Active(a) => {
            (a.media == CaptureMedia::Video, a.keepalive_anchor, a.dialog_deadline)
        }
        _ => return,
    };
    if !is_video {
        return;
    }
    match deadline {
        None => {
            if now.duration_since(anchor) >= KEEPALIVE {
                spawn_dialog(state, renderer, now + GRACE);
            }
        }
        Some(dl) => {
            if now >= dl {
                info!("video keep-alive expired — stopping capture");
                stop(state, renderer);
            } else {
                let secs = dl.saturating_duration_since(now).as_secs() as u32;
                set_countdown(state, secs);
            }
        }
    }
}

fn spawn_dialog(state: &mut Loop, renderer: &mut GlesRenderer, deadline: Instant) {
    let (sw, sh) = output_size(state);
    let rect = Rectangle::new(
        Point::from(((sw - DIALOG_W) / 2, (sh - DIALOG_H) / 2)),
        Size::from((DIALOG_W, DIALOG_H)),
    );
    let dialog = load(
        state,
        renderer,
        ContinueDialog::new(GRACE.as_secs() as u32),
        rect,
        IcedSpace::Screen,
        Layer::SCENE.bits(),
    );
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).continue_dialog_id = Some(dialog.untyped());
    forward(state, dialog);
    if let CapturePhase::Active(a) = &mut state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).phase {
        a.dialog_deadline = Some(deadline);
    }
    info!("video over 5 min — continue prompt shown");
}

fn set_countdown(state: &mut Loop, secs: u32) {
    let Some(id) = state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).continue_dialog_id else {
        return;
    };
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        let _ = reg.dispatch_message(
            IcedHandle::<ContinueDialog>::from_id(id),
            CaptureMessage::SetCountdown(secs),
        );
    }
}

// ---------------------------------------------------------------------------
// Geometry helpers
// ---------------------------------------------------------------------------

fn output_size(state: &Loop) -> (i32, i32) {
    // The ACTIVE monitor's mode (the one being captured), not the primary — a
    // ScreenRegion crop on a secondary monitor must size to that monitor.
    match state.inner.active_output().current_mode() {
        Some(m) => (m.size.w, m.size.h),
        None => (0, 0),
    }
}

fn target_crop_physical(
    state: &Loop,
    target: &CaptureTarget,
    sw: i32,
    sh: i32,
) -> Option<Rectangle<i32, Physical>> {
    let phys = match target {
        CaptureTarget::FullScreen => return None,
        CaptureTarget::ScreenRegion(r) => *r,
        CaptureTarget::WorldRegion(r) => world_to_phys(state, *r),
        CaptureTarget::Windows(_) => world_to_phys(state, state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE).windows_bbox?),
    };
    clamp_rect(phys, sw, sh)
}

/// Native (camera-zoom-independent) entry size for a render-based target:
/// `world size × output scale`, capped to [`CAPTURE_MAX_DIM`] (preserving aspect)
/// so a huge union bbox (multiple large/spread windows) can't blow past GPU /
/// wgpu (`maxTextureDimension2D` 8192) / NVENC (4096) limits — which would fail
/// the dmabuf allocation/import and silently discard the capture. The
/// `window_render_job` fit-scale draws the content into whatever size we return.
fn render_entry_size(state: &Loop, target: &CaptureTarget) -> Option<Size<i32, Physical>> {
    let scale = state.size_ctx_all().scale;
    let world = match target {
        CaptureTarget::WorldRegion(r) => *r,
        CaptureTarget::Windows(_) => state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE).windows_bbox?,
        _ => return None,
    };
    let mut w = (world.size.w as f64 * scale).round() as i32;
    let mut h = (world.size.h as f64 * scale).round() as i32;
    if w < 1 || h < 1 {
        return None;
    }
    let big = w.max(h);
    if big > CAPTURE_MAX_DIM {
        let f = CAPTURE_MAX_DIM as f64 / big as f64;
        w = ((w as f64 * f).round() as i32).max(1);
        h = ((h as f64 * f).round() as i32).max(1);
    }
    Some(Size::from((w, h)))
}

/// The on-screen rect to outline with the indicator, UNCLAMPED (so the border
/// crops at the screen edge instead of shrinking). World targets project through
/// the current camera; screen targets are fixed.
fn indicator_rect(state: &Loop, target: &CaptureTarget, sw: i32, sh: i32) -> Option<OverlayRect> {
    let phys = match target {
        CaptureTarget::FullScreen => {
            return Some(OverlayRect {
                x: 0,
                y: 0,
                w: sw,
                h: sh,
            });
        }
        CaptureTarget::ScreenRegion(r) => *r,
        CaptureTarget::WorldRegion(r) => world_to_phys(state, *r),
        CaptureTarget::Windows(_) => world_to_phys(state, live_windows_bbox(state)?),
    };
    Some(to_overlay(phys))
}

/// The window's **compositor slot** rect in y5-world `Logical` — its enforced
/// location + size, not the full subsurface `element_bbox`. This is what the
/// capture region tracks/invalidates against, so popups/subsurfaces extending
/// past the window don't bloat the captured region. Falls back to the element
/// geometry when no slot size has been decided yet.
fn window_slot_rect(state: &Loop, w: &smithay::desktop::Window) -> Option<Rectangle<i32, Logical>> {
    let loc = state.inner.space_state().state.element_location(w)?;
    match slot::expected_size(w).filter(|s| s.w > 0 && s.h > 0) {
        Some(size) => Some(Rectangle::new(loc, size)),
        None => state.inner.space_state().state.element_geometry(w),
    }
}

/// The live union bbox (y5-world) of the captured (`force_set`) windows.
fn live_windows_bbox(state: &Loop) -> Option<Rectangle<i32, Logical>> {
    let mut acc: Option<Rectangle<i32, Logical>> = None;
    for w in state.inner.space_state().state.elements() {
        let Some(id) = w.uuid() else { continue };
        if !state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE).force_set.contains(&id) {
            continue;
        }
        if let Some(g) = window_slot_rect(state, w) {
            acc = Some(match acc {
                None => g,
                Some(a) => union(a, g),
            });
        }
    }
    acc
}

fn world_to_phys(state: &Loop, r: Rectangle<i32, Logical>) -> Rectangle<i32, Physical> {
    let t: Transform = (r, state.size_ctx_all()).into();
    t.into()
}

fn phys_to_world(state: &Loop, r: Rectangle<i32, Physical>) -> Rectangle<i32, Logical> {
    // `(physical, ctx).into()` reverse-projects the screen rect into y5-world
    // coordinates inside the Transform; extract the RAW world rect via
    // `into_storage_rect()`. (A plain `.into()` would re-apply the forward
    // camera projection, double-projecting the region — the world-region bug.)
    let t: Transform = (r, state.size_ctx_all()).into();
    t.into_storage_rect()
}

fn clamp_rect(r: Rectangle<i32, Physical>, sw: i32, sh: i32) -> Option<Rectangle<i32, Physical>> {
    let x0 = r.loc.x.clamp(0, sw);
    let y0 = r.loc.y.clamp(0, sh);
    let x1 = (r.loc.x + r.size.w).clamp(0, sw);
    let y1 = (r.loc.y + r.size.h).clamp(0, sh);
    let w = x1 - x0;
    let h = y1 - y0;
    if w < 1 || h < 1 {
        None
    } else {
        Some(Rectangle::new(Point::from((x0, y0)), Size::from((w, h))))
    }
}


fn windows_bbox_world_of(state: &Loop, ids: &[Uuid]) -> Option<Rectangle<i32, Logical>> {
    let set: HashSet<Uuid> = ids.iter().copied().collect();
    let mut acc: Option<Rectangle<i32, Logical>> = None;
    for w in state.inner.space_state().state.elements() {
        let Some(id) = w.uuid() else { continue };
        if !set.contains(&id) {
            continue;
        }
        if let Some(g) = window_slot_rect(state, w) {
            acc = Some(match acc {
                None => g,
                Some(a) => union(a, g),
            });
        }
    }
    acc
}

fn compute_force_set(state: &mut Loop, target: &CaptureTarget) {
    let set: HashSet<Uuid> = match target {
        CaptureTarget::Windows(ids) => ids.iter().copied().collect(),
        CaptureTarget::FullScreen => HashSet::new(),
        CaptureTarget::WorldRegion(r) => windows_intersecting_world(state, *r),
        CaptureTarget::ScreenRegion(sr) => {
            let world = phys_to_world(state, *sr);
            windows_intersecting_world(state, world)
        }
    };
    state.inner.kernel.get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_MUT).force_set = set;
}

fn windows_intersecting_world(state: &Loop, region: Rectangle<i32, Logical>) -> HashSet<Uuid> {
    state
        .inner.space_state()
        .state
        .elements()
        .filter_map(|w| {
            let id = w.uuid()?;
            let g = state.inner.space_state().state.element_bbox(w)?;
            if rects_overlap(g, region) {
                Some(id)
            } else {
                None
            }
        })
        .collect()
}

fn selected_uuids(state: &Loop) -> Vec<Uuid> {
    state.inner.select()
        
        .Selection
        .iter()
        .filter_map(|w| w.uuid())
        .collect()
}

fn union(a: Rectangle<i32, Logical>, b: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    let x0 = a.loc.x.min(b.loc.x);
    let y0 = a.loc.y.min(b.loc.y);
    let x1 = (a.loc.x + a.size.w).max(b.loc.x + b.size.w);
    let y1 = (a.loc.y + a.size.h).max(b.loc.y + b.size.h);
    Rectangle::new(Point::from((x0, y0)), Size::from((x1 - x0, y1 - y0)))
}

fn rects_overlap(a: Rectangle<i32, Logical>, b: Rectangle<i32, Logical>) -> bool {
    a.loc.x < b.loc.x + b.size.w
        && b.loc.x < a.loc.x + a.size.w
        && a.loc.y < b.loc.y + b.size.h
        && b.loc.y < a.loc.y + a.size.h
}

// ---------------------------------------------------------------------------
// Overlay rect <-> physical rect
// ---------------------------------------------------------------------------

fn full_rect(w: i32, h: i32) -> Rectangle<i32, Physical> {
    Rectangle::new(Point::from((0, 0)), Size::from((w, h)))
}

fn to_overlay(r: Rectangle<i32, Physical>) -> OverlayRect {
    OverlayRect {
        x: r.loc.x,
        y: r.loc.y,
        w: r.size.w,
        h: r.size.h,
    }
}

fn from_overlay(r: OverlayRect) -> Rectangle<i32, Physical> {
    Rectangle::new(Point::from((r.x, r.y)), Size::from((r.w, r.h)))
}

// ---------------------------------------------------------------------------
// Iced registry plumbing
// ---------------------------------------------------------------------------

/// Every mapped output's `output_key` + physical mode size (w, h). Used to place
/// one overlay instance per monitor. Outputs without a current mode are skipped.
fn output_sizes(state: &Loop) -> Vec<(String, (i32, i32))> {
    state
        .inner.space_state()
        .state
        .outputs()
        .filter_map(|o| {
            let key = compositor_orchestration_core_state_base::state::output_key(o);
            o.current_mode().map(|m| (key, (m.size.w, m.size.h)))
        })
        .collect()
}

/// Bind a screen-space overlay surface to a physical output, so the scene
/// builder draws it only on that monitor and the pointer path hit-tests it only
/// when the cursor is on that monitor (see `IcedItem::output`).
fn bind_output(state: &mut Loop, id: HandleId, key: &str) {
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        reg.set_output_affinity_by_id(id, Some(key.to_string()));
    }
}

/// Install a message handler that forwards a capture UI's messages onto the
/// surface message channel (drained by the surface pump → `handle`).
fn forward<U>(state: &mut Loop, handle: IcedHandle<U>)
where
    U: IcedUi<Message = CaptureMessage>,
{
    let tx = state.inner.surface_mut().surface_message_buffer_channel.0.clone();
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        if let Some(inst) = reg.instance_mut(handle) {
            inst.runtime_mut()
                .set_message_handler(move |m: &CaptureMessage| {
                    let _ = tx.send(SurfaceMessage {
                        message: SurfaceMessageType::Capture(m.clone()),
                    });
                });
        }
    }
}

fn set_region_msg<U>(state: &mut Loop, id: Option<HandleId>, rect: OverlayRect)
where
    U: IcedUi<Message = CaptureMessage>,
{
    let Some(id) = id else { return };
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        let _ = reg.dispatch_message(IcedHandle::<U>::from_id(id), CaptureMessage::SetRegion(rect));
    }
}

fn destroy(state: &mut Loop, id: Option<HandleId>) {
    let Some(id) = id else { return };
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        reg.destroy_by_id(id);
    }
}
