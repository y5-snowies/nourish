//! The Current-World panel: live-preview pane (placeholder until the preview
//! widget lands), the available-shader list, and the editable `@prop` controls.
use compositor_configurator_settings_surface_control::control;
use compositor_configurator_settings_surface_message::message::{
    SettingsMessage, ShaderProp, ShaderPropKind,
};
use compositor_configurator_settings_surface_preview::preview::ParallaxPreview;
use compositor_configurator_settings_surface_style::style;
use compositor_support_iced_core_engine_base::Renderer;
use iced_core::{Alignment, Element, Length, Padding, Theme};
use iced_widget::{button, column, container, row, scrollable, shader, slider, text, toggler};

type El<'a> = Element<'a, SettingsMessage, Theme, Renderer>;

/// Build the Current-World panel for the active world.
pub fn build<'a>(
    shaders: &'a [String],
    current: Option<&'a str>,
    props: &'a [ShaderProp],
    preview_source: &'a str,
    status: Option<&'a str>,
) -> El<'a> {
    column![
        text("CURRENT WORLD").size(16).color(style::ACCENT),
        text("The parallax shader rendered behind your workspace. Reacts to zoom & pan.")
            .size(11).color(style::MUTED),
        preview_or_error(props, preview_source, status),
        row![
            container(shader_list(shaders, current)).width(Length::FillPortion(1)).height(Length::Fill),
            container(variables(props)).width(Length::FillPortion(1)).height(Length::Fill),
        ].spacing(24).height(Length::Fill),
    ].spacing(14).height(Length::Fill).into()
}

/// The preview pane — or a compile-error card when the selected shader failed for
/// the active renderer (the built-in is running; the preview is hidden then).
fn preview_or_error<'a>(props: &'a [ShaderProp], source: &'a str, status: Option<&'a str>) -> El<'a> {
    if let Some(err) = status {
        let body = column![
            text("\u{26A0} SHADER FAILED TO COMPILE").size(12).color(style::ACCENT),
            text("The built-in parallax is running. Fix the shader and re-select it.")
                .size(11).color(style::MUTED),
            text(err.to_string()).size(10).color(style::MUTED),
        ].spacing(8).padding(16);
        return container(scrollable(body)).style(style::card)
            .width(Length::Fill).height(Length::Fixed(320.0)).into();
    }
    preview_pane(props, source)
}

/// The live wgpu preview of the selected shader, driven by the current variable
/// values + mouse pan (drag) / zoom (scroll).
fn preview_pane<'a>(props: &'a [ShaderProp], source: &'a str) -> El<'a> {
    let mut params = [0.0f32; 16];
    for p in props {
        if p.slot < 16 { params[p.slot] = p.value; }
    }
    let inner: El<'a> = if source.is_empty() {
        container(text("Preparing preview\u{2026}").size(11).color(style::MUTED))
            .padding(16).into()
    } else {
        shader(ParallaxPreview { source: source.to_string(), params })
            .width(Length::Fill).height(Length::Fill).into()
    };
    container(inner).style(style::card).width(Length::Fill).height(Length::Fixed(320.0)).into()
}

/// The available shaders: a scrollable selectable list of built-in + every bundle.
fn shader_list<'a>(shaders: &'a [String], current: Option<&'a str>) -> El<'a> {
    let item = |label: String, value: String, selected: bool| -> El<'a> {
        button(text(label).size(13))
            .width(Length::Fill).padding(Padding::from([8, 14]))
            .on_press(SettingsMessage::SetWorldShader(value))
            .style(control::sidebar_item(selected)).into()
    };
    let mut col = column![text("SHADER").size(10).color(style::MUTED)].spacing(4);
    col = col.push(item("Built-in parallax".into(), String::new(), current.is_none()));
    for s in shaders {
        col = col.push(item(shader_label(s), s.clone(), current == Some(s.as_str())));
    }
    scrollable(col).height(Length::Fill).into()
}

/// Display label for a shader selection: a compiled-in `builtin:leafy-planet`
/// shows as "Leafy Planet"; a user bundle folder name is shown verbatim.
fn shader_label(value: &str) -> String {
    match value.strip_prefix("builtin:") {
        Some(rest) => rest
            .split(['-', '_'])
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        None => value.to_string(),
    }
}

fn variables<'a>(props: &'a [ShaderProp]) -> El<'a> {
    if props.is_empty() {
        return text("This shader exposes no variables.").size(11).color(style::MUTED).into();
    }
    let mut col = column![text("VARIABLES").size(10).color(style::MUTED)].spacing(10);
    for p in props {
        col = col.push(variable_row(p, props));
    }
    scrollable(col).height(Length::Fill).into()
}

fn variable_row<'a>(p: &ShaderProp, all: &[ShaderProp]) -> El<'a> {
    let name = p.name.clone();
    let base = params_of(all);
    let control: El<'a> = match p.kind {
        ShaderPropKind::Float => {
            let (b, n) = (base.clone(), name.clone());
            slider(p.min..=p.max, p.value, move |v| {
                SettingsMessage::SetWorldShaderParams(with_value(&b, &n, v))
            })
            .step(((p.max - p.min) / 100.0).max(0.0001))
            .width(Length::Fixed(200.0)).style(control::slider).into()
        }
        ShaderPropKind::Bool => {
            let (b, n) = (base.clone(), name.clone());
            toggler(p.value > 0.5).on_toggle(move |on| {
                SettingsMessage::SetWorldShaderParams(with_value(&b, &n, if on { 1.0 } else { 0.0 }))
            }).style(control::toggler).into()
        }
    };
    let value = text(format!("{:.2}", p.value)).color(style::ACCENT).width(Length::Fixed(48.0));
    container(
        row![text(p.label.clone()).width(Length::Fill), control, value]
            .align_y(Alignment::Center).spacing(10).padding(12),
    ).style(style::card).width(Length::Fill).into()
}

/// The current (name, value) list for every variable.
fn params_of(props: &[ShaderProp]) -> Vec<(String, f32)> {
    props.iter().map(|p| (p.name.clone(), p.value)).collect()
}

/// `base` with the entry named `name` set to `value`.
fn with_value(base: &[(String, f32)], name: &str, value: f32) -> Vec<(String, f32)> {
    base.iter()
        .map(|(n, v)| (n.clone(), if n == name { value } else { *v }))
        .collect()
}
