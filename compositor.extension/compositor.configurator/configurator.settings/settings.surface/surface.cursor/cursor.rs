//! Input controls (left column of the INPUT module): live pointer speed +
//! touchpad natural-scroll. Both apply immediately and have a restore-to-default.
use compositor_support_iced_core_engine_base::Renderer;
use compositor_configurator_settings_surface_message::message::SettingsMessage;
use compositor_configurator_settings_surface_style::style;
use compositor_configurator_settings_surface_control::control;
use iced_core::{Alignment, Element, Length, Theme};
use iced_widget::{button, column, container, row, slider, text, toggler};

type El<'a> = Element<'a, SettingsMessage, Theme, Renderer>;

/// A small "restore to default" (↺) button.
fn reset<'a>(msg: SettingsMessage) -> El<'a> {
    button(text("↺").size(12)).on_press(msg).style(control::action).into()
}

pub fn build<'a>(sensitivity: f32, natural: bool) -> El<'a> {
    let head = column![
        text("MOUSE & TOUCHPAD").size(16).color(style::ACCENT),
        text("Pointer speed and touchpad scrolling.").size(11).color(style::MUTED),
    ].spacing(4);
    let speed = column![
        row![
            text("POINTER SPEED").size(12).color(style::MUTED).width(Length::Fill),
            text(format!("{sensitivity:.2}×")).size(12).color(style::ACCENT),
            reset(SettingsMessage::Cursor(1.0)),
        ].spacing(10).align_y(Alignment::Center),
        slider(0.2..=3.0, sensitivity, SettingsMessage::Cursor).step(0.05f32).style(control::slider),
    ].spacing(8);
    let natural_row = container(
        row![
            text("Natural scroll (touchpad)").width(Length::Fill),
            toggler(natural).on_toggle(SettingsMessage::NaturalScroll).style(control::toggler),
            reset(SettingsMessage::NaturalScroll(true)),
        ].align_y(Alignment::Center).spacing(10).padding(12),
    ).style(style::card).width(Length::Fill);
    column![head, speed, natural_row].spacing(16).into()
}
