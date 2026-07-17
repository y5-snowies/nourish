//! The Misc module: keyboard layout (xkb, live — see surface.keyboard) followed by
//! the input method the compositor launches (preferences.json `ime`, applied on the
//! next start). Each IME edit emits the whole `Ime`, persisted live by the handler.
use compositor_developer_environment_preference_base::base::{Ime, KeyboardLayout};
use compositor_support_iced_core_engine_base::Renderer;
use compositor_configurator_settings_surface_message::message::SettingsMessage;
use compositor_configurator_settings_surface_style::style;
use compositor_configurator_settings_surface_control::control;
use compositor_configurator_settings_surface_keyboard::keyboard;
use iced_core::{Alignment, Element, Length, Theme};
use iced_widget::{button, column, container, row, scrollable, text, text_input, Column};

type El<'a> = Element<'a, SettingsMessage, Theme, Renderer>;

fn card<'a>(inner: El<'a>) -> El<'a> {
    container(inner).style(style::card).width(Length::Fill).into()
}

pub fn build<'a>(ime: &'a Ime, kbd: &'a KeyboardLayout, protocol_foreign: &'a str, protocol_foreign_all_worlds: bool) -> El<'a> {
    let head = column![
        text("MISC").size(16).color(style::ACCENT),
        text("Keyboard layout and the input method launched by the compositor.").size(11).color(style::MUTED),
    ].spacing(4);

    let mut rows: Vec<El<'a>> = vec![head.into()];
    rows.extend(keyboard::rows(kbd));

    // Input-method section (applied on next start).
    rows.push(text("INPUT METHOD").size(14).color(style::ACCENT).into());
    rows.push(text("Launched by the compositor — applied on next start. Empty = none.").size(11).color(style::MUTED).into());

    // Executable (empty = no input method). The field owns a clone of the whole `Ime`
    // and re-emits it with `exec` replaced.
    let base = ime.clone();
    let exec_field = text_input("e.g. fcitx5 (empty = no input method)", &ime.exec)
        .width(Length::Fixed(280.0))
        .on_input(move |s| { let mut x = base.clone(); x.exec = s; SettingsMessage::Ime(x) });
    rows.push(card(
        row![text("Input method exec").width(Length::Fill), exec_field]
            .align_y(Alignment::Center).spacing(10).padding(12).into(),
    ));

    // Arguments: one editable row per arg with a remove (−), then a trailing add (+).
    // Button messages are computed at view time, so each carries the already-mutated `Ime`.
    rows.push(text("ARGUMENTS").size(10).color(style::MUTED).into());
    for (idx, arg) in ime.args.iter().enumerate() {
        let base = ime.clone();
        let edit = text_input("argument", arg)
            .width(Length::Fill)
            .on_input(move |s| { let mut x = base.clone(); x.args[idx] = s; SettingsMessage::Ime(x) });
        let remove = button(text("−").size(14)).style(control::action)
            .on_press({ let mut x = ime.clone(); x.args.remove(idx); SettingsMessage::Ime(x) });
        rows.push(card(
            row![edit, remove].align_y(Alignment::Center).spacing(10).padding(12).into(),
        ));
    }
    let add = button(text("+ add argument").size(12)).style(control::action)
        .on_press({ let mut x = ime.clone(); x.args.push(String::new()); SettingsMessage::Ime(x) });
    rows.push(add.into());

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
