//! Per-AttributeKind editor widget builders + a value-summary helper.
//!
//! For Text / Path / EnumOf / chrome_profile attributes, the editor is
//! either a plain `text_input + chevron button` (when no combo is open)
//! or a `combo_box` bound to the UI's single shared combo state (when
//! the user has opened the alternatives picker for this attribute).
//! Only one combo can be open at a time across the whole UI.

use std::any::Any;
use std::sync::Arc;

use iced_core::{Element, Length, Theme};
use iced_widget::{
    button, checkbox, column, combo_box, row, text, text_input,
};
use compositor_introspection_extraction_window_base::{AttributeDescriptor, AttributeKind, EnvPair};
use compositor_support_iced_core_engine_base::Renderer;

use crate::message::{EnvField, PlaceholderMessage};
use crate::style;
use crate::ui::PlaceholderUi;

/// Build the editor widget for one attribute.
pub fn render<'a>(
    ui: &'a PlaceholderUi,
    descriptor: &AttributeDescriptor,
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    let value = ui.working.current_raw(descriptor);

    match &descriptor.kind {
        AttributeKind::Text => text_editor(ui, descriptor, &value),
        AttributeKind::Path => path_editor(ui, descriptor, &value),
        AttributeKind::Bool => bool_editor(descriptor, &value),
        AttributeKind::StringList => string_list_editor(descriptor, &value),
        AttributeKind::EnumOf(variants) => enum_editor(ui, descriptor, &value, variants),
        AttributeKind::Custom(tag) => custom_editor(ui, descriptor, &value, tag),
    }
}

// ── Editors per kind ────────────────────────────────────────────────

fn text_editor<'a>(
    ui: &'a PlaceholderUi,
    descriptor: &AttributeDescriptor,
    value: &Option<Arc<dyn Any + Send + Sync>>,
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    let current = value
        .as_ref()
        .and_then(|v| v.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    text_or_combo(ui, descriptor, current, "")
}

fn path_editor<'a>(
    ui: &'a PlaceholderUi,
    descriptor: &AttributeDescriptor,
    value: &Option<Arc<dyn Any + Send + Sync>>,
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    let current = value
        .as_ref()
        .and_then(|v| v.downcast_ref::<std::path::PathBuf>().cloned())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    text_or_combo(ui, descriptor, current, "path")
}

/// Choose between combo_box (when alternatives picker is open for this
/// attribute) and text_input + chevron (otherwise).
fn text_or_combo<'a>(
    ui: &'a PlaceholderUi,
    descriptor: &AttributeDescriptor,
    current: String,
    placeholder: &'static str,
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    let key = descriptor.key;
    let alternatives_count = ui.alternatives_for(descriptor).len();
    let has_alternatives = alternatives_count > 0;

    if ui.combo_active == Some(key) {
        // Expanded combo_box bound to the UI's shared state.
        combo_box(
            &ui.combo_state,
            placeholder,
            None,
            move |label: String| PlaceholderMessage::AlternativeSelected {
                descriptor_key: key,
                label,
            },
        )
        .on_input(move |v| PlaceholderMessage::AttributeTextChanged {
            descriptor_key: key,
            value: v,
        })
        .width(Length::Fill)
        .into()
    } else {
        // Plain text_input + (optional) chevron to open the picker.
        let input = text_input(placeholder, &current)
            .on_input(move |v| PlaceholderMessage::AttributeTextChanged {
                descriptor_key: key,
                value: v,
            })
            .padding(style::PAD_SMALL)
            .size(style::TEXT_SIZE_BODY)
            .width(Length::Fill);

        if has_alternatives {
            row![
                input,
                button(text("▾").size(style::TEXT_SIZE_BODY))
                    .padding(style::PAD_SMALL)
                    .on_press(PlaceholderMessage::ComboOpen { descriptor_key: key }),
            ]
            .spacing(4)
            .into()
        } else {
            input.into()
        }
    }
}

fn bool_editor<'a>(
    descriptor: &AttributeDescriptor,
    value: &Option<Arc<dyn Any + Send + Sync>>,
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    let current = value
        .as_ref()
        .and_then(|v| v.downcast_ref::<bool>().copied())
        .unwrap_or(false);
    let key = descriptor.key;
    checkbox(current)
        .on_toggle(move |v| PlaceholderMessage::AttributeBoolChanged {
            descriptor_key: key,
            value: v,
        })
        .into()
}

fn string_list_editor<'a>(
    descriptor: &AttributeDescriptor,
    value: &Option<Arc<dyn Any + Send + Sync>>,
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    let list: Vec<String> = value
        .as_ref()
        .and_then(|v| v.downcast_ref::<Vec<String>>().cloned())
        .unwrap_or_default();

    let key = descriptor.key;
    let mut col = column![].spacing(4);

    for (index, item) in list.iter().enumerate() {
        let row_ = row![
            text_input("", item)
                .on_input(move |v| PlaceholderMessage::AttributeStringListItemChanged {
                    descriptor_key: key,
                    index,
                    value: v,
                })
                .padding(style::PAD_SMALL)
                .size(style::TEXT_SIZE_BODY)
                .width(Length::Fill),
            button(text("×").size(style::TEXT_SIZE_BODY))
                .padding(style::PAD_SMALL)
                .on_press(PlaceholderMessage::AttributeStringListRemove {
                    descriptor_key: key,
                    index,
                }),
        ]
        .spacing(6);
        col = col.push(row_);
    }

    col = col.push(
        button(text("+ Add").size(style::TEXT_SIZE_HINT))
            .padding(style::PAD_SMALL)
            .on_press(PlaceholderMessage::AttributeStringListAdd {
                descriptor_key: key,
            }),
    );
    col.into()
}

fn enum_editor<'a>(
    _ui: &'a PlaceholderUi,
    descriptor: &AttributeDescriptor,
    value: &Option<Arc<dyn Any + Send + Sync>>,
    variants: &[&'static str],
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    let current = value
        .as_ref()
        .and_then(|v| v.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| variants.first().map(|s| s.to_string()).unwrap_or_default());

    let key = descriptor.key;

    // Find current's position in variants; if absent, treat as -1 so
    // the next click lands on the first variant.
    let next_index = variants
        .iter()
        .position(|v| *v == current)
        .map(|i| (i + 1) % variants.len())
        .unwrap_or(0);

    let next_value: String = variants
        .get(next_index)
        .map(|s| s.to_string())
        .unwrap_or_default();

    row![
        text(current.clone())
            .size(style::TEXT_SIZE_BODY)
            .style(|_| iced_widget::text::Style { color: Some(style::TEXT) })
            .width(Length::Fill),
        button(text("↻").size(style::TEXT_SIZE_BODY))
            .padding(style::PAD_SMALL)
            .on_press(PlaceholderMessage::AttributeTextChanged {
                descriptor_key: key,
                value: next_value,
            }),
    ]
    .spacing(8)
    .into()
}

fn custom_editor<'a>(
    ui: &'a PlaceholderUi,
    descriptor: &AttributeDescriptor,
    value: &Option<Arc<dyn Any + Send + Sync>>,
    tag: &'static str,
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    match tag {
        "handler_id" => readonly(value.as_ref().map_or(String::from("—"), |v| summarize_value(v))),
        "sandbox" => readonly(value.as_ref().map_or(String::from("—"), |v| summarize_value(v))),
        "chrome_profile" => {
            // Editable dropdown sourced from chrome.available_profiles hints.
            chrome_profile_picker(ui, descriptor, value)
        }
        "chrome_profile_list" => {
            // Read-only list display.
            chrome_profile_list_readonly(value)
        }
        "env_pair_list" => env_pair_list_editor(descriptor, value),
        _ => readonly(format!("(unsupported widget kind: {tag})")),
    }
}

fn readonly<'a>(s: String) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    text(s)
        .size(style::TEXT_SIZE_BODY)
        .style(|_| iced_widget::text::Style {
            color: Some(style::TEXT_DIM),
        })
        .into()
}

fn chrome_profile_picker<'a>(
    ui: &'a PlaceholderUi,
    descriptor: &AttributeDescriptor,
    value: &Option<Arc<dyn Any + Send + Sync>>,
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    use compositor_introspection_extraction_window_base::handlers::chrome::attributes::{
        AvailableProfiles, ChromeProfileInfo,
    };

    let avail: Vec<ChromeProfileInfo> = ui
        .working
        .current::<AvailableProfiles>()
        .unwrap_or_default();

    let current = value
        .as_ref()
        .and_then(|v| v.downcast_ref::<String>().cloned())
        .unwrap_or_default();

    let key = descriptor.key;

    // No discovered profiles → plain text input.
    if avail.is_empty() {
        return text_input("Default", &current)
            .on_input(move |v| PlaceholderMessage::AttributeTextChanged {
                descriptor_key: key,
                value: v,
            })
            .padding(style::PAD_SMALL)
            .size(style::TEXT_SIZE_BODY)
            .width(Length::Fill)
            .into();
    }

    // Profiles known → text input + cycle button.
    let next_index = avail
        .iter()
        .position(|p| p.directory_name == current)
        .map(|i| (i + 1) % avail.len())
        .unwrap_or(0);

    let next_value = avail
        .get(next_index)
        .map(|p| p.directory_name.clone())
        .unwrap_or_default();

    row![
        text_input("Default", &current)
            .on_input(move |v| PlaceholderMessage::AttributeTextChanged {
                descriptor_key: key,
                value: v,
            })
            .padding(style::PAD_SMALL)
            .size(style::TEXT_SIZE_BODY)
            .width(Length::Fill),
        button(text("↻").size(style::TEXT_SIZE_BODY))
            .padding(style::PAD_SMALL)
            .on_press(PlaceholderMessage::AttributeTextChanged {
                descriptor_key: key,
                value: next_value,
            }),
    ]
    .spacing(4)
    .into()
}

fn chrome_profile_list_readonly<'a>(
    value: &Option<Arc<dyn Any + Send + Sync>>,
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    use compositor_introspection_extraction_window_base::handlers::chrome::attributes::ChromeProfileInfo;
    let list: Vec<ChromeProfileInfo> = value
        .as_ref()
        .and_then(|v| v.downcast_ref::<Vec<ChromeProfileInfo>>().cloned())
        .unwrap_or_default();

    if list.is_empty() {
        return readonly("(no profiles discovered)".to_string());
    }
    let mut col = column![].spacing(2);
    for p in list {
        col = col.push(
            text(format!("• {} ({})", p.display_name, p.directory_name))
                .size(style::TEXT_SIZE_HINT)
                .style(|_| iced_widget::text::Style {
                    color: Some(style::TEXT_DIM),
                }),
        );
    }
    col.into()
}

fn env_pair_list_editor<'a>(
    descriptor: &AttributeDescriptor,
    value: &Option<Arc<dyn Any + Send + Sync>>,
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    let list: Vec<EnvPair> = value
        .as_ref()
        .and_then(|v| v.downcast_ref::<Vec<EnvPair>>().cloned())
        .unwrap_or_default();

    let key = descriptor.key;
    let mut col = column![].spacing(4);

    for (index, pair) in list.iter().enumerate() {
        let row_ = row![
            text_input("KEY", &pair.key)
                .on_input(move |v| PlaceholderMessage::AttributeEnvPairChanged {
                    descriptor_key: key,
                    index,
                    field: EnvField::Key,
                    value: v,
                })
                .padding(style::PAD_SMALL)
                .size(style::TEXT_SIZE_BODY)
                .width(Length::FillPortion(1)),
            text_input("value", &pair.value)
                .on_input(move |v| PlaceholderMessage::AttributeEnvPairChanged {
                    descriptor_key: key,
                    index,
                    field: EnvField::Value,
                    value: v,
                })
                .padding(style::PAD_SMALL)
                .size(style::TEXT_SIZE_BODY)
                .width(Length::FillPortion(2)),
            button(text("×").size(style::TEXT_SIZE_BODY))
                .padding(style::PAD_SMALL)
                .on_press(PlaceholderMessage::AttributeEnvPairRemove {
                    descriptor_key: key,
                    index,
                }),
        ]
        .spacing(6);
        col = col.push(row_);
    }

    col = col.push(
        button(text("+ Add").size(style::TEXT_SIZE_HINT))
            .padding(style::PAD_SMALL)
            .on_press(PlaceholderMessage::AttributeEnvPairAdd {
                descriptor_key: key,
            }),
    );
    col.into()
}

// ── Value summarizer ────────────────────────────────────────────────

/// Produce a human-readable summary of a type-erased value, used to show
/// "best inferred: X" lines below editors. Best-effort downcast through
/// the known concrete types; falls back to a generic Debug-like message.
pub fn summarize_value(v: &Arc<dyn Any + Send + Sync>) -> String {
    use compositor_introspection_extraction_window_base::handlers::chrome::attributes::{BrowserVariant, ChromeProfileInfo};
    use compositor_introspection_extraction_window_base::handlers::jetbrains::attributes::{LauncherKind, Product};
    use compositor_introspection_extraction_window_base::handlers::terminal::attributes::TerminalKind;
    use compositor_introspection_extraction_window_base::{HandlerId, SandboxIdentity};

    if let Some(s) = v.downcast_ref::<String>() {
        return s.clone();
    }
    if let Some(b) = v.downcast_ref::<bool>() {
        return b.to_string();
    }
    if let Some(p) = v.downcast_ref::<std::path::PathBuf>() {
        return p.to_string_lossy().into_owned();
    }
    if let Some(list) = v.downcast_ref::<Vec<String>>() {
        return format!("[{}]", list.join(", "));
    }
    if let Some(pairs) = v.downcast_ref::<Vec<EnvPair>>() {
        let joined: Vec<String> = pairs
            .iter()
            .map(|p| format!("{}={}", p.key, p.value))
            .collect();
        return format!("{{{}}}", joined.join(", "));
    }
    if let Some(h) = v.downcast_ref::<HandlerId>() {
        return h.to_string();
    }
    if let Some(s) = v.downcast_ref::<SandboxIdentity>() {
        return match s {
            SandboxIdentity::None => "None".to_string(),
            SandboxIdentity::Flatpak { app_id } => format!("Flatpak({app_id})"),
            SandboxIdentity::Snap { instance_name } => format!("Snap({instance_name})"),
            SandboxIdentity::OtherContainer { hint } => format!("Container({hint})"),
        };
    }
    if let Some(profiles) = v.downcast_ref::<Vec<ChromeProfileInfo>>() {
        let names: Vec<&str> = profiles.iter().map(|p| p.directory_name.as_str()).collect();
        return format!("[{}]", names.join(", "));
    }
    if let Some(variant) = v.downcast_ref::<BrowserVariant>() {
        return format!("{variant:?}");
    }
    if let Some(p) = v.downcast_ref::<Product>() {
        return format!("{p:?}");
    }
    if let Some(l) = v.downcast_ref::<LauncherKind>() {
        return format!("{l:?}");
    }
    if let Some(t) = v.downcast_ref::<TerminalKind>() {
        return format!("{t:?}");
    }
    "(unknown type)".to_string()
}
