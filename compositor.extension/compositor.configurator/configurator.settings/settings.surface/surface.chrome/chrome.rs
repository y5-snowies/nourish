//! Full-width "SYSTEM CONFIGURATION" chrome: title bar, left module sidebar,
//! scrollable section content, and a bottom status/apply bar — built from plain
//! data so the `IcedUi` owner stays tiny. Section bodies live in surface.* builders.
use compositor_model_environment_config_base::base::Environment;
use compositor_model_environment_preference_base::base::{Ime, KeyboardLayout};
use compositor_model_environment_keybinding_base::base::KeyRow;
use compositor_model_environment_preference_base::base::LayoutPlacement;
use compositor_model_environment_tearing_config::config::{Config, Pacing, Tagging, Tearing};
use compositor_model_environment_tearing_rate::rate::{PaceMode, Rate, TearMode};
use compositor_model_environment_tearing_select::select::{Exclusivity, Selector};
use compositor_configurator_hardware_gpu_base::base::RenderDevice;
use compositor_orchestration_driver_output_base::base::{DisplayInfo, ModeInfo, TouchDeviceInfo};
use compositor_support_iced_core_engine_base::Renderer;
use compositor_y5_audio_controller_interface::interface::AudioState;
use compositor_configurator_network_backend_base::base::WifiSnapshot;
use compositor_configurator_bluetooth_backend_base::base::BtSnapshot;
use compositor_configurator_settings_surface_display::display;
use compositor_configurator_settings_surface_cursor::cursor;
use compositor_configurator_settings_surface_keys::keys as keybinds;
use compositor_configurator_settings_surface_environment::environment;
use compositor_configurator_settings_surface_misc::misc;
use compositor_configurator_settings_surface_language::language;
use compositor_configurator_audio_tab_base::base as audio_tab;
use compositor_configurator_network_tab_base::base as network_tab;
use compositor_configurator_bluetooth_tab_base::base as bluetooth_tab;
use compositor_configurator_settings_surface_message::message::{Applied, GraphicsTab, InputTab, SettingsMessage, ShaderProp, Tab};
use compositor_configurator_settings_surface_style::style;
use compositor_configurator_settings_surface_control::control;
use compositor_configurator_settings_surface_world::world;
use compositor_configurator_settings_surface_graphics::graphics;
use compositor_configurator_settings_surface_pen::pen;
use compositor_model_environment_graphics_base::base::GraphicsAaConfig;
use compositor_model_environment_preference_base::base::PenConfig;
use iced_core::{Alignment, Element, Length, Padding, Theme};
use iced_widget::{button, column, container, responsive, row, scrollable, slider, text, toggler, Column, Row};

type El<'a> = Element<'a, SettingsMessage, Theme, Renderer>;

fn fixed(px: f32) -> Length {
    Length::Fixed(px)
}

/// Same sidebar module? Every sub-tab of a tabbed module counts as that one
/// module, so the sidebar row stays lit as you move between its sub-tabs.
fn same_module(a: Tab, b: Tab) -> bool {
    matches!(
        (a, b),
        (Tab::Input(_), Tab::Input(_)) | (Tab::Graphics(_), Tab::Graphics(_))
    ) || a == b
}

fn module<'a>(icon: &'a str, label: &'a str, t: Tab, sel: Tab) -> El<'a> {
    button(row![text(icon), text(label).size(13)].spacing(10).align_y(Alignment::Center))
        .width(Length::Fill).padding(Padding::from([8, 14]))
        .on_press(SettingsMessage::Tab(t)).style(control::sidebar_item(same_module(sel, t))).into()
}

fn sidebar<'a>(sel: Tab) -> El<'a> {
    let list = column![
        text("CONFIG MODULES").size(10).color(style::MUTED),
        module("◑", "CURRENT WORLD", Tab::World, sel),
        module("▦", "DISPLAY", Tab::Display, sel),
        module("♪", "AUDIO", Tab::Audio, sel),
        module("⌨", "INPUT", Tab::Input(InputTab::Mouse), sel),
        module("≋", "NETWORK", Tab::Network, sel),
        module("❖", "BLUETOOTH", Tab::Bluetooth, sel),
        module("▲", "PERFORMANCE", Tab::Performance, sel),
        module("⚙", "SYSTEM", Tab::System, sel),
        module("文", "LANGUAGE", Tab::Language, sel),
        module("◆", "GRAPHICS", Tab::Graphics(GraphicsTab::Aa), sel),
        module("⋯", "MISC", Tab::Misc, sel),
    ].spacing(4).padding(14);
    container(list).width(fixed(224.0)).height(Length::Fill).style(style::sidebar).into()
}

fn titlebar<'a>(dirty: bool) -> El<'a> {
    let sub = if dirty { "y5 COMPOSITOR · REBOOT TO APPLY SOME CHANGES" } else { "y5 COMPOSITOR · RUNTIME CONFIG" };
    let title = column![
        text("SYSTEM CONFIGURATION").size(20),
        text(sub).size(10).color(style::MUTED),
    ].spacing(3);
    container(title).style(style::strip).width(Length::Fill).padding(Padding::from([14, 22])).into()
}

fn performance<'a>(fps: u32, show_fps: bool, release_hidden: bool, fractional_invisible: &'a str) -> El<'a> {
    let cell = container(row![text("FRAME RATE").color(style::MUTED).width(Length::Fill), text(format!("{fps} FPS")).color(style::ACCENT)].align_y(Alignment::Center).padding(16))
        .style(style::card).width(Length::Fill);
    let overlay = container(row![text("FPS OVERLAY (per monitor)").color(style::MUTED).width(Length::Fill), toggler(show_fps).on_toggle(SettingsMessage::SetShowFps).style(control::toggler)].align_y(Alignment::Center).padding(16))
        .style(style::card).width(Length::Fill);
    let release = container(row![text("RELEASE HIDDEN SURFACE MEMORY").color(style::MUTED).width(Length::Fill), toggler(release_hidden).on_toggle(SettingsMessage::SetReleaseHidden).style(control::toggler)].align_y(Alignment::Center).padding(16))
        .style(style::card).width(Length::Fill);
    // Invisible-window fractional scale: off (always update, historical) /
    // optimized (freeze invisible; other worlds render at scale 1) / full
    // (all invisible windows render at scale 1 to free client memory).
    let mk = |label: &'a str, value: &'a str| {
        let b = button(text(label).size(12)).on_press(SettingsMessage::SetFractionalInvisible(value.to_string()));
        if fractional_invisible == value { b.style(control::accent) } else { b.style(control::action) }
    };
    let fractional = container(row![
        text("INVISIBLE WINDOW SCALE").color(style::MUTED).width(Length::Fill),
        mk("Off", "off"),
        mk("Optimized", "optimized"),
        mk("Full", "full"),
    ].align_y(Alignment::Center).spacing(10).padding(16))
        .style(style::card).width(Length::Fill);

    column![
        text("PERFORMANCE").size(16).color(style::ACCENT),
        text("Live runtime metrics.").size(11).color(style::MUTED),
        cell, overlay, release, fractional,
    ].spacing(12).into()
}

/// The GRAPHICS module: a sub-tab bar over the selected sub-tab's body. The
/// sub-tab is carried in `Tab::Graphics`, so it persists across launches.
fn graphics_body<'a>(sub: GraphicsTab, graphics: &'a GraphicsAaConfig, flip: Config) -> El<'a> {
    let tab = |label: &'a str, t: GraphicsTab| -> El<'a> {
        button(text(label).size(13))
            .padding(Padding::from([6, 14]))
            .on_press(SettingsMessage::Tab(Tab::Graphics(t)))
            .style(control::tab(sub == t))
            .into()
    };
    let bar = row![
        tab("Anti-aliasing", GraphicsTab::Aa),
        tab("FSR", GraphicsTab::Fsr),
        tab("Pacing", GraphicsTab::Pacing),
    ].spacing(6);
    let body: El<'a> = match sub {
        GraphicsTab::Aa => graphics::build_aa(graphics),
        GraphicsTab::Fsr => graphics::build_fsr(graphics),
        GraphicsTab::Pacing => scrollable(flip_section(flip)).height(Length::Fill).into(),
    };
    column![bar, body].spacing(16).height(Length::Fill).into()
}

/// One settings row: a label, a set of choices, and a balloon describing the
/// choice that is currently selected. The balloon is the whole point — several
/// of these options tear, and a flip policy that tears without saying so is the
/// failure mode worth designing against here.
fn field<'a>(label: &'a str, choices: Vec<El<'a>>, balloon: &'a str) -> El<'a> {
    let mut r = row![text(label).color(style::MUTED).width(Length::Fill)]
        .align_y(Alignment::Center).spacing(8);
    for c in choices { r = r.push(c); }
    container(column![r, text(balloon).size(10).color(style::MUTED)].spacing(6).padding(16))
        .style(style::card).width(Length::Fill).into()
}

fn pick<'a, T: PartialEq + Copy + 'a>(
    all: &'a [T], current: T, label: fn(T) -> &'static str, msg: impl Fn(T) -> SettingsMessage,
) -> Vec<El<'a>> {
    all.iter().map(|&v| {
        let b = button(text(label(v)).size(11)).on_press(msg(v));
        if v == current { b.style(control::accent) } else { b.style(control::action) }
    }.into()).collect()
}

/// The tearing policy and its pacing fallback. Tearing outranks pacing and the
/// two are mutually exclusive in TIME: at any instant one section governs, or
/// neither and the compositor default applies.
fn flip_section<'a>(f: Config) -> El<'a> {
    use compositor_configurator_settings_surface_tearhint::tearhint as d;
    use compositor_configurator_settings_surface_tearlabel::tearlabel as t;

    let t_sel = pick(&Selector::ALL, f.tearing.selector, t::selector_label,
        move |v| SettingsMessage::SetFlip(Config { tearing: Tearing { selector: v, ..f.tearing }, ..f }));
    let t_mode = pick(&TearMode::ALL, f.tearing.mode, t::tear_mode_label,
        move |v| SettingsMessage::SetFlip(Config { tearing: Tearing { mode: v, ..f.tearing }, ..f }));
    let t_excl = pick(&Exclusivity::ALL, f.tearing.exclusivity, t::exclusivity_label,
        move |v| SettingsMessage::SetFlip(Config { tearing: Tearing { exclusivity: v, ..f.tearing }, ..f }));

    let p_sel = pick(&Selector::ALL, f.pacing.selector, t::selector_label,
        move |v| SettingsMessage::SetFlip(Config { pacing: Pacing { selector: v, ..f.pacing }, ..f }));
    let p_mode = pick(&PaceMode::ALL, f.pacing.mode, t::pace_mode_label,
        move |v| SettingsMessage::SetFlip(Config { pacing: Pacing { mode: v, ..f.pacing }, ..f }));
    let p_excl = pick(&Exclusivity::ALL, f.pacing.exclusivity, t::exclusivity_label,
        move |v| SettingsMessage::SetFlip(Config { pacing: Pacing { exclusivity: v, ..f.pacing }, ..f }));

    column![
        text("GRAPHICS — PACING").size(16).color(style::ACCENT),
        text("Which windows the page flip follows, and whether it waits for the scan-out.")
            .size(11).color(style::MUTED),
        tag_section(f),
        text("PAGE FLIP — TEARING").size(14).color(style::ACCENT),
        field("WHEN", t_sel, d::selector_describe(f.tearing.selector)),
        field("MODE", t_mode, d::tear_mode_describe(f.tearing.mode)),
        rate_field(f, true),
        field("EXCLUSIVITY", t_excl, d::exclusivity_describe(f.tearing.exclusivity)),
        text("PAGE FLIP — PACING (fallback)").size(14).color(style::ACCENT),
        text("Applies only while the tearing section above is not in force.")
            .size(10).color(style::MUTED),
        field("WHEN", p_sel, d::selector_describe(f.pacing.selector)),
        field("MODE", p_mode, d::pace_mode_describe(f.pacing.mode)),
        rate_field(f, false),
        field("EXCLUSIVITY", p_excl, d::exclusivity_describe(f.pacing.exclusivity)),
        text("EXCLUSIVITY FLOOR").size(14).color(style::ACCENT),
        text("Shared by both sections: whichever one is in force, this is how slow \
              the rest of the desktop may get while a target owns the cadence.")
            .size(10).color(style::MUTED),
        floor_field(f),
        reset_row(f),
    ].spacing(10).into()
}

/// Reset every control on this page at once.
///
/// The whole page is a single `Config`, so this is literally `Config::default()`
/// — there is no per-field list here to drift out of sync when a field is added
/// to the struct. Accented while anything differs, so it reads as "there is
/// something to undo" rather than as a button that is always live.
fn reset_row<'a>(f: Config) -> El<'a> {
    let b = button(text("Reset to defaults").size(11))
        .on_press(SettingsMessage::SetFlip(Config::default()));
    let b = if f == Config::default() { b.style(control::action) } else { b.style(control::accent) };
    field(
        "DEFAULTS",
        vec![b.into()],
        "Restores tearing, pacing, target tagging and the exclusivity floor to \
         their shipped values. Applies immediately, like every other control here.",
    )
}

/// What counts as a "target" for the WHEN/EXCLUSIVITY rules below. A client can
/// say so itself with `wp_tearing_control_v1`, but almost no game does, so the
/// tag is also inferred from the process — which is a guess, hence the toggle.
///
/// Inference reads `/proc`, far too heavy to repeat per commit, so it resolves
/// once when a window maps. That is why both balloons say "opened after": these
/// switches cannot retag a window that is already on screen.
fn tag_section<'a>(f: Config) -> El<'a> {
    let toggle = toggler(f.tag.steam)
        .on_toggle(move |v| SettingsMessage::SetFlip(Config { tag: Tagging { steam: v }, ..f }))
        .style(control::toggler);
    let steam = container(column![
        row![text("AUTO-TAG STEAM APPS").color(style::MUTED).width(Length::Fill), toggle]
            .align_y(Alignment::Center),
        text("Treats a window as a target when it is a Steam title — a steam_app_* \
              identity, Steam's launch environment, or an executable inside a Steam \
              library. Steam's own windows are excluded. Applies to windows opened \
              after the change.")
            .size(10).color(style::MUTED),
    ].spacing(6).padding(16)).style(style::card).width(Length::Fill);
    let env = container(column![
        text("Y5_TEARING=1").color(style::MUTED),
        text("Always honoured, with or without the toggle above: any process started \
              with this in its environment is a target, and so is anything it launches. \
              Set it in a Steam launch command to tag one game, then reopen it. It \
              cannot reach a title running under X11 — those all share the xwayland \
              proxy's environment.")
            .size(10).color(style::MUTED),
    ].spacing(6).padding(16)).style(style::card).width(Length::Fill);
    column![
        text("TARGET TAGGING").size(14).color(style::ACCENT),
        steam,
        env,
    ].spacing(10).into()
}

/// Rate: pick the KIND (uncapped / multiple of refresh / absolute FPS), then step
/// the value. Kept as one control because the three are alternatives, not an
/// enable plus a number.
fn rate_field<'a>(f: Config, tearing_section: bool) -> El<'a> {
    use compositor_configurator_settings_surface_tearhint::tearhint as d;
    use compositor_configurator_settings_surface_tearlabel::tearlabel as t;
    let cur = if tearing_section { f.tearing.rate } else { f.pacing.rate };
    let put = move |r: Rate| {
        if tearing_section {
            SettingsMessage::SetFlip(Config { tearing: Tearing { rate: r, ..f.tearing }, ..f })
        } else {
            SettingsMessage::SetFlip(Config { pacing: Pacing { rate: r, ..f.pacing }, ..f })
        }
    };
    let kind = |lbl: &'a str, r: Rate, on: bool| -> El<'a> {
        let b = button(text(lbl).size(11)).on_press(put(r));
        if on { b.style(control::accent) } else { b.style(control::action) }.into()
    };
    let mut choices: Vec<El<'a>> = vec![
        kind("Uncapped", Rate::Uncapped, matches!(cur, Rate::Uncapped)),
        kind("x refresh", Rate::Multiplier(2.0), matches!(cur, Rate::Multiplier(_))),
        kind("FPS", Rate::Fps(60.0), matches!(cur, Rate::Fps(_))),
    ];
    let step = |delta: f32| match cur {
        Rate::Multiplier(m) => Rate::Multiplier(m + delta * 0.5),
        Rate::Fps(v) => Rate::Fps(v + delta * 10.0),
        Rate::Uncapped => Rate::Uncapped,
    };
    if !matches!(cur, Rate::Uncapped) {
        choices.push(button(text("-").size(11)).on_press(put(step(-1.0))).style(control::action).into());
        choices.push(text(t::rate_label(cur)).size(11).color(style::ACCENT).into());
        choices.push(button(text("+").size(11)).on_press(put(step(1.0))).style(control::action).into());
    }
    field("RATE", choices, d::rate_describe(cur))
}

/// The watchdog floor. Same control shape as [`rate_field`] because it is the
/// same kind of quantity — a rate against this monitor's refresh — but it bounds
/// the loop from BELOW: how slow the desktop may get while a target owns the
/// cadence. One setting for both sections; there is one redraw loop to rescue.
///
/// No "Off" row, unlike [`rate_field`]: the rescue frames are what carry the
/// cursor, the compositor's UI and the admitted client's frame callbacks while a
/// gate is engaged, so turning them off is a hang rather than a setting. The
/// stepper clamps at `FLOOR_MIN_FPS` and the model normalizes anything slower.
fn floor_field<'a>(f: Config) -> El<'a> {
    use compositor_configurator_settings_surface_tearhint::tearhint as d;
    use compositor_configurator_settings_surface_tearlabel::tearlabel as t;
    use compositor_model_environment_tearing_config::config::{FLOOR_DEFAULT, FLOOR_MIN_FPS};
    let cur = f.floor;
    let put = move |r: Rate| SettingsMessage::SetFlip(Config { floor: r, ..f });
    let kind = |lbl: &'a str, r: Rate, on: bool| -> El<'a> {
        let b = button(text(lbl).size(11)).on_press(put(r));
        if on { b.style(control::accent) } else { b.style(control::action) }.into()
    };
    let mut choices: Vec<El<'a>> = vec![
        kind("x refresh", Rate::Multiplier(1.0), matches!(cur, Rate::Multiplier(_))),
        kind("FPS", Rate::Fps(30.0), matches!(cur, Rate::Fps(_))),
    ];
    // A `Multiplier` floor is clamped against the live mode, not here, so the
    // stepper only guards the absolute form.
    let step = |delta: f32| match cur {
        Rate::Multiplier(m) => Rate::Multiplier((m + delta * 0.25).max(0.25)),
        Rate::Fps(v) => Rate::Fps((v + delta * 5.0).max(FLOOR_MIN_FPS)),
        Rate::Uncapped => FLOOR_DEFAULT,
    };
    choices.push(button(text("-").size(11)).on_press(put(step(-1.0))).style(control::action).into());
    choices.push(text(t::rate_label(cur)).size(11).color(style::ACCENT).into());
    choices.push(button(text("+").size(11)).on_press(put(step(1.0))).style(control::action).into());
    field("FLOOR", choices, d::floor_describe(cur))
}

/// The INPUT module: a sub-tab bar (Mouse & Touchpad / Touch / Keyboard) over the
/// selected sub-tab's body. The sub-tab is carried in `Tab::Input`, so it persists.
fn input_body<'a>(
    sub: InputTab, cursor_sensitivity: f32, natural: bool, touch_pan_speed: f32, touch_linear_pan: bool,
    osk_size: f32, osk_world_position: bool,
    keys: &'a [KeyRow], touch_devices: &'a [TouchDeviceInfo], displays: &'a [DisplayInfo], pen: &'a PenConfig,
    pen_capturing: bool,
) -> El<'a> {
    let tab = |label: &'a str, t: InputTab| -> El<'a> {
        button(text(label).size(13))
            .padding(Padding::from([6, 14]))
            .on_press(SettingsMessage::Tab(Tab::Input(t)))
            .style(control::tab(sub == t))
            .into()
    };
    let bar = row![
        tab("Mouse & Touchpad", InputTab::Mouse),
        tab("Touch", InputTab::Touch),
        tab("Pen", InputTab::Pen),
        tab("Keyboard", InputTab::Keyboard),
    ].spacing(6);
    let body: El<'a> = match sub {
        InputTab::Mouse => cursor::build(cursor_sensitivity, natural),
        InputTab::Touch => touch_input(touch_pan_speed, touch_linear_pan, osk_size, osk_world_position, touch_devices, displays),
        InputTab::Pen => pen::build(pen, pen_capturing),
        InputTab::Keyboard => keybinds::build(keys),
    };
    column![bar, body].spacing(16).height(Length::Fill).into()
}

/// The Touch sub-tab: pan-speed slider, linear-pan toggle, and the touch↔display
/// link (relocated here from the Display tab). Each touch device gets a row of
/// per-monitor buttons — the lit one is its current claim; pressing it releases
/// back to auto, pressing another moves the claim to that monitor.
fn touch_input<'a>(
    pan_speed: f32, linear_pan: bool, osk_size: f32, osk_world_position: bool,
    touch_devices: &'a [TouchDeviceInfo], displays: &'a [DisplayInfo],
) -> El<'a> {
    let reset = |msg: SettingsMessage| -> El<'a> {
        button(text("↺").size(12)).on_press(msg).style(control::action).into()
    };
    let head = column![
        text("TOUCH").size(16).color(style::ACCENT),
        text("Touchscreen pan feel and the touch↔display link.").size(11).color(style::MUTED),
    ].spacing(4);
    // Discoverability hint for the touch menu (the sticky tool-mode pane).
    let hint = container(
        row![
            text("ℹ").size(16).color(style::ACCENT),
            text("Swipe in from the left edge with two fingers to open the touch menu.")
                .size(12).color(style::MUTED).width(Length::Fill),
        ].align_y(Alignment::Center).spacing(10).padding(12),
    ).style(style::card).width(Length::Fill);
    let speed = column![
        row![
            text("PAN SPEED").size(12).color(style::MUTED).width(Length::Fill),
            text(format!("{pan_speed:.2}×")).size(12).color(style::ACCENT),
            reset(SettingsMessage::TouchPanSpeed(1.0)),
        ].spacing(10).align_y(Alignment::Center),
        slider(0.1..=4.0, pan_speed, SettingsMessage::TouchPanSpeed).step(0.05f32).style(control::slider),
    ].spacing(8);
    let linear = container(
        row![
            text("Linear pan (1:1, no glide)").width(Length::Fill),
            toggler(linear_pan).on_toggle(SettingsMessage::TouchLinearPan).style(control::toggler),
            reset(SettingsMessage::TouchLinearPan(true)),
        ].align_y(Alignment::Center).spacing(10).padding(12),
    ).style(style::card).width(Length::Fill);

    let mut link: Vec<El<'a>> = vec![
        text("TOUCH ↔ DISPLAY").size(11).color(style::MUTED).into(),
        text("Bind a touchscreen to the monitor it physically overlays.").size(11).color(style::MUTED).into(),
    ];
    if touch_devices.is_empty() {
        link.push(text("No touch devices connected.").size(12).color(style::MUTED).into());
    } else if displays.is_empty() {
        link.push(text("No monitors detected.").size(12).color(style::MUTED).into());
    } else {
        for d in touch_devices {
            let mut cells: Vec<El<'a>> = vec![text(d.name.clone()).size(12).width(Length::Fill).into()];
            for disp in displays {
                let here = d.assigned_edid.as_deref() == Some(disp.edid_key.as_str());
                let msg = if here {
                    SettingsMessage::ClaimTouch(disp.edid_key.clone(), None)
                } else {
                    SettingsMessage::ClaimTouch(disp.edid_key.clone(), Some(d.id.clone()))
                };
                let b = button(text(disp.name.clone()).size(12)).on_press(msg);
                cells.push(if here { b.style(control::accent) } else { b.style(control::action) }.into());
            }
            link.push(
                container(Row::with_children(cells).spacing(8).align_y(Alignment::Center).padding(10))
                    .style(style::card).width(Length::Fill).into(),
            );
        }
    }
    let osk = column![
        row![
            text("OSK SIZE").size(12).color(style::MUTED).width(Length::Fill),
            text(format!("{osk_size:.2}×")).size(12).color(style::ACCENT),
            reset(SettingsMessage::OskSize(1.0)),
        ].spacing(10).align_y(Alignment::Center),
        slider(0.6..=1.4, osk_size, SettingsMessage::OskSize).step(0.05f32).style(control::slider),
    ].spacing(8);
    let osk_world = container(
        row![
            text("On-screen keyboard on world position").width(Length::Fill),
            toggler(osk_world_position).on_toggle(SettingsMessage::OskWorldPosition).style(control::toggler),
            reset(SettingsMessage::OskWorldPosition(false)),
        ].align_y(Alignment::Center).spacing(10).padding(12),
    ).style(style::card).width(Length::Fill);

    column![head, hint, speed, linear, osk, osk_world, Column::with_children(link).spacing(8)].spacing(16).into()
}

#[allow(clippy::too_many_arguments)]
pub fn render<'a>(
    tab: Tab, dirty: bool, cursor_sensitivity: f32, natural: bool, touch_pan_speed: f32, touch_linear_pan: bool, osk_size: f32, osk_world_position: bool, show_fps: bool, release_hidden: bool, fractional_invisible: &'a str, flip: Config, env: &'a Environment,
    displays: &'a [DisplayInfo], touch_devices: &'a [TouchDeviceInfo], active_edid: &'a str, selected_display: &'a str,
    selected_mode: Option<ModeInfo>, pending: Option<&'a Applied>,
    staged_active: Option<&'a (String, Option<ModeInfo>)>, confirming: bool,
    keys: &'a [KeyRow], audio: &'a AudioState, wifi: &'a WifiSnapshot, bt: &'a BtSnapshot,
    wifi_selected: Option<&'a str>, wifi_password: &'a str, devices: &'a [RenderDevice], fps: u32,
    layout: &'a [LayoutPlacement], selected_placement: Option<u64>, cyclic: bool, selected_inactive: bool,
    ime: &'a Ime, keyboard: &'a KeyboardLayout, catalog: &'a [(String, String)],
    lang_picker_open: bool, lang_search: &'a str,
    protocol_foreign: &'a str, protocol_foreign_all_worlds: bool,
    shaders: &'a [String], shader_current: Option<&'a str>, shader_props: &'a [ShaderProp],
    preview_source: &'a str, shader_status: Option<&'a str>,
    invert_pan_x: bool, invert_pan_y: bool, srgb: bool,
    graphics: &'a GraphicsAaConfig,
    pen: &'a PenConfig,
    pen_capturing: bool,
) -> El<'a> {
    // Each section still scrolls its own lists vertically. The content pane is
    // RESPONSIVE: it fills the available width up to `MAX_CONTENT` — a readability
    // cap, since the label-left/control-right (space-between) rows get absurd gaps
    // on wide monitors — staying anchored to the left (next to the sidebar) when
    // there's more room than that. It never shrinks below `MIN_CONTENT`: iced has
    // no `min_width`, so a fixed floor + horizontal scroll is the narrow-window
    // mechanism.
    const MIN_CONTENT: f32 = 620.0;
    const MAX_CONTENT: f32 = 900.0;
    let content_area: El<'a> = responsive(move |avail| {
        let body: El<'a> = match tab {
            Tab::Display => display::build(displays, touch_devices, active_edid, selected_display, selected_mode, confirming, pending, staged_active, layout, selected_placement, cyclic, selected_inactive),
            Tab::Audio => audio_tab::build(audio),
            Tab::Input(sub) => input_body(sub, cursor_sensitivity, natural, touch_pan_speed, touch_linear_pan, osk_size, osk_world_position, keys, touch_devices, displays, pen, pen_capturing),
            Tab::Network => network_tab::build(wifi, wifi_selected, wifi_password),
            Tab::Bluetooth => bluetooth_tab::build(bt),
            Tab::Performance => performance(fps, show_fps, release_hidden, fractional_invisible),
            Tab::System => environment::build(env, devices),
            Tab::Misc => misc::build(protocol_foreign, protocol_foreign_all_worlds),
            Tab::Language => language::build(keyboard, catalog, lang_picker_open, lang_search, ime),
            Tab::World => world::build(shaders, shader_current, shader_props, preview_source, shader_status, invert_pan_x, invert_pan_y, srgb),
            Tab::Graphics(sub) => graphics_body(sub, graphics, flip),
        };
        let content = column![body].spacing(16).height(Length::Fill);
        let pane_w = avail.width.clamp(MIN_CONTENT, MAX_CONTENT);
        let pane = container(content).width(fixed(pane_w)).height(Length::Fill).padding(24);
        scrollable(pane)
            .direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::default()))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    })
    .into();
    let main = row![sidebar(tab), content_area].height(Length::Fill);
    container(column![titlebar(dirty), main]).width(Length::Fill).height(Length::Fill).style(style::backdrop).into()
}
