//! Audio module: output sinks — pick the default + per-sink volume.
use compositor_y5_audio_controller_interface::interface::AudioState;
use compositor_support_iced_core_engine_base::Renderer;
use compositor_configurator_settings_surface_message::message::SettingsMessage;
use compositor_configurator_settings_surface_style::style;
use compositor_configurator_settings_surface_control::control;
use iced_core::{Alignment, Element, Length, Theme};
use iced_widget::{button, column, container, row, slider, text, Column};

type El<'a> = Element<'a, SettingsMessage, Theme, Renderer>;

pub fn build<'a>(a: &'a AudioState) -> El<'a> {
    let head = column![
        text("AUDIO").size(16).color(style::ACCENT),
        text("Output devices and volume.").size(11).color(style::MUTED),
    ].spacing(4);
    let mut rows: Vec<El<'a>> = vec![head.into()];
    if a.sinks.is_empty() {
        rows.push(text("No audio outputs found.").size(12).color(style::MUTED).into());
    }
    for s in &a.sinks {
        let pick = s.name.clone();
        let vol_name = s.name.clone();
        let reset_name = s.name.clone();
        let mute_name = s.name.clone();
        let label = if s.description.is_empty() { s.name.clone() } else { s.description.clone() };
        let title = row![
            button(text(if s.is_default { "● DEFAULT" } else { "○ SET DEFAULT" }).size(11))
                .on_press(SettingsMessage::SetDefaultSink(pick)).style(control::action),
            text(label).width(Length::Fill),
            text(format!("{}%", (s.volume * 100.0).round() as i32)).size(12).color(style::ACCENT),
            // Toggle mute. Reflects the live `muted` from the sink controller —
            // no optimistic flip; the state updates once the controller reports it.
            button(text(if s.muted { "🔇 MUTED" } else { "🔊 MUTE" }).size(11))
                .on_press(SettingsMessage::SetSinkMute(mute_name, !s.muted)).style(control::action),
            // Restore volume to the 100% default.
            button(text("↺").size(12)).on_press(SettingsMessage::SetSinkVolume(reset_name, 1.0)).style(control::action),
        ].spacing(10).align_y(Alignment::Center);
        let cell = column![
            title,
            slider(0.0..=1.0, s.volume as f32, move |v| SettingsMessage::SetSinkVolume(vol_name.clone(), v)).step(0.02f32).style(control::slider),
        ].spacing(10);
        rows.push(container(cell).padding(14).style(style::card).width(Length::Fill).into());
    }
    Column::with_children(rows).spacing(12).into()
}
