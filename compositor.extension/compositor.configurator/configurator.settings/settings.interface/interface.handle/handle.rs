//! Applies forwarded settings-window messages (drained from the surface pump).
//! Preferences are mutated on the live `inner.preference` object (so the change
//! takes effect immediately) and then persisted to preferences.json. Environment
//! edits write settings.json (the UI already flagged the reboot banner); output
//! modes go via the OUTPUT_MODE_REQUEST channel (apply / confirm+persist / revert).
use compositor_model_environment_preference_base::base as pref;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_driver_output_base::base::{ActiveRevert, ApplyResult, OutputModeRequest, OUTPUTS_SNAPSHOT, OUTPUT_ACTIVE_REVERT, OUTPUT_ACTIVE_REVERT_MUT, OUTPUT_MODE_REQUEST_MUT, OUTPUT_MODE_RESULT_MUT, OUTPUT_RECONCILE_REQUEST_MUT};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use compositor_orchestration_driver_settings_base::base::{PenCapture, SETTINGS_MUT};
use compositor_configurator_settings_surface_message::message::{SettingsMessage, Tab};
use compositor_configurator_network_backend_base::base::{self as wifi, WifiCmd};
use compositor_configurator_bluetooth_backend_base::base::{self as bt, BtCmd};
use compositor_orchestration_driver_audio_base::base::AUDIO;
use smithay::backend::renderer::gles::GlesRenderer;

/// How long a provisional activate/deactivate survives without an explicit APPLY
/// before auto-reverting — mirrors the resolution gate's `CONFIRM_TIMEOUT`.
const ACTIVE_CONFIRM_TIMEOUT: Duration = Duration::from_secs(15);
/// Monotonic arm counter: stamps each provisional activation so a superseded one-shot
/// watchdog (a prior gate confirmed/reverted, then a NEW gate armed within the
/// timeout) no-ops instead of reverting the current gate.
static ACTIVE_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Restore the baseline of the armed provisional activate/deactivate (if it is still
/// the current gate) and reconcile back — shared by the REVERT button and the
/// auto-revert watchdog. `expect_epoch` is `Some` for the watchdog (act only if it is
/// still the live gate) and `None` for an explicit REVERT (always undo the current
/// gate). Emits `Reverted` so the settings UI drops the confirm bar and re-syncs.
fn active_revert_now(state: &mut Loop, expect_epoch: Option<u64>) {
    let Some(rev) = state.inner.kernel.get(&OUTPUT_ACTIVE_REVERT).clone() else { return };
    if let Some(e) = expect_epoch {
        if rev.epoch != e {
            return; // a newer gate (or a confirm) superseded this watchdog — stale.
        }
    }
    *state.inner.kernel.get_mut(&OUTPUT_ACTIVE_REVERT_MUT) = None;
    pref::set_active(&mut state.inner.preference.outputs, &rev.edid, rev.prior_active);
    let _ = pref::save(&state.inner.preference);
    *state.inner.kernel.get_mut(&OUTPUT_RECONCILE_REQUEST_MUT) = true;
    state.inner.ping_control();
    *state.inner.kernel.get_mut(&OUTPUT_MODE_RESULT_MUT) = Some(ApplyResult::Reverted);
}

/// Currently-connected monitors' EDID keys (from the kernel snapshot) — the set the
/// live teleport map is filtered against when it is rebuilt.
fn connected_keys(state: &Loop) -> Vec<String> {
    state
        .inner
        .kernel
        .get(&OUTPUTS_SNAPSHOT)
        .displays
        .iter()
        .filter(|d| d.connected)
        .map(|d| d.edid_key.clone())
        .collect()
}

/// Set the active world's background pan inversion (per axis; `None` leaves an axis
/// unchanged): persist it on the world's `Two` slot and flip the live instance in
/// place so the background reacts without a rebuild.
fn set_world_invert(state: &mut Loop, x: Option<bool>, y: Option<bool>) {
    let world = state.inner.worlds.active_id();
    if let Some(two) = state
        .inner
        .worlds
        .active_mut()
        .storage_mut()
        .try_get_mut(&compositor_background_two_storage_base::base::BG_TWO_MUT)
    {
        if let Some(v) = x { two.invert_pan_x = v; }
        if let Some(v) = y { two.invert_pan_y = v; }
        let (ix, iy) = (two.invert_pan_x, two.invert_pan_y);
        if let Some(inst) = two.instance.as_mut() {
            inst.invert_pan_x = ix;
            inst.invert_pan_y = iy;
        }
        compositor_support_system_persist_mark_base::base::mark_world(world, true);
    }
}

/// Set the active world's sRGB background output: persist it on the world's `Two`
/// slot and flip the live instance in place (no rebuild — it's a shader-side encode).
fn set_world_srgb(state: &mut Loop, on: bool) {
    let world = state.inner.worlds.active_id();
    if let Some(two) = state
        .inner
        .worlds
        .active_mut()
        .storage_mut()
        .try_get_mut(&compositor_background_two_storage_base::base::BG_TWO_MUT)
    {
        two.srgb = on;
        if let Some(inst) = two.instance.as_mut() {
            inst.srgb = on;
        }
        compositor_support_system_persist_mark_base::base::mark_world(world, true);
    }
}

/// Set the active world's optimized background variant: persist it on the world's
/// `Two` slot, then get the change on screen the cheapest way that works.
///
/// The stock parallax has both variants compiled in and re-picks per draw, so it
/// flips in place. A runtime-loaded shader baked the flag into its SPIR-V when it
/// was compiled, so it has to be cleared for `TwoSystem` to rebuild it next frame
/// — flipping the bool alone would leave the old module rendering, and because the
/// renderer's pipeline cache is keyed on `(id, format)` and ignores the bytes on a
/// hit, it would stay stale until restart.
fn set_world_optimized(state: &mut Loop, on: bool) {
    let world = state.inner.worlds.active_id();
    if let Some(two) = state
        .inner
        .worlds
        .active_mut()
        .storage_mut()
        .try_get_mut(&compositor_background_two_storage_base::base::BG_TWO_MUT)
    {
        two.optimized = on;
        match two.instance.as_mut() {
            Some(i) if i.uses_loaded_shader() => two.instance = None,
            Some(i) => i.optimized = on,
            None => {}
        }
        compositor_support_system_persist_mark_base::base::mark_world(world, true);
    }
}

pub fn handle(state: &mut Loop, _renderer: &mut GlesRenderer, m: SettingsMessage) {
    match m {
        SettingsMessage::Cursor(v) => {
            state.inner.preference.cursor_sensitivity = v as f64;
            let _ = pref::save(&state.inner.preference);
        }
        SettingsMessage::SetGraphics(g) => {
            // Persist to preferences.json; `pref::save` also pushes the config to
            // the kernel-readable global, so the renderer applies it live.
            state.inner.preference.graphics = g;
            let _ = pref::save(&state.inner.preference);
        }
        SettingsMessage::SetPen(p) => {
            // The input handlers read `preference.pen` live per event, so this applies
            // immediately; save persists it for the next launch.
            state.inner.preference.pen = p;
            let _ = pref::save(&state.inner.preference);
        }
        // Pen click-to-bind: arm/disarm the capture the input handlers watch. The
        // reconciler applies the captured result to the live pen config + syncs the UI.
        SettingsMessage::PenCaptureKey(target) => {
            let st = state.inner.kernel.get_mut(&SETTINGS_MUT);
            st.pen_capture = PenCapture::Key;
            st.pen_capture_target = Some(target);
        }
        SettingsMessage::PenCapturePad => {
            state.inner.kernel.get_mut(&SETTINGS_MUT).pen_capture = PenCapture::Pad;
        }
        SettingsMessage::PenCaptureCancel => {
            let st = state.inner.kernel.get_mut(&SETTINGS_MUT);
            st.pen_capture = PenCapture::None;
            st.pen_capture_target = None;
        }
        SettingsMessage::NaturalScroll(b) => {
            state.inner.preference.input_natural_scroll = b;
            let _ = pref::save(&state.inner.preference);
        }
        SettingsMessage::TouchPanSpeed(v) => {
            // Read live per touch event, so this takes effect immediately; save
            // persists it for the next launch.
            state.inner.preference.input_touch_pan_speed = v as f64;
            let _ = pref::save(&state.inner.preference);
        }
        SettingsMessage::TouchLinearPan(b) => {
            state.inner.preference.input_touch_linear_pan = b;
            let _ = pref::save(&state.inner.preference);
        }
        SettingsMessage::OskSize(v) => {
            state.inner.preference.osk_size = v as f64;
            let _ = pref::save(&state.inner.preference);
        }
        SettingsMessage::OskWorldPosition(b) => {
            state.inner.preference.osk_world_position = b;
            let _ = pref::save(&state.inner.preference);
        }
        SettingsMessage::SetShowFps(b) => {
            state.inner.preference.show_fps = b;
            let _ = pref::save(&state.inner.preference);
        }
        SettingsMessage::SetReleaseHidden(b) => {
            state.inner.preference.release_hidden_surfaces = b;
            let _ = pref::save(&state.inner.preference);
        }
        SettingsMessage::SetTripleBuffer(tb) => {
            // Live: `pref::save` republishes into the process-global the worker
            // re-reads each pass, so the knobs apply without a restart. Only
            // `enabled` waits for the next start.
            state.inner.preference.background_triple_buffer = tb;
            let _ = pref::save(&state.inner.preference);
        }
        SettingsMessage::SetInterfaceBuffer(s) => {
            // `slots` and `pipeline` are live — the surfaces re-read the published
            // setting each frame. `enabled` is not: on Vulkan it also decides
            // whether bevy's worker thread exists, so it takes effect next start.
            state.inner.preference.interface_triple_buffer = s.normalized();
            let _ = pref::save(&state.inner.preference);
        }
        SettingsMessage::SetFractionalInvisible(v) => {
            // Invisible-window fractional-scale strategy ("off"/"optimized"/"full").
            // Read live each frame by `update_fractional`, so no reboot needed.
            state.inner.preference.fractional_invisible = v;
            let _ = pref::save(&state.inner.preference);
        }
        SettingsMessage::SetFlip(t) => {
            // Page-flip policy. `pref::save` mirrors it into the scanout-readable
            // global, so the next frame picks it up — no reboot. `normalized()`
            // there also clamps the stepper values.
            // Normalize HERE, not just in the mirror: the stored value is what
            // the steppers read back next render, so an un-clamped one would let
            // repeated "-" drift the cap negative and need the same number of
            // "+" presses to climb back out of a range that displays identically.
            state.inner.preference.flip = t.normalized();
            let _ = pref::save(&state.inner.preference);
        }
        SettingsMessage::Env(e) => {
            let _ = compositor_model_environment_config_base::base::save(&e);
        }
        SettingsMessage::Ime(ime) => {
            // Persist the input-method launch command live to preferences.json. Applied
            // on the next compositor start (the IME is spawned once at boot).
            state.inner.preference.ime = Some(ime);
            let _ = pref::save(&state.inner.preference);
        }
        SettingsMessage::SetProtocolForeign(v) => {
            // Gate for the wlr + ext foreign-toplevel (dock) protocols. Persist to
            // preferences.json; read once at the next start (globals are bound then).
            state.inner.preference.protocol_foreign = v;
            let _ = pref::save(&state.inner.preference);
        }
        SettingsMessage::SetProtocolForeignAllWorlds(v) => {
            // Advertise windows from all worlds vs just the active one. Unlike
            // `protocol_foreign` (which binds globals, so it's boot-only), this only
            // changes which windows the reconcile publishes — so hot-reload it live:
            // persist, flip the live scope flag, then re-reconcile once. This message
            // IS the event, so the re-advertise is event-driven (no per-frame poll).
            state.inner.preference.protocol_foreign_all_worlds = v;
            let _ = pref::save(&state.inner.preference);
            state.state.foreign.set_all_worlds(v);
            state.foreign_reconcile();
        }
        SettingsMessage::Keyboard(kl) => {
            // Persist AND apply the keyboard layout live: mutate the preference, save,
            // then recompile the keymap on the seat's keyboard. `get_keyboard()` hands
            // back an owned handle, so `&mut state.state` can be borrowed alongside the
            // `&state.inner.preference` read (disjoint fields of `Loop`).
            state.inner.preference.keyboard = kl;
            let _ = pref::save(&state.inner.preference);
            if let Some(keyboard) = state.state.seat.seat.get_keyboard() {
                compositor_support_smithay_state_seat_xkb::xkb::apply(
                    &keyboard,
                    &mut state.state,
                    &state.inner.preference.keyboard,
                );
            }
        }
        SettingsMessage::Apply(a) => {
            // Per-pipe mode change on the SELECTED monitor (multi-output: every output
            // is independently driven, so this is never an active-output switch).
            *state.inner.kernel.get_mut(&OUTPUT_MODE_REQUEST_MUT) = Some(OutputModeRequest::Apply {
                edid_key: a.edid_key,
                width: a.mode.width,
                height: a.mode.height,
                refresh_mhz: a.mode.refresh_mhz,
            });
            state.inner.ping_control();
        }
        SettingsMessage::Keep(a) => {
            *state.inner.kernel.get_mut(&OUTPUT_MODE_REQUEST_MUT) = Some(OutputModeRequest::Confirm);
            // Persist to the SELECTED monitor's profile (multi-output).
            pref::upsert_output(&mut state.inner.preference.outputs, &a.edid_key, pref::ModeRequest::Advertised {
                width: a.mode.width,
                height: a.mode.height,
                refresh_mhz: a.mode.refresh_mhz,
            });
            let _ = pref::save(&state.inner.preference);
            state.inner.ping_control();
        }
        SettingsMessage::Revert => {
            // REVERT resolves whichever fault gate is armed: a provisional
            // activate/deactivate (restore the prior active state + reconcile) takes
            // precedence over a provisional resolution change.
            if state.inner.kernel.get(&OUTPUT_ACTIVE_REVERT).is_some() {
                active_revert_now(state, None);
            } else {
                *state.inner.kernel.get_mut(&OUTPUT_MODE_REQUEST_MUT) = Some(OutputModeRequest::Revert);
                state.inner.ping_control();
            }
        }
        SettingsMessage::Rebind(id, combo) => {
            state.inner.keybinding.set(&id, combo);
            let _ = compositor_model_environment_keybinding_base::base::save(&state.inner.keybinding);
        }
        SettingsMessage::ResetBind(id) => {
            state.inner.keybinding.clear(&id);
            let _ = compositor_model_environment_keybinding_base::base::save(&state.inner.keybinding);
        }
        SettingsMessage::SetDefaultSink(name) => {
            if let Some(a) = state.inner.kernel.get(&AUDIO) {
                let _ = a.set_default_sink(&name);
            }
        }
        SettingsMessage::SetSinkVolume(name, v) => {
            if let Some(a) = state.inner.kernel.get(&AUDIO) {
                let _ = a.set_sink_volume(&name, v as f64);
            }
        }
        SettingsMessage::SetSinkMute(name, muted) => {
            if let Some(a) = state.inner.kernel.get(&AUDIO) {
                let _ = a.set_sink_mute(&name, muted);
            }
        }
        SettingsMessage::WifiEnable(b) => wifi::command(WifiCmd::SetEnabled(b)),
        SettingsMessage::WifiScan => wifi::command(WifiCmd::Scan),
        SettingsMessage::WifiConnect(ssid, pw) => wifi::command(WifiCmd::Connect(ssid, pw)),
        SettingsMessage::BtPower(b) => bt::command(BtCmd::SetPowered(b)),
        SettingsMessage::BtScan(b) => bt::command(BtCmd::Scan(b)),
        SettingsMessage::BtPair(p) => bt::command(BtCmd::Pair(p)),
        SettingsMessage::BtConnect(p) => bt::command(BtCmd::Connect(p)),
        // Set the CURRENT world's background-shader override: write it into the
        // world's own `Two` slot (persisted by `BackgroundDoc` on mark), and clear
        // the instance so `TwoSystem::update` rebuilds next frame. Empty = default.
        SettingsMessage::SetWorldShader(name) => {
            let world = state.inner.worlds.active_id();
            if let Some(two) = state
                .inner
                .worlds
                .active_mut()
                .storage_mut()
                .try_get_mut(&compositor_background_two_storage_base::base::BG_TWO_MUT)
            {
                two.background_shader = if name.is_empty() { None } else { Some(name) };
                two.instance = None;
                compositor_support_system_persist_mark_base::base::mark_world(world, true);
            }
        }
        // Set the current world's shader params: store the full vector on the
        // world's `Two` slot (persisted, debounced — drags fire fast) and update
        // the live instance in place so the background reacts without a rebuild.
        SettingsMessage::SetWorldShaderParams(values) => {
            let world = state.inner.worlds.active_id();
            if let Some(two) = state
                .inner
                .worlds
                .active_mut()
                .storage_mut()
                .try_get_mut(&compositor_background_two_storage_base::base::BG_TWO_MUT)
            {
                two.params = values.clone();
                // Map the name-keyed overrides onto the live instance's param
                // slots (slot = the prop's index in the selected shader's props).
                //
                // From the LOADED bundle when there is one. `properties_for` reads
                // and re-parses the bundle off disk, and this runs on every event
                // of a slider drag; the running bundle already holds the same list
                // and cannot be a stale copy of it.
                let props = match two.props() {
                    Some(p) => p.to_vec(),
                    None => {
                        let selection = two.background_shader.clone().or_else(
                            compositor_model_stats_registry_base::base::background_shader_default,
                        );
                        match &selection {
                            Some(sel) => {
                                compositor_pipeline_bundle_load_base::properties_for(sel)
                            }
                            None => compositor_pipeline_bundle_builtin_base::builtin_props(),
                        }
                    }
                };
                if let Some(inst) = two.instance.as_mut() {
                    for (name, val) in &values {
                        if let Some(slot) = props.iter().position(|p| &p.name == name) {
                            // 16-float param capacity (4×vec4), matching the rebuild
                            // (`draw.select`) and preview paths — an `< 8` cap here
                            // silently dropped live edits to slots 8..16 (e.g. hole_spacing).
                            if slot < 16 {
                                inst.params[slot] = *val;
                            }
                        }
                    }
                }
                compositor_support_system_persist_mark_base::base::mark_world(world, false);
            }
        }
        SettingsMessage::SetWorldInvertPanX(v) => set_world_invert(state, Some(v), None),
        SettingsMessage::SetWorldInvertPanY(v) => set_world_invert(state, None, Some(v)),
        SettingsMessage::SetWorldSrgb(v) => set_world_srgb(state, v),
        SettingsMessage::SetWorldOptimized(v) => set_world_optimized(state, v),
        SettingsMessage::Close => {
            state.inner.kernel.get_mut(&SETTINGS_MUT).open = false;
        }
        // Inbound / UI-local — never forwarded to the handler.
        // Tab IS forwarded (so the compositor knows the visible module): gate the
        // live-FPS push on the Performance tab being open.
        // Purely a view of the shader list — it changes nothing about the world.
        // It is still forwarded, for the same reason `Tab` is: so the choice
        // outlives the surface and the panel reopens where it was left.
        SettingsMessage::SelectShaderCategory(c) => {
            state.inner.kernel.get_mut(&SETTINGS_MUT).shader_category = c;
        }
        SettingsMessage::Tab(t) => {
            let st = state.inner.kernel.get_mut(&SETTINGS_MUT);
            st.fps_wanted = matches!(t, Tab::Performance);
            st.tab = t.to_index(); // remember the module for the session (restored on reopen)
            // Leaving Display abandons any provisional mode change → revert it
            // (a no-op in the kernel if nothing is pending).
            if !matches!(t, Tab::Display) {
                *state.inner.kernel.get_mut(&OUTPUT_MODE_REQUEST_MUT) = Some(OutputModeRequest::Revert);
                state.inner.ping_control();
            }
        }
        // Cursor-teleport layout committed on drag-end: persist the whole
        // arrangement and rebuild the live teleport layout so crossings take effect
        // immediately (no reboot). Layout is teleport-only — no modeset.
        SettingsMessage::LayoutCommit(placements) => {
            pref::set_layout(&mut state.inner.preference, placements);
            let _ = pref::save(&state.inner.preference);
            let keys = connected_keys(state);
            let layout = compositor_orchestration_driver_output_base::base::build_teleport(&state.inner.preference, &keys);
            *state.inner.kernel.get_mut(&compositor_orchestration_driver_output_base::base::TELEPORT_LAYOUT_MUT) = layout;
            // The tracked placement may have been removed/renumbered; re-resolve lazily.
            *state.inner.kernel.get_mut(&compositor_orchestration_driver_output_base::base::CURSOR_PLACEMENT_MUT) = None;
        }
        // CHECK CHANGES on an activate/deactivate: apply it LIVE now and arm the
        // auto-revert watchdog — the same fault gate as a resolution change (so the
        // user SEES a deactivation before committing, and it auto-reverts if they walk
        // away). `None` = deactivate ("Inactive", refused for the last active+connected
        // monitor); `Some(mode)` = (re)activate at `mode`. APPLY (`SetActive`) keeps it;
        // REVERT / timeout restores the prior active state (`active_revert_now`).
        SettingsMessage::StageActive(edid, mode_opt) => {
            if mode_opt.is_none() {
                let active_connected = state
                    .inner
                    .kernel
                    .get(&OUTPUTS_SNAPSHOT)
                    .displays
                    .iter()
                    .filter(|d| d.connected && d.enabled)
                    .count();
                if active_connected <= 1 {
                    // Nothing applied → clear the (optimistically-armed) confirm bar.
                    *state.inner.kernel.get_mut(&OUTPUT_MODE_RESULT_MUT) = Some(ApplyResult::Failed);
                    return; // keep at least one active monitor
                }
            }
            // Baseline to restore on revert, captured BEFORE mutating.
            let prior_active = pref::output_active(&state.inner.preference.outputs, &edid);
            match &mode_opt {
                None => pref::set_active(&mut state.inner.preference.outputs, &edid, false),
                Some(mode) => {
                    pref::set_active(&mut state.inner.preference.outputs, &edid, true);
                    pref::upsert_output(&mut state.inner.preference.outputs, &edid, pref::ModeRequest::Advertised {
                        width: mode.width,
                        height: mode.height,
                        refresh_mhz: mode.refresh_mhz,
                    });
                }
            }
            let _ = pref::save(&state.inner.preference);
            // Reconcile (bring the pipe up / tear it down); the ping drains it
            // input-independently (not on the libinput source).
            *state.inner.kernel.get_mut(&OUTPUT_RECONCILE_REQUEST_MUT) = true;
            state.inner.ping_control();
            // Record the baseline + arm the one-shot auto-revert watchdog.
            let epoch = ACTIVE_EPOCH.fetch_add(1, Ordering::Relaxed) + 1;
            *state.inner.kernel.get_mut(&OUTPUT_ACTIVE_REVERT_MUT) =
                Some(ActiveRevert { edid, prior_active, epoch });
            let _ = state.loop_handle.insert_source(
                Timer::from_duration(ACTIVE_CONFIRM_TIMEOUT),
                move |_, _, state: &mut Loop| {
                    active_revert_now(state, Some(epoch));
                    TimeoutAction::Drop
                },
            );
        }
        // APPLY of a provisional activate/deactivate: KEEP it — just disarm the
        // auto-revert watchdog (the change was already applied + persisted on CHECK).
        SettingsMessage::SetActive(_edid, _mode_opt) => {
            *state.inner.kernel.get_mut(&OUTPUT_ACTIVE_REVERT_MUT) = None;
        }
        SettingsMessage::SetCyclic(b) => {
            state.inner.preference.teleport_cyclic = b;
            let _ = pref::save(&state.inner.preference);
            let keys = connected_keys(state);
            let layout = compositor_orchestration_driver_output_base::base::build_teleport(&state.inner.preference, &keys);
            *state.inner.kernel.get_mut(&compositor_orchestration_driver_output_base::base::TELEPORT_LAYOUT_MUT) = layout;
        }
        // Claim/release a touch device for a monitor: persist to preferences.json.
        // The per-frame pump re-reads the preference and re-sends SyncTouchDevices,
        // so the UI + the touch resolver both reflect it on the next frame.
        SettingsMessage::ClaimTouch(edid, device_id) => {
            pref::set_touch_device(&mut state.inner.preference.outputs, &edid, device_id);
            let _ = pref::save(&state.inner.preference);
        }
        SettingsMessage::Fps(_)
        | SettingsMessage::Tick
        | SettingsMessage::ModeResult(_)
        | SettingsMessage::SyncSystem(..)
        | SettingsMessage::SyncDisplays(_)
        | SettingsMessage::SyncTouchDevices(_)
        | SettingsMessage::SyncShaders(..)
        | SettingsMessage::SyncShaderProps(..)
        | SettingsMessage::SyncShaderPreview(..)
        | SettingsMessage::SyncShaderFacts(..)
        | SettingsMessage::SyncShaderStatus(..)
        | SettingsMessage::SyncShaderNotice(..)
        | SettingsMessage::SyncWorldInvert(..)
        | SettingsMessage::SyncWorldSrgb(..)
        | SettingsMessage::SyncWorldOptimized(..)
        | SettingsMessage::SelectDisplay(_)
        | SettingsMessage::SelectMode(_)
        | SettingsMessage::SelectInactive
        // Layout edits are applied UI-locally in the view; only LayoutCommit forwards.
        | SettingsMessage::LayoutPlace(..)
        | SettingsMessage::LayoutMove(..)
        | SettingsMessage::LayoutResize(..)
        | SettingsMessage::LayoutSelect(_)
        | SettingsMessage::LayoutRemove(_)
        | SettingsMessage::WifiSelect(_)
        | SettingsMessage::WifiPassword(_)
        // Language-tab layout picker open/search: UI-local, handled only in the view.
        | SettingsMessage::LangPickerOpen(_)
        | SettingsMessage::LangSearch(_)
        // Pushed straight to the surface by the reconciler (never forwarded here).
        | SettingsMessage::SyncPen(_) => {}
    }
}
