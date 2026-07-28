//! The Language tab: keyboard layout (xkb, live — see surface.keyboard) followed by
//! the input method the compositor launches (preferences.json `ime`, applied on the
//! next start). Split out of the former Misc tab; the keyboard section is the
//! multi-layout editor, and each IME edit emits the whole `Ime`, persisted live by
//! the handler.
use compositor_model_environment_preference_base::base::{Ime, KeyboardLayout};
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

pub fn build<'a>(
    kbd: &'a KeyboardLayout,
    catalog: &'a [(String, String)],
    picker_open: bool,
    search: &'a str,
    ime: &'a Ime,
) -> El<'a> {
    let mut rows: Vec<El<'a>> = vec![
        column![
            text("LANGUAGE").size(16).color(style::ACCENT),
            text("Keyboard layouts and the input method launched by the compositor.")
                .size(11)
                .color(style::MUTED),
        ]
        .spacing(4)
        .into(),
    ];

    rows.extend(keyboard::rows(kbd, catalog, picker_open, search));

    // Input-method section (applied on next start). The field/button own a clone of
    // the whole `Ime` and re-emit it with one field replaced.
    rows.push(text("INPUT METHOD").size(14).color(style::ACCENT).into());
    rows.push(text("Launched by the compositor — applied on next start. Empty = none.").size(11).color(style::MUTED).into());

    let base = ime.clone();
    let exec_field = text_input("e.g. fcitx5 (empty = no input method)", &ime.exec)
        .width(Length::Fixed(280.0))
        .on_input(move |s| { let mut x = base.clone(); x.exec = s; SettingsMessage::Ime(x) });
    rows.push(card(
        row![text("Input method exec").width(Length::Fill), exec_field]
            .align_y(Alignment::Center).spacing(10).padding(12).into(),
    ));

    // Arguments: one editable row per arg with a remove (−), then a trailing add (+).
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

    scrollable(Column::with_children(rows).spacing(10)).height(Length::Fill).into()
}
