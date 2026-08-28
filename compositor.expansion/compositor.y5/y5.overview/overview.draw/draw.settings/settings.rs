//! Settings-tab embed: the settings iced surface below the menu bar while the
//! Settings tab is active. Replaces the Super+. mount, reusing the UI + driver.
use compositor_monitor_compositor_iced_base::{HandleId, IcedHandle, IcedSpace};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_draw_layer_base::base::Layer;
use compositor_orchestration_driver_audio_base::base::AUDIO;
use compositor_orchestration_driver_output_base::base::{OutputModeRequest, OutputsSnapshot, OUTPUTS_SNAPSHOT, OUTPUT_MODE_REQUEST_MUT, OUTPUT_MODE_RESULT_MUT, TouchDeviceInfo, TOUCH_DEVICES_SNAPSHOT};
use compositor_orchestration_driver_settings_base::base::{SETTINGS, SETTINGS_MUT};
use compositor_configurator_network_backend_base::base::{self as wifi, WifiCmd, WifiSnapshot};
use compositor_configurator_bluetooth_backend_base::base::{self as bt, BtCmd, BtSnapshot};
use compositor_configurator_settings_surface_message::message::{SettingsMessage, ShaderEntry, ShaderFacts, ShaderProp};
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
    static LAST_TOUCH: RefCell<Option<Vec<TouchDeviceInfo>>> = const { RefCell::new(None) };
    /// Last (bundles, selection, variables, preview source) pushed to the panel.
    #[allow(clippy::type_complexity)]
    static LAST_SHADERS: RefCell<Option<ShaderState>> = const { RefCell::new(None) };
    /// See `bundle_categories`.
    static LAST_CATEGORIES: RefCell<Option<(Vec<String>, Vec<(String, String)>)>> =
        const { RefCell::new(None) };
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
///
/// Named rather than a tuple: it is compared field-for-field, and a dozen
/// mostly-`Option<String>` members are one transposition from a silent swap.
#[derive(Clone, PartialEq)]
struct ShaderState {
    options: Vec<ShaderEntry>,
    current: Option<String>,
    props: Vec<ShaderProp>,
    /// `None` = this selection has no previewable source. See `SyncShaderPreview`.
    preview: Option<String>,
    facts: Option<ShaderFacts>,
    /// The selected shader's compile error, if it failed.
    status: Option<String>,
    /// A non-fatal condition about a shader that IS running.
    notice: Option<String>,
    invert_x: bool,
    invert_y: bool,
    srgb: bool,
    optimized: bool,
    can_optimize: bool,
}

/// Every selectable shader, grouped and in picker order: the stock parallax, the
/// compiled-in built-ins under their own headings, then every bundle on disk.
///
/// Sorted by category first, keeping disk order within each.
fn shader_options() -> Vec<ShaderEntry> {
    let mut out = vec![ShaderEntry {
        value: String::new(),
        label: "Built-in parallax".to_string(),
        category: compositor_pipeline_bundle_builtin_base::DEFAULT_CATEGORY.to_string(),
    }];
    out.extend(compositor_pipeline_bundle_builtin_base::builtins().iter().map(|b| ShaderEntry {
        value: b.id.to_string(),
        label: title_case(b.id.trim_start_matches(
            compositor_pipeline_bundle_builtin_base::BUILTIN_PREFIX,
        )),
        category: b.category.to_string(),
    }));
    // The compiled-in MULTIPASS bundles. Separate from `builtins()` because a
    // `Builtin` is one WGSL source and these are a manifest plus a tree of passes;
    // labelled by bundle name like a disk bundle rather than title-cased like a
    // built-in world, since that is the name their manifest and folder carry.
    out.extend(compositor_pipeline_bundle_embed_base::embed::BUNDLES.iter().map(|b| {
        let value = compositor_pipeline_bundle_embed_base::embed::id_of(b);
        let category = compositor_pipeline_bundle_load_base::category_for(&value);
        ShaderEntry { value, label: b.to_string(), category }
    }));
    let mut bundles: Vec<ShaderEntry> = bundle_categories()
        .into_iter()
        .map(|(value, category)| ShaderEntry { label: value.clone(), value, category })
        .collect();
    // Stable so the disk order survives inside each group.
    bundles.sort_by(|a, b| a.category.cmp(&b.category));
    out.extend(bundles);
    out
}

/// `(bundle, category)` for every bundle on disk, memoized on the bundle LIST.
///
/// A category means parsing that bundle's `pipeline.json`, and this runs every
/// frame the panel is open — ~60 file reads. Only the parse is cached; the
/// readdir behind it is one syscall.
fn bundle_categories() -> Vec<(String, String)> {
    let names = compositor_pipeline_bundle_locate_base::list_bundles();
    LAST_CATEGORIES.with(|c| {
        let mut c = c.borrow_mut();
        if c.as_ref().map(|(n, _)| n) != Some(&names) {
            let resolved = names
                .iter()
                .map(|n| (n.clone(), compositor_pipeline_bundle_load_base::category_for(n)))
                .collect();
            *c = Some((names, resolved));
        }
        c.as_ref().map(|(_, r)| r.clone()).unwrap_or_default()
    })
}

/// `leafy-drift` → `Leafy Drift`. Bundle folder names are shown verbatim; only
/// the built-in ids get this, because only they are known to be kebab-case.
fn title_case(id: &str) -> String {
    id.split(['-', '_'])
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Resolve everything the Current-World panel shows, for the active world.
fn shader_state(state: &Loop) -> ShaderState {
    let options = shader_options();
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
        .or_else(compositor_model_stats_registry_base::base::background_shader_default);
    // A live, non-fatal condition about a shader that IS running. Read from the
    // background layer rather than stored per world because it is a property of
    // what is on screen this instant, not of the selection — it appears and clears
    // as clients come and go, and the change-detect below re-dispatches when it
    // flips. Only meaningful while a bundle is actually selected.
    let notice = (current.is_some()
        && compositor_background_two_draw_offload::offload::textures_unavailable())
    .then(|| {
        "window textures are unavailable right now — a surface on screen cannot be \
         shared with the background thread, so this shader is drawing inline"
            .to_string()
    });
    let overrides = two.map(|t| t.params.clone()).unwrap_or_default();
    // This world's per-axis background pan inversion + sRGB output (default off).
    let (invert_x, invert_y) = two.map(|t| (t.invert_pan_x, t.invert_pan_y)).unwrap_or((false, false));
    let srgb = two.map(|t| t.srgb).unwrap_or(false);
    // The built-in parallax's cheap variant. Only meaningful with no selection —
    // any selected shader loads its own module and has no optimized twin.
    let optimized = two.map(|t| t.optimized).unwrap_or(false);

    // From the LOADED bundle when there is one: same union, no disk read, and by
    // construction the list the running shader is indexed by.
    let props = match (two.and_then(|t| t.props()), &current) {
        (Some(p), _) => p.to_vec(),
        (None, Some(sel)) => compositor_pipeline_bundle_load_base::properties_for(sel),
        (None, None) => compositor_pipeline_bundle_builtin_base::builtin_props(),
    };
    // Shared with the inline editor so the two panels cannot disagree.
    let dtos =
        compositor_configurator_settings_surface_message::message::shader_props(&props, &overrides);
    // Preview source: the selected shader's WGSL (vulkan/ or wgsl/ bundle).
    //
    // The parallax source is the preview only when the built-in is SELECTED. It
    // used to be the fallback for any unpreviewable selection, so every
    // `passes/`-only bundle previewed as a starfield. `None` now says so.
    let preview = match current.as_deref() {
        None => Some(
            match optimized {
                true => compositor_background_two_draw_vulkan::vulkan::PARALLAX_OPTIMIZED_WGSL,
                false => compositor_background_two_draw_vulkan::vulkan::PARALLAX_WGSL,
            }
            .to_string(),
        ),
        Some(s) => compositor_pipeline_bundle_load_base::preview_wgsl(s, optimized),
    };
    // Read off the live pipeline, not the manifest: `place` is resolved at load.
    let facts = two.and_then(|t| t.bundle()).map(shader_facts);
    // Is there anything to optimize? The stock parallax (no selection) always has
    // its twin; a selected shader only does if its source declares `@optimized`.
    let can_optimize = current
        .as_deref()
        .map_or(true, compositor_pipeline_bundle_load_base::supports_optimized);
    ShaderState {
        options,
        current,
        props: dtos,
        preview,
        facts,
        status,
        notice,
        invert_x,
        invert_y,
        srgb,
        optimized,
        can_optimize,
    }
}

/// Flatten a loaded bundle into the panel's properties card. `owns` and `place`
/// read as sentences rather than enum names — they change how the desktop behaves.
fn shader_facts(
    cp: &compositor_pipeline_build_pipeline_base::pipeline::CompiledPipeline,
) -> compositor_configurator_settings_surface_message::message::ShaderFacts {
    use compositor_pipeline_bundle_manifest_base::manifest::{Decorations, Evaluate, LetterboxMode};
    use compositor_pipeline_build_place_base::place::Offload;
    use compositor_pipeline_abi_seam_base::base::WorldOwn;
    let owns = match cp.owns {
        WorldOwn::Engine => "drawn by the engine",
        WorldOwn::Windows => "windows drawn by the shader",
        WorldOwn::World => "whole band drawn by the shader",
    };
    let place = match cp.offload {
        Offload::Whole => "background thread",
        Offload::BeforeBand => "background thread, then the compositor",
        Offload::AfterBand => "the compositor, then the background thread",
        Offload::None => "the compositor thread",
    };
    let warp = cp.warp.as_ref().map(|_| {
        match cp.warp_evaluate {
            Evaluate::MapStatic => "displaced; corrected from a baked map",
            Evaluate::Pointwise => "displaced; corrected per event",
            Evaluate::Map => "displaced; corrected from a per-frame GPU map",
        }
        .to_string()
    });
    let chrome = match (cp.decorations == Decorations::Off, cp.letterbox) {
        (false, LetterboxMode::Keep) => None,
        (true, LetterboxMode::Keep) => Some("no border".to_string()),
        (false, l) => Some(format!("letterbox {l:?}")),
        (true, l) => Some(format!("no border, letterbox {l:?}")),
    };
    compositor_configurator_settings_surface_message::message::ShaderFacts {
        before: cp.before.len(),
        after: cp.after.len(),
        owns: owns.to_string(),
        place: place.to_string(),
        requires: compositor_pipeline_bundle_require_base::require::Requirement::ALL
            .iter()
            .filter(|r| cp.requires.has(**r))
            .map(|r| (r.to_string(), r.cost().to_string()))
            .collect(),
        warp,
        chrome,
    }
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
    match (shown, state.inner.settings_surface(), state.inner.kernel.get(&SETTINGS).open) {
        (true, None, _) => create(state, renderer, size),
        (true, Some(id), false) => { destroy(state, id); compositor_y5_overview_interface_base::base::request_close(state); } // panel Close
        (true, Some(id), true) => sync(state, id, size),
        (false, Some(id), _) => destroy(state, id),
        // The panel was torn down by `OverviewSystem::on_disable` (this world went
        // inactive), which owns the surface but not the process-wide resources.
        (false, None, true) => release(state),
        (false, None, false) => {}
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
    // Pen click-to-bind: the input handlers recorded a captured key combo / pad button
    // into SettingsState. Apply it to the live pen config, persist, and sync the tab.
    let cap_key = state.inner.kernel.get_mut(&SETTINGS_MUT).pen_captured_key.take();
    let cap_pad = state.inner.kernel.get_mut(&SETTINGS_MUT).pen_captured_pad.take();
    let mut pen_changed = false;
    if let Some(bind) = cap_key {
        let target = state.inner.kernel.get_mut(&SETTINGS_MUT).pen_capture_target.take();
        if let Some(target) = target {
            state.inner.preference.pen.set_key_bind(&target, bind);
            pen_changed = true;
        }
    }
    if let Some((device, button)) = cap_pad {
        state.inner.preference.pen.add_pad_button(&device, button);
        pen_changed = true;
    }
    if pen_changed {
        let _ = compositor_model_environment_preference_base::base::save(&state.inner.preference);
        let pen = state.inner.preference.pen.clone();
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            let _ = reg.dispatch_message(IcedHandle::<Settings>::from_id(id), SettingsMessage::SyncPen(pen));
        }
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
    // Live touch-device list + per-monitor claim. Merge the kernel's device snapshot
    // (id/name) with the live preference (which monitor claimed each), so a hotplug
    // OR a claim change re-dispatches. Diff on the merged result.
    let touch: Vec<TouchDeviceInfo> = {
        let outputs = &state.inner.preference.outputs;
        state.inner.kernel.get(&TOUCH_DEVICES_SNAPSHOT).devices.iter().map(|d| TouchDeviceInfo {
            id: d.id.clone(),
            name: d.name.clone(),
            assigned_edid: outputs.iter()
                .find(|p| p.touch_device.as_deref() == Some(d.id.as_str()))
                .and_then(|p| p.identity.clone()),
        }).collect()
    };
    let touch_changed = LAST_TOUCH.with(|l| { let mut l = l.borrow_mut(); if l.as_ref() != Some(&touch) { *l = Some(touch.clone()); true } else { false } });
    if touch_changed {
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            let _ = reg.dispatch_message(IcedHandle::<Settings>::from_id(id), SettingsMessage::SyncTouchDevices(touch));
        }
    }
    // Available shader bundles + the active world's selection + the selected
    // shader's variables: re-dispatch only when any of them change (a world
    // switch, a folder edit, or a param edit).
    let shaders = shader_state(state);
    let shaders_changed = LAST_SHADERS.with(|l| { let mut l = l.borrow_mut(); if l.as_ref() != Some(&shaders) { *l = Some(shaders.clone()); true } else { false } });
    if shaders_changed {
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            let s = shaders;
            let handle = IcedHandle::<Settings>::from_id(id);
            let _ = reg.dispatch_message(handle, SettingsMessage::SyncShaders(s.options, s.current));
            let _ = reg.dispatch_message(handle, SettingsMessage::SyncShaderProps(s.props));
            let _ = reg.dispatch_message(handle, SettingsMessage::SyncShaderPreview(s.preview));
            let _ = reg.dispatch_message(handle, SettingsMessage::SyncShaderFacts(s.facts));
            let _ = reg.dispatch_message(handle, SettingsMessage::SyncShaderStatus(s.status));
            let _ = reg.dispatch_message(handle, SettingsMessage::SyncShaderNotice(s.notice));
            let _ = reg.dispatch_message(handle, SettingsMessage::SyncWorldInvert(s.invert_x, s.invert_y));
            let _ = reg.dispatch_message(handle, SettingsMessage::SyncWorldSrgb(s.srgb));
            let _ = reg.dispatch_message(handle, SettingsMessage::SyncWorldOptimized(s.optimized, s.can_optimize));
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
    let env = compositor_model_environment_config_base::base::read_current();
    state.inner.preference = compositor_model_environment_preference_base::base::load();
    state.inner.keybinding = compositor_model_environment_keybinding_base::base::load();
    let cursor = state.inner.preference.cursor_sensitivity as f32;
    let natural = state.inner.preference.input_natural_scroll;
    let edge_pan = state.inner.preference.input_edge_pan;
    let edge_pan_speed = state.inner.preference.input_edge_pan_speed as f32;
    let edge_pan_continuous = state.inner.preference.input_edge_pan_continuous;
    let touch_pan_speed = state.inner.preference.input_touch_pan_speed as f32;
    let touch_linear_pan = state.inner.preference.input_touch_linear_pan;
    let osk_size = state.inner.preference.osk_size as f32;
    let osk_world_position = state.inner.preference.osk_world_position;
    let show_fps = state.inner.preference.show_fps;
    let release_hidden = state.inner.preference.release_hidden_surfaces;
    let background_triple_buffer = state.inner.preference.background_triple_buffer;
    let interface_triple_buffer = state.inner.preference.interface_triple_buffer;
    let fractional_invisible = state.inner.preference.fractional_invisible.clone();
    let flip = state.inner.preference.flip;
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
    let protocol_foreign = state.inner.preference.protocol_foreign.clone();
    let protocol_foreign_all_worlds = state.inner.preference.protocol_foreign_all_worlds;
    let session_capture = state.inner.preference.session_capture.clone();
    let pen = state.inner.preference.pen.clone();
    let ui = Settings::new(env, cursor, natural, edge_pan, edge_pan_speed, edge_pan_continuous, touch_pan_speed, touch_linear_pan, osk_size, osk_world_position, show_fps, release_hidden, fractional_invisible, background_triple_buffer, interface_triple_buffer, flip, snap, keys, tab, layout, cyclic, ime, keyboard, protocol_foreign, protocol_foreign_all_worlds, session_capture, pen);
    let handle = load(state, renderer, ui, rect, IcedSpace::Screen, Layer::SCENE.bits());
    install_handler(state, handle);
    // Restore the shader-picker category, the same way `tab` is restored above.
    let category = state.inner.kernel.get(&SETTINGS).shader_category.clone();
    if category.is_some() {
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            let _ = reg.dispatch_message(handle, SettingsMessage::SelectShaderCategory(category));
        }
    }
    let untyped = handle.untyped();
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() { reg.set_keyboard_focus(Some(untyped)); }
    *state.inner.settings_surface_mut() = Some(untyped);
    state.inner.kernel.get_mut(&SETTINGS_MUT).open = true;
    if let Some(a) = state.inner.kernel.get(&AUDIO) { let _ = a.refresh(); }
    wifi::command(WifiCmd::Scan);
    bt::command(BtCmd::Scan(true));
    LAST.with(|l| *l.borrow_mut() = None);
    LAST_OUTPUTS.with(|l| *l.borrow_mut() = None);
    LAST_TOUCH.with(|l| *l.borrow_mut() = None);
    LAST_SHADERS.with(|l| *l.borrow_mut() = None);
}

fn destroy(state: &mut Loop, id: HandleId) {
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        reg.destroy_by_id(id);
        reg.set_keyboard_focus(None);
    }
    release(state);
}

/// The PROCESS-WIDE half of closing settings: the one audio subscription, the one
/// bluetooth scan, the one provisional-mode confirm, the de-dup caches, and the
/// session-wide `open` flag. None of it is per-world, and all of it is paired
/// against a single live panel.
///
/// Split out because the panel can also be closed by `OverviewSystem::on_disable`
/// when its world goes inactive — a system reaches its own storage but not this.
/// So it destroys the surface and clears the slot, `open` outlives the handle, and
/// the reconciler's `(false, None, true)` arm lands here on the next frame.
fn release(state: &mut Loop) {
    // Closing settings abandons any provisional mode change → revert it (no-op if
    // nothing is pending).
    *state.inner.kernel.get_mut(&OUTPUT_MODE_REQUEST_MUT) = Some(OutputModeRequest::Revert);
    state.inner.ping_control();
    // Drop our audio subscription — unsubscribes until the surface reopens.
    AUDIO_WATCH.with(|w| *w.borrow_mut() = None);
    bt::command(BtCmd::Scan(false));
    *state.inner.settings_surface_mut() = None;
    state.inner.kernel.get_mut(&SETTINGS_MUT).open = false;
}

fn install_handler(state: &mut Loop, handle: IcedHandle<Settings>) {
    let tx = state.inner.surface_mut().surface_message_buffer_channel.0.clone();
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        {
            reg.set_message_handler(handle, move |m: &SettingsMessage| {
                // `StageActive` IS forwarded now — CHECK CHANGES applies an
                // activate/deactivate live-provisionally through the handler (arming the
                // auto-revert gate), so it must reach `interface.handle`. The rest here
                // are UI-local (sync pushes, tab/selection state) and never forwarded.
                if matches!(m, SettingsMessage::SyncSystem(..) | SettingsMessage::SyncDisplays(_) | SettingsMessage::SyncTouchDevices(_) | SettingsMessage::SyncShaders(..) | SettingsMessage::SyncShaderProps(..) | SettingsMessage::SyncShaderPreview(..) | SettingsMessage::SyncShaderStatus(..) | SettingsMessage::Tick | SettingsMessage::WifiSelect(_) | SettingsMessage::WifiPassword(_) | SettingsMessage::SelectDisplay(_) | SettingsMessage::SelectMode(_) | SettingsMessage::SelectInactive) { return; }
                let _ = tx.send(SurfaceMessage { message: SurfaceMessageType::Settings(m.clone()) });
            });
        }
    }
}
