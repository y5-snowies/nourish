//! The Current-World panel: live-preview pane (placeholder until the preview
//! widget lands), the available-shader list, and the editable `@prop` controls.
use compositor_configurator_settings_surface_control::control;
use compositor_configurator_settings_surface_message::message::{
    SettingsMessage, ShaderEntry, ShaderFacts, ShaderProp, ShaderPropKind,
};
use compositor_configurator_settings_surface_preview::preview::ParallaxPreview;
use compositor_configurator_settings_surface_style::style;
use compositor_pipeline_bundle_builtin_base::USER_MARK;
use compositor_monitor_selection_font_base::font::MATERIAL_FAMILY;
use compositor_monitor_selection_font_base::font_map;
use compositor_support_iced_core_engine_base::Renderer;
use iced_core::{Alignment, Element, Length, Padding, Theme};
use iced_widget::{
    button, column, container, responsive, row, scrollable, shader, slider, text, toggler,
};

type El<'a> = Element<'a, SettingsMessage, Theme, Renderer>;

/// Build the Current-World panel for the active world.
/// The preview pane's height, physical px.
const PREVIEW_H: f32 = 320.0;
/// Everything stacked above the two lists.
const ABOVE_LISTS: f32 = PREVIEW_H + 150.0;
/// Width of the category rail, physical px.
const CATEGORY_W: f32 = 128.0;
/// Floor for the shader list and the details column, physical px. iced has no
/// `min_height`, so it is a fixed floor plus a scroll to absorb the overflow.
const MIN_LISTS: f32 = 300.0;

#[allow(clippy::too_many_arguments)]
pub fn build<'a>(
    shaders: &'a [ShaderEntry],
    current: Option<&'a str>,
    category: Option<&'a str>,
    props: &'a [ShaderProp],
    preview_source: Option<&'a str>,
    facts: Option<&'a ShaderFacts>,
    status: Option<&'a str>,
    notice: Option<&'a str>,
    invert_pan_x: bool,
    invert_pan_y: bool,
    srgb: bool,
    optimized: bool,
    can_optimize: bool,
) -> El<'a> {
    responsive(move |avail| {
        // Grow with the window, but never past the floor going down.
        let lists = (avail.height - ABOVE_LISTS).max(MIN_LISTS);
        let body = column![
            text("CURRENT WORLD").size(16).color(style::ACCENT),
            text("The parallax shader rendered behind your workspace. Reacts to zoom & pan.")
                .size(11).color(style::MUTED),
            preview_or_error(props, preview_source, status),
            advisory(notice),
            display_row(invert_pan_x, invert_pan_y, srgb, optimized, can_optimize),
            row![
                container(category_rail(shaders, category))
                    .width(Length::Fixed(CATEGORY_W)).height(Length::Fixed(lists)),
                container(shader_list(shaders, current, category))
                    .width(Length::FillPortion(1)).height(Length::Fixed(lists)),
                container(details(facts, props))
                    .width(Length::FillPortion(1)).height(Length::Fixed(lists)),
            ].spacing(16).height(Length::Fixed(lists)),
        ].spacing(14);
        // Once `lists` is pinned at the floor the column outgrows the window.
        scrollable(body).height(Length::Fill).into()
    })
    .into()
}

/// The right-hand column: what the shader IS, above what it exposes. One scroll
/// region over both, so a tall card scrolls away instead of squeezing the list.
fn details<'a>(facts: Option<&'a ShaderFacts>, props: &'a [ShaderProp]) -> El<'a> {
    scrollable(column![properties_card(facts), variables(props)].spacing(14))
        .height(Length::Fill)
        .into()
}

/// What the selected shader declared. Empty for the built-in and single-pass
/// bundles — they have no graph to report.
fn properties_card(facts: Option<&ShaderFacts>) -> El<'_> {
    let Some(f) = facts else { return column![].into() };
    let line = |k: &'static str, v: String| -> El<'_> {
        row![
            text(k).size(11).color(style::MUTED).width(Length::Fixed(110.0)),
            text(v).size(11),
        ].spacing(8).into()
    };
    let bands = match f.after {
        0 => format!("{} pass{}", f.before, if f.before == 1 { "" } else { "es" }),
        n => format!("{} before, {n} after", f.before),
    };
    let mut col = column![
        text("PROPERTIES").size(10).color(style::MUTED),
        line("Graph", bands),
        line("World band", f.owns.clone()),
        line("Runs on", f.place.clone()),
    ].spacing(6);
    if let Some(w) = &f.warp {
        col = col.push(line("Pointer", w.clone()));
    }
    if let Some(c) = &f.chrome {
        col = col.push(line("Window chrome", c.clone()));
    }
    // One row per declared entry, in the words the manifest uses.
    for (entry, cost) in &f.requires {
        col = col.push(
            row![
                text(entry.clone()).size(11).color(style::ACCENT).width(Length::Fixed(110.0)),
                text(cost.clone()).size(10).color(style::MUTED),
            ].spacing(8),
        );
    }
    container(col.padding(12)).style(style::card).width(Length::Fill).into()
}

/// Per-world display toggles (persisted per world): flip the background parallax on
/// either axis (handy when a scene reads reversed relative to the pan), gamma-encode
/// the output to sRGB for the brighter, preview-matching look on the display, and
/// pick the cheap variant of the shader for a GPU that cannot afford the real one.
/// `optimize_available` is false when the selected shader declares no `@optimized`
/// knobs — it has no cheap twin, and a toggle that silently did nothing would be a
/// lie, so it renders disabled (a `toggler` with no `on_toggle`).
fn display_row<'a>(
    invert_pan_x: bool,
    invert_pan_y: bool,
    srgb: bool,
    optimized: bool,
    optimize_available: bool,
) -> El<'a> {
    let toggle = |label: &'a str, on: bool, msg: fn(bool) -> SettingsMessage| -> El<'a> {
        row![
            text(label).size(12),
            toggler(on).on_toggle(msg).style(control::toggler),
        ].spacing(8).align_y(Alignment::Center).into()
    };
    let optimize: El<'a> = row![
        text("Optimized")
            .size(12)
            .color(if optimize_available { style::TEXT } else { style::MUTED }),
        match optimize_available {
            true => toggler(optimized).on_toggle(SettingsMessage::SetWorldOptimized).style(control::toggler),
            false => toggler(false).style(control::toggler),
        },
    ].spacing(8).align_y(Alignment::Center).into();
    container(
        row![
            text("DISPLAY").size(10).color(style::MUTED).width(Length::Fill),
            toggle("Invert pan X", invert_pan_x, SettingsMessage::SetWorldInvertPanX),
            toggle("Invert pan Y", invert_pan_y, SettingsMessage::SetWorldInvertPanY),
            toggle("sRGB colour", srgb, SettingsMessage::SetWorldSrgb),
            optimize,
        ].spacing(20).align_y(Alignment::Center).padding(12),
    ).style(style::card).width(Length::Fill).into()
}

/// A non-fatal note about a shader that IS running.
///
/// Deliberately not the error card: the bundle compiled, is correct and is drawing.
/// What changed is where — it fell back to the compositor thread because something
/// it needs is momentarily unavailable. Replacing the preview over that would read
/// as "your shader is broken", which is worse than saying nothing; a quiet line
/// that appears and disappears with the condition is the honest shape.
///
/// Empty (zero-height) when there is nothing to say, so the panel does not shift.
fn advisory(notice: Option<&str>) -> El<'_> {
    match notice {
        Some(n) => container(
            text(format!("\u{26A0} {n}")).size(11).color(style::MUTED),
        )
        .padding(Padding::from([6, 12]))
        .style(style::card)
        .width(Length::Fill)
        .into(),
        None => column![].into(),
    }
}

/// The preview pane — or a compile-error card when the selected shader failed for
/// the active renderer (the built-in is running; the preview is hidden then), or a
/// no-preview card when the selection simply has nothing to preview.
///
/// An error outranks no-preview: a bundle that failed to compile also has no
/// preview, and "it is broken" is the more useful thing to say.
fn preview_or_error<'a>(
    props: &'a [ShaderProp],
    source: Option<&'a str>,
    status: Option<&'a str>,
) -> El<'a> {
    if let Some(err) = status {
        let body = column![
            text("\u{26A0} SHADER FAILED TO COMPILE").size(12).color(style::ACCENT),
            text("The built-in parallax is running. Fix the shader and re-select it.")
                .size(11).color(style::MUTED),
            text(err.to_string()).size(10).color(style::MUTED),
        ].spacing(8).padding(16);
        return container(scrollable(body)).style(style::card)
            .width(Length::Fill).height(Length::Fixed(PREVIEW_H)).into();
    }
    match source {
        Some(src) => preview_pane(props, src),
        None => no_preview(),
    }
}

/// The selection is a multipass graph, which the preview widget cannot run — it
/// drives one pass against one fixed binding. Previously this fell back to the
/// built-in parallax, showing a starfield for a completely different shader.
fn no_preview<'a>() -> El<'a> {
    let body = column![
        text("NO PREVIEW").size(12).color(style::MUTED),
        text("This shader renders as a multipass graph, which cannot run in the \
              preview. The desktop behind this window is showing it.")
            .size(11).color(style::MUTED),
    ].spacing(8).padding(16);
    container(body).style(style::card).width(Length::Fill).height(Length::Fixed(PREVIEW_H)).into()
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
    container(inner).style(style::card).width(Length::Fill).height(Length::Fixed(PREVIEW_H)).into()
}

/// The category rail: every heading present in the list, plus "All". Derived from
/// the entries, so a category a bundle invents needs no change here. First-seen
/// order — sorting would bury the stock parallax under `Abstract`.
fn category_rail<'a>(shaders: &'a [ShaderEntry], active: Option<&'a str>) -> El<'a> {
    let item = |label: String, to: Option<String>, selected: bool| -> El<'a> {
        // A heading from the shader folder is marked at its source (see
        // `shader.builtin::USER_MARK`) so it stays a SEPARATE group from a shipped
        // one of the same name. Here that mark becomes the icon it was standing in
        // for; the grouping and the filter keep using the marked string.
        let body: El<'a> = match label.strip_prefix(USER_MARK) {
            Some(rest) => row![
                text(font_map::Person).font(MATERIAL_FAMILY).size(13),
                text(rest.to_string()).size(12),
            ]
            .spacing(5)
            .align_y(Alignment::Center)
            .into(),
            None => text(label).size(12).into(),
        };
        button(body)
            .width(Length::Fill).padding(Padding::from([6, 10]))
            .on_press(SettingsMessage::SelectShaderCategory(to))
            .style(control::sidebar_item(selected)).into()
    };
    let mut col = column![text("CATEGORY").size(10).color(style::MUTED)].spacing(3);
    col = col.push(item("All".into(), None, active.is_none()));
    let mut seen: Vec<&str> = Vec::new();
    for s in shaders {
        if seen.contains(&s.category.as_str()) {
            continue;
        }
        seen.push(&s.category);
        col = col.push(item(
            s.category.clone(),
            Some(s.category.clone()),
            active == Some(s.category.as_str()),
        ));
    }
    scrollable(col).height(Length::Fill).into()
}

/// The available shaders, filtered to the active category. No inline headings —
/// the rail already names every group, in the same order.
fn shader_list<'a>(
    shaders: &'a [ShaderEntry],
    current: Option<&'a str>,
    category: Option<&'a str>,
) -> El<'a> {
    let item = |label: String, value: String, selected: bool| -> El<'a> {
        button(text(label).size(13))
            .width(Length::Fill).padding(Padding::from([8, 14]))
            .on_press(SettingsMessage::SetWorldShader(value))
            .style(control::sidebar_item(selected)).into()
    };
    let mut col = column![text("SHADER").size(10).color(style::MUTED)].spacing(4);
    let mut any = false;
    for s in shaders {
        if category.is_some_and(|c| c != s.category) {
            continue;
        }
        any = true;
        // The built-in parallax is the empty selection, and `current` is `None`
        // for it — so match on the value rather than on `Some(value)`.
        let selected = match s.value.is_empty() {
            true => current.is_none(),
            false => current == Some(s.value.as_str()),
        };
        col = col.push(item(s.label.clone(), s.value.clone(), selected));
    }
    // A category can empty out while the panel is open.
    if !any {
        col = col.push(text("Nothing in this category.").size(11).color(style::MUTED));
    }
    scrollable(col).height(Length::Fill).into()
}

/// The variable controls, split under their declared `group=` headings. Ungrouped
/// first. Does not scroll — [`details`] owns this column's one scroll region.
fn variables<'a>(props: &'a [ShaderProp]) -> El<'a> {
    if props.is_empty() {
        return text("This shader exposes no variables.").size(11).color(style::MUTED).into();
    }
    let mut col = column![text("VARIABLES").size(10).color(style::MUTED)].spacing(10);
    let mut shown: Option<&str> = None;
    for p in props {
        let group = (!p.group.is_empty()).then_some(p.group.as_str());
        if group.is_some() && group != shown {
            col = col.push(text(p.group.to_uppercase()).size(10).color(style::MUTED));
        }
        shown = group;
        col = col.push(variable_row(p, props));
    }
    col.into()
}

fn variable_row<'a>(p: &ShaderProp, all: &[ShaderProp]) -> El<'a> {
    let name = p.name.clone();
    let base = params_of(all);
    // Every kind produces one float in this variable's slot; only the picking differs.
    let set = |b: Vec<(String, f32)>, n: String| {
        move |v: f32| SettingsMessage::SetWorldShaderParams(with_value(&b, &n, v))
    };
    let control: El<'a> = match p.kind {
        ShaderPropKind::Float | ShaderPropKind::Color => {
            let send = set(base.clone(), name.clone());
            slider(p.min..=p.max, p.value, send)
                .step(((p.max - p.min) / 100.0).max(0.0001))
                .width(Length::Fixed(200.0)).style(control::slider).into()
        }
        // Whole numbers only — a `int` prop at 2.37 is a branch the shader lacks.
        ShaderPropKind::Int => {
            let send = set(base.clone(), name.clone());
            slider(p.min..=p.max, p.value.round(), send)
                .step(1.0)
                .width(Length::Fixed(200.0)).style(control::slider).into()
        }
        ShaderPropKind::Bool => {
            let send = set(base.clone(), name.clone());
            toggler(p.value > 0.5)
                .on_toggle(move |on| send(if on { 1.0 } else { 0.0 }))
                .style(control::toggler).into()
        }
        ShaderPropKind::Choice => {
            let selected = p.value.round().max(0.0) as usize;
            let mut r = row![].spacing(6);
            for (i, label) in p.choices.iter().enumerate() {
                let send = set(base.clone(), name.clone());
                r = r.push(
                    button(text(label.clone()).size(11))
                        .padding(Padding::from([4, 10]))
                        .on_press(send(i as f32))
                        .style(control::sidebar_item(i == selected)),
                );
            }
            r.into()
        }
    };
    // A picker already shows which entry is live; a number beside it is noise.
    let value: El<'a> = match p.kind {
        ShaderPropKind::Choice => column![].width(Length::Fixed(48.0)).into(),
        ShaderPropKind::Int => text(format!("{}", p.value.round() as i32))
            .color(style::ACCENT).width(Length::Fixed(48.0)).into(),
        _ => text(format!("{:.2}", p.value)).color(style::ACCENT).width(Length::Fixed(48.0)).into(),
    };
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
