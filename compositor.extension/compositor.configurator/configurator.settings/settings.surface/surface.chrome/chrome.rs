//! Full-width "SYSTEM CONFIGURATION" chrome: title bar, left module sidebar,
//! scrollable section content, and a bottom status/apply bar — built from plain
//! data so the `IcedUi` owner stays tiny. Section bodies live in surface.* builders.
use compositor_developer_environment_config_base::base::Environment;
use compositor_developer_environment_preference_base::base::{Ime, KeyboardLayout};
use compositor_developer_environment_keybinding_base::base::KeyRow;
use compositor_developer_environment_preference_base::base::LayoutPlacement;
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
use compositor_configurator_settings_surface_message::message::{Applied, InputTab, SettingsMessage, ShaderProp, Tab};
use compositor_configurator_settings_surface_style::style;
use compositor_configurator_settings_surface_control::control;
use compositor_configurator_settings_surface_world::world;
use compositor_configurator_settings_surface_graphics::graphics;
use compositor_configurator_settings_surface_pen::pen;
use compositor_developer_environment_graphics_base::base::GraphicsAaConfig;
use compositor_developer_environment_preference_base::base::PenConfig;
use iced_core::{Alignment, Element, Length, Padding, Theme};
use iced_widget::{button, column, container, responsive, row, scrollable, slider, text, toggler, Column, Row};

type El<'a> = Element<'a, SettingsMessage, Theme, Renderer>;

fn fixed(px: f32) -> Length {
    Length::Fixed(px)
}

/// Same sidebar module? Every `Input` sub-tab counts as the one INPUT module so the
/// sidebar row stays lit across the sub-tabs.
fn same_module(a: Tab, b: Tab) -> bool {
    matches!((a, b), (Tab::Input(_), Tab::Input(_))) || a == b
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
        module("◆", "GRAPHICS", Tab::Graphics, sel),
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
    column![text("PERFORMANCE").size(16).color(style::ACCENT), text("Live runtime metrics.").size(11).color(style::MUTED), cell, overlay, release, fractional].spacing(12).into()
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
    tab: Tab, dirty: bool, cursor_sensitivity: f32, natural: bool, touch_pan_speed: f32, touch_linear_pan: bool, osk_size: f32, osk_world_position: bool, show_fps: bool, release_hidden: bool, fractional_invisible: &'a str, env: &'a Environment,
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
            Tab::Graphics => graphics::build(graphics),
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
