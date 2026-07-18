//! The Pen module: override a graphics tablet's driver defaults, since most tablets
//! ship no Linux configurator. Every control defaults to "Default" (native driver /
//! tablet-v2 behavior). Stylus barrel + pad buttons and the dial are remapped via
//! dropdowns; a button can also be bound to a captured key combo ("Key combo…" → press
//! the keys). Pad buttons are added by pressing them ("Add pad button"). A pressure
//! threshold turns light contact into a plain cursor with bindable in-pen clicks.
//! Edits emit the full [`PenConfig`]; the handler persists it to `preferences.json`.

use compositor_developer_environment_preference_base::base::{PenAction, PenBindTarget, PenButton, PenConfig};
use compositor_support_iced_core_engine_base::Renderer;
use compositor_configurator_settings_surface_message::message::SettingsMessage;
use compositor_configurator_settings_surface_style::style;
use compositor_configurator_settings_surface_control::control;
use iced_core::{Alignment, Element, Length, Theme};
use iced_widget::{button, column, container, pick_list, row, scrollable, slider, text, toggler, Column};

type El<'a> = Element<'a, SettingsMessage, Theme, Renderer>;

/// linux/input-event-codes.h stylus buttons.
const BTN_STYLUS: u32 = 0x14b;
const BTN_STYLUS2: u32 = 0x14c;

/// xkb modifier keycodes (evdev + 8) for the preset dial wheel remaps.
const XKB_CTRL: u32 = 37;
const XKB_SHIFT: u32 = 50;
const XKB_ALT: u32 = 64;

/// Emit an edited config.
fn msg(cfg: &PenConfig, edit: impl FnOnce(&mut PenConfig)) -> SettingsMessage {
    let mut x = cfg.clone();
    edit(&mut x);
    SettingsMessage::SetPen(x)
}

fn section<'a>(title: &'a str, sub: &'a str) -> El<'a> {
    column![
        text(title).size(13).color(style::ACCENT),
        text(sub).size(11).color(style::MUTED),
    ]
    .spacing(2)
    .into()
}

// ── stylus / pad button actions ────────────────────────────────────────────────

/// Fixed action options; "Key combo…" (appended per row) triggers a key capture.
const BUTTON_OPTIONS: [&str; 6] =
    ["Default", "Right click", "Middle click", "Left click", "Toggle hand mode", "Open touch menu"];
const KEY_OPTION: &str = "Key combo…";

fn button_action_from_label(s: &str) -> PenAction {
    match s {
        "Right click" => PenAction::Click(PenButton::Right),
        "Middle click" => PenAction::Click(PenButton::Middle),
        "Left click" => PenAction::Click(PenButton::Left),
        "Toggle hand mode" => PenAction::ToggleHandMode,
        "Open touch menu" => PenAction::OpenTouchMenu,
        _ => PenAction::Passthrough,
    }
}

fn button_action_label(a: &PenAction) -> &'static str {
    match a {
        PenAction::Click(PenButton::Right) => "Right click",
        PenAction::Click(PenButton::Middle) => "Middle click",
        PenAction::Click(PenButton::Left) => "Left click",
        PenAction::ToggleHandMode => "Toggle hand mode",
        PenAction::OpenTouchMenu => "Open touch menu",
        PenAction::Key(_) => "Key combo",
        _ => "Default",
    }
}

/// Apply a fixed action to a bind target (removing the entry for `Default`).
fn apply_target(x: &mut PenConfig, target: &PenBindTarget, action: PenAction) {
    let passthrough = matches!(action, PenAction::Passthrough);
    match target {
        PenBindTarget::Stylus(code) => {
            if passthrough {
                x.stylus_buttons.remove(&code.to_string());
            } else {
                x.stylus_buttons.insert(code.to_string(), action);
            }
        }
        PenBindTarget::Pad { device, button } => {
            let key = PenConfig::pad_key(device, *button);
            if passthrough {
                x.pad_buttons.remove(&key);
            } else {
                x.pad_buttons.insert(key, action);
            }
        }
    }
}

/// A `"device\u{1f}button"` pad key back to its parts.
fn parse_pad_key(k: &str) -> Option<(String, u32)> {
    let (d, b) = k.split_once('\u{1f}')?;
    Some((d.to_string(), b.parse().ok()?))
}

/// One button row: a label + an action dropdown. Selecting a fixed action forwards
/// `SetPen`; selecting "Key combo…" arms a key capture for this `target`.
fn action_row<'a>(cfg: &PenConfig, label: String, target: PenBindTarget, current: PenAction) -> El<'a> {
    let cur = button_action_label(&current).to_string();
    let mut options: Vec<String> = BUTTON_OPTIONS.iter().map(|s| s.to_string()).collect();
    options.push(KEY_OPTION.to_string());
    let c = cfg.clone();
    let t = target.clone();
    let picker = pick_list(Some(cur), options, |s: &String| s.clone())
        .on_select(move |s: String| {
            if s == KEY_OPTION {
                SettingsMessage::PenCaptureKey(t.clone())
            } else {
                let mut x = c.clone();
                apply_target(&mut x, &t, button_action_from_label(&s));
                SettingsMessage::SetPen(x)
            }
        })
        .width(Length::Fixed(220.0))
        .style(control::picklist)
        .menu_style(control::menu);
    container(
        row![text(label).width(Length::Fill), picker]
            .align_y(Alignment::Center)
            .spacing(10)
            .padding(12),
    )
    .style(style::card)
    .width(Length::Fill)
    .into()
}

// ── dial ───────────────────────────────────────────────────────────────────────

const DIAL_OPTIONS: [&str; 5] = ["Default", "Alt + Wheel", "Ctrl + Wheel", "Shift + Wheel", "Wheel"];

fn dial_from_label(s: &str) -> PenAction {
    match s {
        "Alt + Wheel" => PenAction::Wheel { mods: vec![XKB_ALT] },
        "Ctrl + Wheel" => PenAction::Wheel { mods: vec![XKB_CTRL] },
        "Shift + Wheel" => PenAction::Wheel { mods: vec![XKB_SHIFT] },
        "Wheel" => PenAction::Wheel { mods: vec![] },
        _ => PenAction::Passthrough,
    }
}

fn dial_label(a: &PenAction) -> &'static str {
    match a {
        PenAction::Wheel { mods } if mods.as_slice() == [XKB_ALT] => "Alt + Wheel",
        PenAction::Wheel { mods } if mods.as_slice() == [XKB_CTRL] => "Ctrl + Wheel",
        PenAction::Wheel { mods } if mods.as_slice() == [XKB_SHIFT] => "Shift + Wheel",
        PenAction::Wheel { .. } => "Wheel",
        _ => "Default",
    }
}

fn dial_row<'a>(cfg: &PenConfig) -> El<'a> {
    let cur = dial_label(&cfg.dial).to_string();
    let options: Vec<String> = DIAL_OPTIONS.iter().map(|s| s.to_string()).collect();
    let c = cfg.clone();
    let picker = pick_list(Some(cur), options, |s: &String| s.clone())
        .on_select(move |s: String| msg(&c, |x| x.dial = dial_from_label(&s)))
        .width(Length::Fixed(220.0))
        .style(control::picklist)
        .menu_style(control::menu);
    container(
        row![
            text("Dial (turn)").width(Length::Fill),
            picker,
        ]
        .align_y(Alignment::Center)
        .spacing(10)
        .padding(12),
    )
    .style(style::card)
    .width(Length::Fill)
    .into()
}

// ── below-threshold cursor ──────────────────────────────────────────────────────

const CLICK_SRC: [&str; 3] = ["None", "Lower barrel", "Upper barrel"];

fn click_src_label(code: Option<u32>) -> &'static str {
    match code {
        Some(BTN_STYLUS) => "Lower barrel",
        Some(BTN_STYLUS2) => "Upper barrel",
        _ => "None",
    }
}

fn click_src_from_label(s: &str) -> Option<u32> {
    match s {
        "Lower barrel" => Some(BTN_STYLUS),
        "Upper barrel" => Some(BTN_STYLUS2),
        _ => None,
    }
}

fn click_row<'a>(cfg: &PenConfig, label: &'a str, cur: Option<u32>, set: fn(&mut PenConfig, Option<u32>)) -> El<'a> {
    let cur_s = click_src_label(cur).to_string();
    let options: Vec<String> = CLICK_SRC.iter().map(|s| s.to_string()).collect();
    let c = cfg.clone();
    let picker = pick_list(Some(cur_s), options, |s: &String| s.clone())
        .on_select(move |s: String| msg(&c, |x| set(x, click_src_from_label(&s))))
        .width(Length::Fixed(180.0))
        .style(control::picklist)
        .menu_style(control::menu);
    container(
        row![text(label).width(Length::Fill), picker]
            .align_y(Alignment::Center)
            .spacing(10)
            .padding(12),
    )
    .style(style::card)
    .width(Length::Fill)
    .into()
}

pub fn build<'a>(cfg: &PenConfig, capturing: bool) -> El<'a> {
    let head = column![
        text("PEN & TABLET").size(16).color(style::ACCENT),
        text("Override the tablet's driver defaults — applied live. Every control is opt-in; \
              \"Default\" keeps the native tablet behavior.")
            .size(11)
            .color(style::MUTED),
    ]
    .spacing(4);

    let mut rows: Vec<El<'a>> = vec![head.into()];

    // Capture banner: a bind is armed — press the key combo / pad button (or cancel).
    if capturing {
        rows.push(
            container(
                row![
                    text("Press a key combo or pad button to bind…").size(13).color(style::ACCENT).width(Length::Fill),
                    button(text("Cancel").size(12)).on_press(SettingsMessage::PenCaptureCancel).style(control::action),
                ]
                .align_y(Alignment::Center)
                .spacing(10)
                .padding(12),
            )
            .style(style::card)
            .width(Length::Fill)
            .into(),
        );
    }

    rows.push(section("STYLUS BUTTONS", "Remap the on-pen barrel buttons. Default: native to tablet apps, else right / middle click."));
    rows.push(action_row(cfg, "Lower barrel button".to_string(), PenBindTarget::Stylus(BTN_STYLUS), cfg.stylus_action(BTN_STYLUS)));
    rows.push(action_row(cfg, "Upper barrel button".to_string(), PenBindTarget::Stylus(BTN_STYLUS2), cfg.stylus_action(BTN_STYLUS2)));

    // Pad buttons: press one to add it (identify by capture), then pick its action.
    rows.push(section("PAD BUTTONS", "Press \"Add pad button\", then press a button on the tablet to bind it."));
    rows.push(
        container(
            button(text("＋ Add pad button").size(13))
                .on_press(SettingsMessage::PenCapturePad)
                .style(control::action),
        )
        .padding(4)
        .into(),
    );
    let mut pads: Vec<(String, u32)> = cfg.pad_buttons.keys().filter_map(|k| parse_pad_key(k)).collect();
    pads.sort();
    for (device, button) in pads {
        let current = cfg.pad_action(&device, button);
        rows.push(action_row(
            cfg,
            format!("{device} · button {button}"),
            PenBindTarget::Pad { device: device.clone(), button },
            current,
        ));
    }

    rows.push(section("DIAL", "Default forwards the tablet-v2 dial (and zooms while the hand tool is on). Remap to a modifier + wheel for apps without tablet-v2 (e.g. Alt+Wheel = brush size)."));
    rows.push(dial_row(cfg));
    rows.push(section("PRESSURE PEN-DOWN", "Decide pen-down from real pressure instead of the driver's tip — for tablets that report \"pen down\" on mere detection. Below the threshold the pen hovers; above it draws/clicks."));

    // Toggle: derive pen-down from pressure.
    {
        let c = cfg.clone();
        rows.push(
            container(
                row![
                    column![
                        text("Pressure controls pen-down").size(13).color(style::ACCENT),
                        text("Ignore the driver's tip event; the pen is down only at/above the pressure below.")
                            .size(11)
                            .color(style::MUTED),
                    ]
                    .spacing(2)
                    .width(Length::Fill),
                    toggler(cfg.below_threshold_cursor)
                        .on_toggle(move |v| msg(&c, |x| x.below_threshold_cursor = v))
                        .style(control::toggler),
                ]
                .align_y(Alignment::Center)
                .spacing(10)
                .padding(12),
            )
            .style(style::card)
            .width(Length::Fill)
            .into(),
        );
    }

    if cfg.below_threshold_cursor {
        // Pen-down pressure threshold.
        let c = cfg.clone();
        rows.push(
            container(
                column![
                    row![
                        text("Pen-down pressure").size(12).color(style::MUTED).width(Length::Fill),
                        text(format!("{:.0}%", cfg.tip_threshold * 100.0)).size(12).color(style::ACCENT),
                    ]
                    .align_y(Alignment::Center)
                    .spacing(10),
                    slider(0.0..=1.0, cfg.tip_threshold, move |v| msg(&c, |x| x.tip_threshold = v))
                        .step(0.01f32)
                        .style(control::slider),
                ]
                .spacing(6)
                .padding(12),
            )
            .style(style::card)
            .width(Length::Fill)
            .into(),
        );
        rows.push(section("IN-PEN CLICKS", "Which barrel button clicks while the pen is below the threshold (a hovering cursor)."));
        rows.push(click_row(cfg, "Left click", cfg.below_left, |x, v| x.below_left = v));
        rows.push(click_row(cfg, "Right click", cfg.below_right, |x, v| x.below_right = v));
    }

    scrollable(Column::with_children(rows).spacing(12))
        .height(Length::Fill)
        .into()
}
