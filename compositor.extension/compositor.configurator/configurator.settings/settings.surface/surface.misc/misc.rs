//! The Misc tab: the foreign-toplevel (dock/taskbar) protocol toggles. Keyboard
//! layout and the input method moved to the Language tab. Each toggle emits a
//! forwarded message the handler persists (and, for all-worlds, applies live).
use compositor_support_iced_core_engine_base::Renderer;
use compositor_configurator_settings_surface_message::message::SettingsMessage;
use compositor_configurator_settings_surface_style::style;
use compositor_configurator_settings_surface_control::control;
use iced_core::{Alignment, Element, Length, Theme};
use iced_widget::{button, column, container, row, scrollable, text, Column};

type El<'a> = Element<'a, SettingsMessage, Theme, Renderer>;

fn card<'a>(inner: El<'a>) -> El<'a> {
    container(inner).style(style::card).width(Length::Fill).into()
}

pub fn build<'a>(protocol_foreign: &'a str, protocol_foreign_all_worlds: bool) -> El<'a> {
    let mut rows: Vec<El<'a>> = vec![
        column![
            text("MISC").size(16).color(style::ACCENT),
            text("Foreign-toplevel (dock/taskbar) protocols.").size(11).color(style::MUTED),
        ]
        .spacing(4)
        .into(),
    ];

    // Foreign-toplevel (dock/taskbar) protocols — wlr + ext, gated as one. Applied
    // on next start; when disabled the globals stay bound but advertise nothing.
    rows.push(text("DOCK PROTOCOLS").size(14).color(style::ACCENT).into());
    rows.push(text("Expose open windows to external docks/taskbars (Waybar, sfwbar) via wlr + ext foreign-toplevel. Applied on next start.").size(11).color(style::MUTED).into());
    let on = protocol_foreign == "enabled";
    let mk = |label: &'a str, value: &'a str, active: bool| {
        let b = button(text(label).size(12)).on_press(SettingsMessage::SetProtocolForeign(value.to_string()));
        if active { b.style(control::accent) } else { b.style(control::action) }
    };
    rows.push(card(
        row![
            text("Foreign-toplevel").width(Length::Fill),
            mk("Enabled", "enabled", on),
            mk("Disabled", "disabled", !on),
        ]
        .align_y(Alignment::Center).spacing(10).padding(12).into(),
    ));

    // Advertise windows from ALL worlds vs just the active one. Only meaningful when the
    // foreign protocols are enabled; applied live (re-advertises immediately, no reboot).
    let all = protocol_foreign_all_worlds;
    let mkb = |label: &'a str, value: bool, active: bool| {
        let b = button(text(label).size(12))
            .on_press(SettingsMessage::SetProtocolForeignAllWorlds(value));
        if active { b.style(control::accent) } else { b.style(control::action) }
    };
    rows.push(card(
        row![
            text("Show windows from all worlds").width(Length::Fill),
            mkb("On", true, all),
            mkb("Off", false, !all),
        ]
        .align_y(Alignment::Center).spacing(10).padding(12).into(),
    ));

    scrollable(Column::with_children(rows).spacing(10)).height(Length::Fill).into()
}
