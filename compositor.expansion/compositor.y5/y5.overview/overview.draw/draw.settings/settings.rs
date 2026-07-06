//! Settings-tab embed: the settings iced surface below the menu bar while the
//! Settings tab is active. Replaces the Super+. mount, reusing the UI + driver.
use compositor_monitor_compositor_iced_base::{HandleId, IcedHandle, IcedSpace};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_draw_layer_base::base::Layer;
use compositor_orchestration_driver_audio_base::base::AUDIO;
use compositor_orchestration_driver_output_base::base::{OutputModeRequest, OutputsSnapshot, OUTPUTS_SNAPSHOT, OUTPUT_MODE_REQUEST_MUT, OUTPUT_MODE_RESULT_MUT};
use compositor_orchestration_driver_settings_base::base::{SETTINGS, SETTINGS_MUT};
use compositor_configurator_network_backend_base::base::{self as wifi, WifiCmd, WifiSnapshot};
use compositor_configurator_bluetooth_backend_base::base::{self as bt, BtCmd, BtSnapshot};
use compositor_configurator_settings_surface_message::message::{SettingsMessage, ShaderProp, ShaderPropKind};
use compositor_configurator_settings_surface_view::Settings;
use compositor_y5_audio_controller_interface::interface::{AudioState, AudioWatch};
use compositor_y5_surface_draw_handle::handle::load;
use compositor_y5_surface_protocol_base::protocol::{SurfaceMessage, SurfaceMessageType};
use compositor_y5_overview_state_base::base::MENU_BAR_HEIGHT;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Rectangle, Size};
use std::cell::RefCell;

thread_local! {
    /// Last snapshot pushed — only re-dispatch (re-render) the UI when it changes.
    static LAST: RefCell<Option<(AudioState, WifiSnapshot, BtSnapshot)>> = const { RefCell::new(None) };
    /// Our subscription to the audio controller for this open settings surface.
    /// Created lazily while the surface is up, dropped in `destroy` — dropping it
    /// unsubscribes (the controller prunes the sender on its next broadcast).
    static AUDIO_WATCH: RefCell<Option<AudioWatch>> = const { RefCell::new(None) };
    /// Last connected-monitor list pushed — re-dispatch the picker only on hotplug change.
    static LAST_OUTPUTS: RefCell<Option<OutputsSnapshot>> = const { RefCell::new(None) };
    /// Last (bundles, selection, variables, preview source) pushed to the panel.
    #[allow(clippy::type_complexity)]
    static LAST_SHADERS: RefCell<Option<(Vec<String>, Option<String>, Vec<ShaderProp>, String, Option<String>)>> = const { RefCell::new(None) };
    /// Output size the surface was last sized to. The settings surface is
    /// screen-space and spans the output, so a mode/resolution change invalidates
    /// its rect — re-size only when this drifts (resizes reallocate a texture).
    static SIZED: RefCell<Option<Size<i32, Physical>>> = const { RefCell::new(None) };
}

/// Screen rect for the settings surface at the given output size: full width,
/// below the menu bar. Single source of truth for `create` and the resize check.
fn settings_rect(size: Size<i32, Physical>) -> Rectangle<i32, Physical> {
    Rectangle::new(Point::from((0, MENU_BAR_HEIGHT)), Size::from((size.w, (size.h - MENU_BAR_HEIGHT).max(1))))
}

/// The available shader bundles, the active world's resolved selection, and the
/// selected shader's editable variables (with current values), for the picker
/// + the variable controls. Resolution: world override → preference default.
#[allow(clippy::type_complexity)]
fn shader_state(state: &Loop) -> (Vec<String>, Option<String>, Vec<ShaderProp>, String, Option<String>) {
    // The compiled-in built-in worlds first, then every user bundle on disk.
    let mut options: Vec<String> = compositor_background_two_shader_builtin::builtins()
        .iter()
        .map(|b| b.id.to_string())
        .collect();
    options.extend(compositor_background_two_shader_locate::list_bundles());
    let two = state
        .inner
        .worlds
        .active()
        .storage()
        .try_get(&compositor_background_two_storage_base::base::BG_TWO);
    // The selected shader's compile error (set by the background system on load).
    let status = two.and_then(|t| t.shader_error.clone());
    let current = two
        .and_then(|t| t.background_shader.clone())
        .or_else(compositor_developer_stats_registry_base::base::background_shader_default);
    let overrides = two.map(|t| t.params.clone()).unwrap_or_default();

    // Properties for the resolved shader (user source, or the built-in list).
    let props = match &current {
        Some(sel) => compositor_background_two_shader_load::properties_for(sel),
        None => compositor_background_two_shader_builtin::builtin_props(),
    };
    // Effective value per prop: this world's override (matched by name) or the
    // declared default.
    let defaults = compositor_background_two_shader_property::default_params(&props);
    let dtos = props
        .iter()
        .take(16)
        .enumerate()
        .map(|(slot, p)| {
            let value = overrides
                .iter()
                .find(|(n, _)| n == &p.name)
                .map(|(_, v)| *v)
                .unwrap_or(defaults[slot]);
            ShaderProp {
                name: p.name.clone(),
                label: p.label.clone().unwrap_or_else(|| p.name.clone()),
                kind: match p.default {
                    compositor_background_two_shader_property::PropValue::Bool(_) => ShaderPropKind::Bool,
                    _ => ShaderPropKind::Float,
                },
                slot,
                min: p.min.unwrap_or(0.0),
                max: p.max.unwrap_or(1.0).max(p.min.unwrap_or(0.0) + 0.0001),
                value,
            }
        })
        .collect();
    // Preview source: the selected shader's WGSL (vulkan/ or wgsl/ bundle), else
    // the built-in parallax WGSL so the preview always renders something valid.
    let preview = current
        .as_deref()
        .and_then(compositor_background_two_shader_load::preview_wgsl)
        .unwrap_or_else(|| {
            compositor_background_two_draw_vulkan::vulkan::PARALLAX_WGSL.to_string()
        });
    (options, current, dtos, preview, status)
}

pub fn per_frame(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    // Runs once per output in the render loop; the settings window is a screen-space
    // surface that belongs to the ACTIVE monitor only. Act only on the active
    // output's pass, so `size` is that monitor's and it isn't created/resized on
    // every other output. (render_output None = single/non-loop pass → run.)
    if let Some(k) = &state.inner.render_output {
        if *k != state.inner.active_output_key() {
            return;
        }
    }
    let shown = state.inner.overview().visible
        && state.inner.overview().overlay_ready()
        && state.inner.overview().is_settings();
    match (shown, state.inner.kernel.get(&SETTINGS).handle, state.inner.kernel.get(&SETTINGS).open) {
        (true, None, _) => create(state, renderer, size),
        (true, Some(id), false) => { destroy(state, id); compositor_y5_overview_interface_base::base::request_close(state); } // panel Close
        (true, Some(id), true) => sync(state, id, size),
        (false, Some(id), _) => destroy(state, id),
        (false, None, _) => {}
    }
}

fn sync(state: &mut Loop, id: HandleId, size: Size<i32, Physical>) {
    // Output size can change while settings is open (mode/resolution change). The
    // surface is screen-space and was sized to the output at create, so re-derive
    // the rect and resize/move when the output size drifts.
    let resized = SIZED.with(|s| { let mut s = s.borrow_mut(); if *s != Some(size) { *s = Some(size); true } else { false } });
    if resized {
        let rect = settings_rect(size);
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            reg.request_resize_by_id(id, rect.size);
            reg.set_location_by_id(id, rect.loc);
        }
        // A resize alone doesn't re-lay-out the iced content, so force a re-render
        // next frame by invalidating the change-detection cache below (the
        // SyncSystem dispatch repaints the panel at the new size).
        LAST.with(|l| *l.borrow_mut() = None);
    }
    // The controller pings this watch (from its PulseAudio thread) when the audio
    // topology changes; we own the re-poll. Ensure we hold a subscription, and on
    // a ping re-query via refresh() — otherwise read the cheap cached state().
    if AUDIO_WATCH.with(|w| w.borrow().is_none()) {
        if let Some(a) = state.inner.kernel.get(&AUDIO) {
            AUDIO_WATCH.with(|w| *w.borrow_mut() = Some(a.watch()));
        }
    }
    let pinged = AUDIO_WATCH.with(|w| w.borrow().as_ref().map(|w| w.pinged()).unwrap_or(false));
    let audio = match state.inner.kernel.get(&AUDIO) {
        Some(a) => { if pinged { let _ = a.refresh(); } a.state() }
        None => AudioState::default(),
    };
    let cur = (audio, wifi::snapshot(), bt::snapshot());
    let changed = LAST.with(|l| { let mut l = l.borrow_mut(); if l.as_ref() != Some(&cur) { *l = Some(cur.clone()); true } else { false } });
    if changed {
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            let _ = reg.dispatch_message(IcedHandle::<Settings>::from_id(id), SettingsMessage::SyncSystem(cur.0, cur.1, cur.2));
        }
    }
    // Live hotplug refresh of the monitor picker: re-dispatch only when the
    // connected-monitor list changes (reconcile/wire.entry write OUTPUTS_SNAPSHOT).
    let outs = state.inner.kernel.get(&OUTPUTS_SNAPSHOT).clone();
    let outs_changed = LAST_OUTPUTS.with(|l| { let mut l = l.borrow_mut(); if l.as_ref() != Some(&outs) { *l = Some(outs.clone()); true } else { false } });
    if outs_changed {
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            let _ = reg.dispatch_message(IcedHandle::<Settings>::from_id(id), SettingsMessage::SyncDisplays(outs.displays));
        }
    }
    // Available shader bundles + the active world's selection + the selected
    // shader's variables: re-dispatch only when any of them change (a world
    // switch, a folder edit, or a param edit).
    let shaders = shader_state(state);
    let shaders_changed = LAST_SHADERS.with(|l| { let mut l = l.borrow_mut(); if l.as_ref() != Some(&shaders) { *l = Some(shaders.clone()); true } else { false } });
    if shaders_changed {
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            let (options, current, props, preview, status) = shaders;
            let handle = IcedHandle::<Settings>::from_id(id);
            let _ = reg.dispatch_message(handle, SettingsMessage::SyncShaders(options, current));
            let _ = reg.dispatch_message(handle, SettingsMessage::SyncShaderProps(props));
            let _ = reg.dispatch_message(handle, SettingsMessage::SyncShaderPreview(preview));
            let _ = reg.dispatch_message(handle, SettingsMessage::SyncShaderStatus(status));
        }
    }
    // Animate the live preview: while the Current-World tab is open, dispatch a
    // per-frame tick so the surface re-renders and the preview clock advances.
    if compositor_configurator_settings_surface_message::message::Tab::from_index(
        state.inner.kernel.get(&SETTINGS).tab,
    ) == compositor_configurator_settings_surface_message::message::Tab::World
    {
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            let _ = reg.dispatch_message(IcedHandle::<Settings>::from_id(id), SettingsMessage::Tick);
        }
    }
    // One-shot mode-apply result → UI (drops the confirm bar; restores the shown
    // mode on auto-revert / failure, commits on Keep).
    let result = state.inner.kernel.get_mut(&OUTPUT_MODE_RESULT_MUT).take();
    if let Some(r) = result {
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            let _ = reg.dispatch_message(IcedHandle::<Settings>::from_id(id), SettingsMessage::ModeResult(r));
        }
    }
}

fn create(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    let rect = settings_rect(size);
    SIZED.with(|s| *s.borrow_mut() = Some(size));
    let env = compositor_developer_environment_config_base::base::read_current();
    state.inner.preference = compositor_developer_environment_preference_base::base::load();
    state.inner.keybinding = compositor_developer_environment_keybinding_base::base::load();
    let cursor = state.inner.preference.cursor_sensitivity as f32;
    let natural = state.inner.preference.input_natural_scroll;
    let show_fps = state.inner.preference.show_fps;
    let release_hidden = state.inner.preference.release_hidden_surfaces;
    let snap = state.inner.kernel.get(&OUTPUTS_SNAPSHOT).clone();
    let mut keys = compositor_y5_overlay_interface_keyboard::keyboard::registry(&state.inner.keybinding);
    keys.extend(compositor_y5_canvas_input_keyboard::navigator::registry(&state.inner.keybinding));
    keys.extend(compositor_y5_canvas_input_keyboard::navigator::fixed());
    keys.extend(compositor_y5_overlay_interface_keyboard::keyboard::fixed());
    let tab = compositor_configurator_settings_surface_message::message::Tab::from_index(state.inner.kernel.get(&SETTINGS).tab);
    let layout = state.inner.preference.outputs_layout.clone();
    let cyclic = state.inner.preference.teleport_cyclic;
    let ime = state.inner.preference.ime.clone().unwrap_or_default();
    let keyboard = state.inner.preference.keyboard.clone();
    let ui = Settings::new(env, cursor, natural, show_fps, release_hidden, snap, keys, tab, layout, cyclic, ime, keyboard);
    let handle = load(state, renderer, ui, rect, IcedSpace::Screen, Layer::SCENE.bits());
    install_handler(state, handle);
    let untyped = handle.untyped();
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() { reg.set_keyboard_focus(Some(untyped)); }
    let st = state.inner.kernel.get_mut(&SETTINGS_MUT);
    st.handle = Some(untyped);
    st.open = true;
    if let Some(a) = state.inner.kernel.get(&AUDIO) { let _ = a.refresh(); }
    wifi::command(WifiCmd::Scan);
    bt::command(BtCmd::Scan(true));
    LAST.with(|l| *l.borrow_mut() = None);
    LAST_OUTPUTS.with(|l| *l.borrow_mut() = None);
    LAST_SHADERS.with(|l| *l.borrow_mut() = None);
}

fn destroy(state: &mut Loop, id: HandleId) {
    // Closing settings (Esc / overview-tab switch / overview close) abandons any
    // provisional mode change → revert it (no-op if nothing is pending).
    *state.inner.kernel.get_mut(&OUTPUT_MODE_REQUEST_MUT) = Some(OutputModeRequest::Revert);
    state.inner.ping_control();
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() { reg.destroy_by_id(id); reg.set_keyboard_focus(None); }
    // Drop our audio subscription — unsubscribes until the surface reopens.
    AUDIO_WATCH.with(|w| *w.borrow_mut() = None);
    bt::command(BtCmd::Scan(false));
    let st = state.inner.kernel.get_mut(&SETTINGS_MUT);
    st.handle = None;
    st.open = false;
}

fn install_handler(state: &mut Loop, handle: IcedHandle<Settings>) {
    let tx = state.inner.surface_mut().surface_message_buffer_channel.0.clone();
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        if let Some(inst) = reg.instance_mut(handle) {
            inst.runtime_mut().set_message_handler(move |m: &SettingsMessage| {
                // `StageActive` IS forwarded now — CHECK CHANGES applies an
                // activate/deactivate live-provisionally through the handler (arming the
                // auto-revert gate), so it must reach `interface.handle`. The rest here
                // are UI-local (sync pushes, tab/selection state) and never forwarded.
                if matches!(m, SettingsMessage::SyncSystem(..) | SettingsMessage::SyncDisplays(_) | SettingsMessage::SyncShaders(..) | SettingsMessage::SyncShaderProps(..) | SettingsMessage::SyncShaderPreview(..) | SettingsMessage::SyncShaderStatus(..) | SettingsMessage::Tick | SettingsMessage::WifiSelect(_) | SettingsMessage::WifiPassword(_) | SettingsMessage::SelectDisplay(_) | SettingsMessage::SelectMode(_) | SettingsMessage::SelectInactive) { return; }
                let _ = tx.send(SurfaceMessage { message: SurfaceMessageType::Settings(m.clone()) });
            });
        }
    }
}
