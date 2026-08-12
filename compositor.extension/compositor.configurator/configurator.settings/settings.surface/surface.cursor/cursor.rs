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

pub fn build<'a>(sensitivity: f32, natural: bool, edge_pan: bool, edge_pan_speed: f32, edge_pan_continuous: bool) -> El<'a> {
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
    // Edge pan owns the screen extents, so it also RE-HOMES cursor teleport onto
    // Super — say so here, since the teleport map itself lives on the Display tab.
    let edge_row = container(
        column![
            row![
                text("Pan canvas at screen edge").width(Length::Fill),
                toggler(edge_pan).on_toggle(SettingsMessage::EdgePan).style(control::toggler),
                reset(SettingsMessage::EdgePan(false)),
            ].align_y(Alignment::Center).spacing(10),
            text(
                "Pushing the cursor past a screen edge pans the canvas — including while moving, \
                 resizing or hand-dragging, but not while box-selecting. With this on, hold Super \
                 (with nothing being dragged) to cross to another monitor instead."
            ).size(11).color(style::MUTED),
            // Speed rides ON TOP of POINTER SPEED above — the edge pan is carried by
            // the pointer's own motion, so this only re-weights it.
            row![
                text("EDGE PAN SPEED").size(12).color(style::MUTED).width(Length::Fill),
                text(format!("{edge_pan_speed:.2}×")).size(12).color(style::ACCENT),
                reset(SettingsMessage::EdgePanSpeed(1.0)),
            ].spacing(10).align_y(Alignment::Center),
            slider(0.25..=4.0, edge_pan_speed, SettingsMessage::EdgePanSpeed)
                .step(0.05f32)
                .style(control::slider),
            text("Multiplies the pointer speed above, for edge panning only.")
                .size(11).color(style::MUTED),
            row![
                text("Keep panning while parked").width(Length::Fill),
                toggler(edge_pan_continuous).on_toggle(SettingsMessage::EdgePanContinuous).style(control::toggler),
                reset(SettingsMessage::EdgePanContinuous(true)),
            ].align_y(Alignment::Center).spacing(10),
            text(
                "On: resting the cursor against an edge keeps the canvas moving, and pushing \
                 into the edge adds speed while you push. Off: it only moves while you push."
            ).size(11).color(style::MUTED),
        ].spacing(6).padding(12),
    ).style(style::card).width(Length::Fill);
    column![head, speed, natural_row, edge_row].spacing(16).into()
}
