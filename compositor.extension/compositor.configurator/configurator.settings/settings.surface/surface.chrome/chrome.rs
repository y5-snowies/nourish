//! Full-width "SYSTEM CONFIGURATION" chrome: title bar, left module sidebar,
//! scrollable section content, and a bottom status/apply bar — built from plain
//! data so the `IcedUi` owner stays tiny. Section bodies live in surface.* builders.
use compositor_developer_environment_config_base::base::Environment;
use compositor_developer_environment_preference_base::base::{Ime, KeyboardLayout};
use compositor_developer_environment_keybinding_base::base::KeyRow;
use compositor_developer_environment_preference_base::base::LayoutPlacement;
use compositor_configurator_hardware_gpu_base::base::RenderDevice;
use compositor_orchestration_driver_output_base::base::{DisplayInfo, ModeInfo};
use compositor_support_iced_core_engine_base::Renderer;
use compositor_y5_audio_controller_interface::interface::AudioState;
use compositor_configurator_network_backend_base::base::WifiSnapshot;
use compositor_configurator_bluetooth_backend_base::base::BtSnapshot;
use compositor_configurator_settings_surface_display::display;
use compositor_configurator_settings_surface_cursor::cursor;
use compositor_configurator_settings_surface_keys::keys as keybinds;
use compositor_configurator_settings_surface_environment::environment;
use compositor_configurator_settings_surface_misc::misc;
use compositor_configurator_audio_tab_base::base as audio_tab;
use compositor_configurator_network_tab_base::base as network_tab;
use compositor_configurator_bluetooth_tab_base::base as bluetooth_tab;
use compositor_configurator_settings_surface_message::message::{Applied, SettingsMessage, ShaderProp, Tab};
use compositor_configurator_settings_surface_style::style;
use compositor_configurator_settings_surface_control::control;
use compositor_configurator_settings_surface_world::world;
use compositor_configurator_settings_surface_graphics::graphics;
use compositor_developer_environment_graphics_base::base::GraphicsAaConfig;
use iced_core::{Alignment, Element, Length, Padding, Theme};
use iced_widget::{button, column, container, row, scrollable, text, toggler};

type El<'a> = Element<'a, SettingsMessage, Theme, Renderer>;

fn fixed(px: f32) -> Length {
    Length::Fixed(px)
}

fn module<'a>(icon: &'a str, label: &'a str, t: Tab, sel: Tab) -> El<'a> {
    button(row![text(icon), text(label).size(13)].spacing(10).align_y(Alignment::Center))
        .width(Length::Fill).padding(Padding::from([8, 14]))
        .on_press(SettingsMessage::Tab(t)).style(control::sidebar_item(sel == t)).into()
}

fn sidebar<'a>(sel: Tab) -> El<'a> {
    let list = column![
        text("CONFIG MODULES").size(10).color(style::MUTED),
        module("◑", "CURRENT WORLD", Tab::World, sel),
        module("▦", "DISPLAY", Tab::Display, sel),
        module("♪", "AUDIO", Tab::Audio, sel),
        module("⌨", "INPUT", Tab::Input, sel),
        module("≋", "NETWORK", Tab::Network, sel),
        module("❖", "BLUETOOTH", Tab::Bluetooth, sel),
        module("▲", "PERFORMANCE", Tab::Performance, sel),
        module("⚙", "SYSTEM", Tab::System, sel),
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

fn performance<'a>(fps: u32, show_fps: bool, release_hidden: bool) -> El<'a> {
    let cell = container(row![text("FRAME RATE").color(style::MUTED).width(Length::Fill), text(format!("{fps} FPS")).color(style::ACCENT)].align_y(Alignment::Center).padding(16))
        .style(style::card).width(Length::Fill);
    let overlay = container(row![text("FPS OVERLAY (per monitor)").color(style::MUTED).width(Length::Fill), toggler(show_fps).on_toggle(SettingsMessage::SetShowFps).style(control::toggler)].align_y(Alignment::Center).padding(16))
        .style(style::card).width(Length::Fill);
    let release = container(row![text("RELEASE HIDDEN SURFACE MEMORY").color(style::MUTED).width(Length::Fill), toggler(release_hidden).on_toggle(SettingsMessage::SetReleaseHidden).style(control::toggler)].align_y(Alignment::Center).padding(16))
        .style(style::card).width(Length::Fill);
    column![text("PERFORMANCE").size(16).color(style::ACCENT), text("Live runtime metrics.").size(11).color(style::MUTED), cell, overlay, release].spacing(12).into()
}

#[allow(clippy::too_many_arguments)]
pub fn render<'a>(
    tab: Tab, dirty: bool, cursor_sensitivity: f32, natural: bool, show_fps: bool, release_hidden: bool, env: &'a Environment,
    displays: &'a [DisplayInfo], active_edid: &'a str, selected_display: &'a str,
    selected_mode: Option<ModeInfo>, pending: Option<&'a Applied>,
    staged_active: Option<&'a (String, Option<ModeInfo>)>, confirming: bool,
    keys: &'a [KeyRow], audio: &'a AudioState, wifi: &'a WifiSnapshot, bt: &'a BtSnapshot,
    wifi_selected: Option<&'a str>, wifi_password: &'a str, devices: &'a [RenderDevice], fps: u32,
    layout: &'a [LayoutPlacement], selected_placement: Option<u64>, cyclic: bool, selected_inactive: bool,
    ime: &'a Ime, keyboard: &'a KeyboardLayout,
    shaders: &'a [String], shader_current: Option<&'a str>, shader_props: &'a [ShaderProp],
    preview_source: &'a str, shader_status: Option<&'a str>,
    invert_pan_x: bool, invert_pan_y: bool, srgb: bool,
    graphics: &'a GraphicsAaConfig,
) -> El<'a> {
    let body: El<'a> = match tab {
        Tab::Display => display::build(displays, active_edid, selected_display, selected_mode, confirming, pending, staged_active, layout, selected_placement, cyclic, selected_inactive),
        Tab::Audio => audio_tab::build(audio),
        Tab::Input => row![
            container(cursor::build(cursor_sensitivity, natural)).width(Length::FillPortion(5)).height(Length::Fill),
            container(keybinds::build(keys)).width(Length::FillPortion(4)).height(Length::Fill),
        ].spacing(24).height(Length::Fill).into(),
        Tab::Network => network_tab::build(wifi, wifi_selected, wifi_password),
        Tab::Bluetooth => bluetooth_tab::build(bt),
        Tab::Performance => performance(fps, show_fps, release_hidden),
        Tab::System => environment::build(env, devices),
        Tab::Misc => misc::build(ime, keyboard),
        Tab::World => world::build(shaders, shader_current, shader_props, preview_source, shader_status, invert_pan_x, invert_pan_y, srgb),
        Tab::Graphics => graphics::build(graphics),
    };
    // Each section still scrolls its own lists vertically. The content area holds a
    // MINIMUM width (`MIN_CONTENT`) so panes never squish/overflow on a narrow window;
    // a horizontal scrollbar appears when the window is narrower than that floor.
    // (iced has no `min_width`, so a fixed floor + horizontal scroll is the mechanism.)
    const MIN_CONTENT: f32 = 620.0;
    let content = column![body].spacing(16).height(Length::Fill);
    let pane = container(content).width(Length::Fixed(MIN_CONTENT)).height(Length::Fill).padding(24);
    let scroller = scrollable(pane)
        .direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::default()))
        .width(Length::Fill)
        .height(Length::Fill);
    let main = row![sidebar(tab), scroller].height(Length::Fill);
    container(column![titlebar(dirty), main]).width(Length::Fill).height(Length::Fill).style(style::backdrop).into()
}
