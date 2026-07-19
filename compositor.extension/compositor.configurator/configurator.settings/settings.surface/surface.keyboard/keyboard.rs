//! KEYBOARD LAYOUT section of the Language tab. An ordered list of xkb layouts with
//! reorder (↑/↓) + remove (−), a searchable picker to ADD a layout from the system
//! catalogue, and a preset switch-hotkey dropdown. Per-layout variants and arbitrary
//! xkb options are NOT editable here — use the environment (`XKB_DEFAULT_*`, the Env
//! source) for those. Every edit emits the whole `KeyboardLayout`
//! (`SettingsMessage::Keyboard`), applied live + persisted by the handler; the
//! picker's open/search are UI-local (`LangPickerOpen`/`LangSearch`).
use compositor_developer_environment_preference_base::base::{KeyboardLayout, LayoutSource, LayoutSwitch};
use compositor_support_iced_core_engine_base::Renderer;
use compositor_configurator_settings_surface_message::message::SettingsMessage;
use compositor_configurator_settings_surface_style::style;
use compositor_configurator_settings_surface_control::control;
use iced_core::{Alignment, Element, Length, Theme};
use iced_widget::{button, column, container, pick_list, row, scrollable, text, text_input, toggler, Column};

type El<'a> = Element<'a, SettingsMessage, Theme, Renderer>;

/// Cap on the number of picker results rendered at once (the full catalogue is ~100
/// layouts; a search narrows it, and an unbounded column would be unwieldy).
const MAX_RESULTS: usize = 60;

fn card<'a>(inner: El<'a>) -> El<'a> {
    container(inner).style(style::card).width(Length::Fill).into()
}

/// Human label for a layout code from the catalogue, e.g. `"Hebrew (il)"`; falls
/// back to the bare code when the catalogue doesn't know it.
fn label_of(catalog: &[(String, String)], code: &str) -> String {
    catalog
        .iter()
        .find(|(c, _)| c == code)
        .map(|(_, n)| format!("{n} ({code})"))
        .unwrap_or_else(|| code.to_string())
}

/// Build the section's rows for splicing into the Language tab's scrollable list.
pub fn rows<'a>(
    k: &'a KeyboardLayout,
    catalog: &'a [(String, String)],
    picker_open: bool,
    search: &'a str,
) -> Vec<El<'a>> {
    let manual = k.source == LayoutSource::Manual;
    let mut out: Vec<El<'a>> = vec![
        column![
            text("KEYBOARD LAYOUT").size(14).color(style::ACCENT),
            text("Applied live. Use the environment (XKB_DEFAULT_*) for variants/options, or list layouts explicitly below.")
                .size(11)
                .color(style::MUTED),
        ]
        .spacing(4)
        .into(),
    ];

    // Source toggle: on = use the environment (hides the explicit list below).
    let src = k.clone();
    out.push(card(
        row![
            text("Use environment (XKB_DEFAULT_*)").width(Length::Fill),
            toggler(k.source == LayoutSource::Env)
                .on_toggle(move |env| {
                    let mut x = src.clone();
                    x.source = if env { LayoutSource::Env } else { LayoutSource::Manual };
                    SettingsMessage::Keyboard(x)
                })
                .style(control::toggler),
        ]
        .align_y(Alignment::Center)
        .spacing(10)
        .padding(12)
        .into(),
    ));

    // Under Env the explicit list is unused; show nothing more.
    if !manual {
        return out;
    }

    // Ordered layout list. First entry = default; the switch hotkey cycles them in
    // order. ↑/↓ reorder, − removes. Buttons only get an `on_press` where the action
    // is valid (no up on the first row, etc.), so edges are inert.
    out.push(text("LAYOUTS (in switch order)").size(11).color(style::MUTED).into());
    let n = k.layouts.len();
    if n == 0 {
        out.push(card(
            text("No layouts yet — add one below (falls back to US until you do).")
                .size(12)
                .color(style::MUTED)
                .into(),
        ));
    }
    for (i, code) in k.layouts.iter().enumerate() {
        let up = {
            let b = button(text("↑").size(13)).style(control::action);
            if i > 0 {
                b.on_press({ let mut x = k.clone(); x.layouts.swap(i, i - 1); SettingsMessage::Keyboard(x) })
            } else {
                b
            }
        };
        let down = {
            let b = button(text("↓").size(13)).style(control::action);
            if i + 1 < n {
                b.on_press({ let mut x = k.clone(); x.layouts.swap(i, i + 1); SettingsMessage::Keyboard(x) })
            } else {
                b
            }
        };
        let remove = button(text("−").size(14))
            .style(control::action)
            .on_press({ let mut x = k.clone(); x.layouts.remove(i); SettingsMessage::Keyboard(x) });
        out.push(card(
            row![text(label_of(catalog, code)).width(Length::Fill), up, down, remove]
                .align_y(Alignment::Center)
                .spacing(8)
                .padding(12)
                .into(),
        ));
    }

    // Add-layout: a toggle opening a searchable picker of the whole catalogue.
    out.push(
        button(text(if picker_open { "− Close" } else { "+ Add language" }).size(12))
            .style(control::action)
            .on_press(SettingsMessage::LangPickerOpen(!picker_open))
            .into(),
    );

    if picker_open {
        out.push(card(
            text_input("search layouts…", search).width(Length::Fill).on_input(SettingsMessage::LangSearch).into(),
        ));
        let q = search.to_ascii_lowercase();
        let mut results: Vec<El<'a>> = Vec::new();
        for (code, name) in catalog.iter() {
            if k.layouts.iter().any(|c| c == code) {
                continue; // already added
            }
            if !q.is_empty()
                && !name.to_ascii_lowercase().contains(&q)
                && !code.to_ascii_lowercase().contains(&q)
            {
                continue;
            }
            results.push(
                button(text(format!("{name}  ({code})")).size(13))
                    .width(Length::Fill)
                    .style(control::action)
                    .on_press({ let mut x = k.clone(); x.layouts.push(code.clone()); SettingsMessage::Keyboard(x) })
                    .into(),
            );
            if results.len() >= MAX_RESULTS {
                break;
            }
        }
        out.push(
            container(scrollable(Column::with_children(results).spacing(4)).height(Length::Fixed(220.0)))
                .style(style::card)
                .into(),
        );
    }

    // Switch hotkey: a preset dropdown → xkb `grp:` option. Only meaningful with two
    // or more layouts, but always shown so the choice is discoverable.
    out.push(text("SWITCH HOTKEY").size(11).color(style::MUTED).into());
    let cur = k.switch.label().to_string();
    let options: Vec<String> = LayoutSwitch::ALL.iter().map(|s| s.label().to_string()).collect();
    let ksw = k.clone();
    let picker = pick_list(Some(cur), options, |s: &String| s.clone())
        .on_select(move |s: String| {
            let mut x = ksw.clone();
            if let Some(sw) = LayoutSwitch::ALL.iter().find(|s2| s2.label() == s) {
                x.switch = *sw;
            }
            SettingsMessage::Keyboard(x)
        })
        .width(Length::Fixed(200.0))
        .style(control::picklist)
        .menu_style(control::menu);
    out.push(card(
        row![text("Cycle layouts with").width(Length::Fill), picker]
            .align_y(Alignment::Center)
            .spacing(10)
            .padding(12)
            .into(),
    ));

    out
}
