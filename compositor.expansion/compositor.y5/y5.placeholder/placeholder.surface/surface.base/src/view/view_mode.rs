//! View mode: the placeholder's face — icon, identity, and the
//! Launch / Edit / Dismiss actions.
//!
//! Visual goals:
//! - Whole content centered on both axes inside the surface, at every step.
//! - Icon sits inside a soft "glassy" circular backdrop with a subtle
//!   highlight border and outer glow — approximating glassmorphism
//!   without backdrop blur (which iced doesn't support natively).
//! - Buttons use the accent color, rounded corners, slightly raised.
//!
//! Two arrangements, chosen by the [`Breakpoint`] the caller measured:
//! [`Breakpoint::Compact`] is a ROW (icon | two text lines | actions flush
//! right), the others a centered COLUMN. Both fit their step without
//! scrolling, so a tile at the layout floor is as usable as a full-size one.

use std::path::PathBuf;

use iced_core::text::{Ellipsis, Wrapping};
use iced_core::{alignment, Alignment, Background, Border, Color, ContentFit, Element, Length, Padding, Shadow, Theme, Vector};
use iced_widget::{button, column, container, image, row, svg, text};
use compositor_introspection_extraction_window_base::attributes::{AppId, DisplayName, ExecArgs, ExecProgram, IconPath};
use compositor_support_iced_core_engine_base::Renderer;

use crate::breakpoint::{Action, Breakpoint};
use crate::message::PlaceholderMessage;
use crate::style;
use crate::ui::PlaceholderUi;

/// Outer "glass" backdrop is a bit larger than the icon itself.
const ICON_BACKDROP_SCALE: f32 = 1.375;

pub fn render(
    ui: &PlaceholderUi,
    step: Breakpoint,
) -> Element<'_, PlaceholderMessage, Theme, Renderer> {
    let plan = ui.shown_plan();
    let icon_px = step.icon_px();

    let icon = match plan.current::<IconPath>() {
        Some(path) => render_icon(path, icon_px),
        None => fallback_glyph(icon_px),
    };
    let icon_with_backdrop = icon;
    // let icon_with_backdrop = backdrop(icon, icon_px);

    // Read the app_id as an ATTRIBUTE, not off `application_data.meta`. The raw
    // Meta is only populated while the window is alive: a plan rebuilt from a
    // persisted record carries `Meta::default()`, so a restored tile would show
    // no app_id at all. The hint survives persistence and honours user edits.
    let app_id = plan.current::<AppId>();
    let exec = exec_line(plan);

    let content: Element<'_, _, _, _> = if step.is_row() {
        // Compact: icon | two identity lines | actions, flush right. The text
        // column takes the slack so the actions sit against the right edge.
        let display_name = plan.current::<DisplayName>();
        row![
            icon_with_backdrop,
            column![
                detail_line(app_id.or(display_name).unwrap_or_default(), step, style::TEXT_DIM, Ellipsis::End),
                detail_line(exec.unwrap_or_default(), step, style::TEXT_HINT, Ellipsis::Start),
            ]
            .spacing(step.line_gap())
            .width(Length::Fill),
            buttons(step),
        ]
        .spacing(step.gap())
        .align_y(Alignment::Center)
        .into()
    } else {
        let display_name = plan
            .current::<DisplayName>()
            .unwrap_or_else(|| "Unknown".to_string());
        let mut col = column![
            icon_with_backdrop,
            text(display_name)
                .size(step.title_size())
                .align_x(alignment::Horizontal::Center)
                .style(|_| iced_widget::text::Style {
                    color: Some(style::TEXT),
                }),
        ]
        .spacing(step.gap())
        .align_x(Alignment::Center);

        if let Some(app_id) = app_id {
            col = col.push(detail_line(app_id, step, style::TEXT_DIM, Ellipsis::End));
        }
        if let Some(exec) = exec {
            col = col.push(detail_line(exec, step, style::TEXT_HINT, Ellipsis::Start));
        }

        col.push(buttons(step)).into()
    };

    container(content)
        .padding(step.outer_padding())
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center)
        .into()
}

/// The command line the plan would run: `ExecProgram` followed by `ExecArgs`.
///
/// Both are what a default launch actually uses — `request_from_plan` hands
/// exactly this pair to `generic_command` — so showing the program alone
/// under-reported the launch whenever the app carried arguments.
///
/// It is the PLAN's command, not the final argv: an active handler's
/// synthesizer may add flags of its own, and a containerised launch is
/// wrapped in `podman exec` later. Showing the plan is the right call here —
/// it is the thing the user edits in Settings.
///
/// Arguments containing whitespace are quoted, so a single argument with a
/// space in it doesn't read as two.
fn exec_line(plan: &compositor_introspection_launchplan_plan_base::LaunchPlan) -> Option<String> {
    let program = plan.current::<ExecProgram>()?;
    let mut line = program.to_string_lossy().into_owned();
    for arg in plan.current::<ExecArgs>().unwrap_or_default() {
        line.push(' ');
        if arg.chars().any(char::is_whitespace) {
            line.push('"');
            line.push_str(&arg);
            line.push('"');
        } else {
            line.push_str(&arg);
        }
    }
    Some(line)
}

/// One secondary identity line (app_id or executable path).
///
/// Held to a single line: these are long, unbreakable strings, and letting
/// them wrap would grow the content past the step it was measured for — the
/// very thing the breakpoints exist to prevent. `ellipsis` picks which end
/// survives the truncation: [`Ellipsis::Start`] for a path, so the binary
/// name stays readable, [`Ellipsis::End`] for an app_id, so its prefix does.
fn detail_line<'a>(
    value: String,
    step: Breakpoint,
    color: Color,
    ellipsis: Ellipsis,
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    text(value)
        .size(step.detail_size())
        .width(Length::Fill)
        .align_x(if step.is_row() {
            alignment::Horizontal::Left
        } else {
            alignment::Horizontal::Center
        })
        .wrapping(Wrapping::None)
        .ellipsis(ellipsis)
        .style(move |_| iced_widget::text::Style { color: Some(color) })
        .into()
}

/// The Edit / Launch / Dismiss row. Order is fixed across breakpoints so the
/// buttons don't move under the pointer when the tile is resized; only their
/// labels (words → glyphs) and metrics change.
fn buttons<'a>(step: Breakpoint) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    row![
        action_button(step, Action::Edit, PlaceholderMessage::EnterSettings, false),
        action_button(step, Action::Launch, PlaceholderMessage::LaunchClicked, true),
        action_button(step, Action::Dismiss, PlaceholderMessage::DismissClicked, false),
    ]
    .spacing(step.button_gap())
    .align_y(Alignment::Center)
    .into()
}

/// One action button, sized per step: content-sized with padding where the
/// label is a word, a fixed square where it is a glyph (so the rounding lands
/// on a true circle rather than an ellipse).
fn action_button<'a>(
    step: Breakpoint,
    action: Action,
    message: PlaceholderMessage,
    primary: bool,
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    let color = if primary { Color::WHITE } else { style::TEXT };
    let label = text(step.label(action))
        .size(step.button_text_size())
        .style(move |_| iced_widget::text::Style { color: Some(color) });

    // A fixed-size button gives the glyph no room to sit off-centre, so the
    // label fills the button and centers itself in both axes.
    let btn = match step.button_diameter() {
        Some(d) => button(label.width(Length::Fill).height(Length::Fill).center())
            .width(Length::Fixed(d))
            .height(Length::Fixed(d))
            .padding(Padding::ZERO),
        None => button(label).padding(step.button_padding()),
    };
    let btn = btn.on_press(message);

    let radius = step.button_radius();
    if primary {
        btn.style(move |theme, status| button_primary(theme, status, radius)).into()
    } else {
        btn.style(move |theme, status| button_secondary(theme, status, radius)).into()
    }
}

// ── Icon rendering ──────────────────────────────────────────────────

/// Wrap the icon in a soft circular "glassy" backdrop.
fn backdrop<'a>(
    inner: Element<'a, PlaceholderMessage, Theme, Renderer>,
    icon_px: f32,
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    let backdrop_px = icon_px * ICON_BACKDROP_SCALE;
    container(inner)
        .width(Length::Fixed(backdrop_px))
        .height(Length::Fixed(backdrop_px))
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center)
        .style(move |_| container::Style {
            background: Some(Background::Color(style::ICON_BG)),
            border: Border {
                color: style::ICON_HIGHLIGHT,
                width: 1.0,
                radius: (backdrop_px / 2.0).into(),
            },
            // Outer accent glow gives the "lit from behind" feeling.
            shadow: Shadow {
                color: style::GLOW,
                offset: Vector::new(0.0, 0.0),
                blur_radius: 28.0,
            },
            text_color: Some(style::TEXT),
            snap: true,
        })
        .into()
}

fn render_icon<'a>(path: PathBuf, icon_px: f32) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    let is_svg = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("svg"))
        .unwrap_or(false);

    if is_svg {
        svg(svg::Handle::from_path(path))
            .width(Length::Fixed(icon_px))
            .height(Length::Fixed(icon_px))
            .content_fit(ContentFit::Contain)
            .into()
    } else {
        image(image::Handle::from_path(path))
            .width(Length::Fixed(icon_px))
            .height(Length::Fixed(icon_px))
            .content_fit(ContentFit::Contain)
            .into()
    }
}

fn fallback_glyph<'a>(icon_px: f32) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    container(
        text("?")
            .size(icon_px / 2.0)
            .style(|_| iced_widget::text::Style {
                color: Some(style::TEXT_HINT),
            }),
    )
    .width(Length::Fixed(icon_px))
    .height(Length::Fixed(icon_px))
    .align_x(alignment::Horizontal::Center)
    .align_y(alignment::Vertical::Center)
    .into()
}

// ── Button styles ───────────────────────────────────────────────────

fn button_primary(
    _theme: &Theme,
    status: iced_widget::button::Status,
    radius: f32,
) -> iced_widget::button::Style {
    use iced_widget::button::Status;
    let bg = match status {
        Status::Hovered => Color {
            r: 0.50,
            g: 0.72,
            b: 0.98,
            a: 1.0,
        },
        Status::Pressed => Color {
            r: 0.30,
            g: 0.55,
            b: 0.88,
            a: 1.0,
        },
        _ => style::ACCENT,
    };
    iced_widget::button::Style {
        background: Some(Background::Color(bg)),
        text_color: Color::WHITE,
        border: Border {
            color: style::BORDER_BRIGHT,
            width: 0.0,
            radius: radius.into(),
        },
        shadow: Shadow {
            color: style::GLOW,
            offset: Vector::new(0.0, 4.0),
            blur_radius: 12.0,
        },
        snap: true,
    }
}

fn button_secondary(
    _theme: &Theme,
    status: iced_widget::button::Status,
    radius: f32,
) -> iced_widget::button::Style {
    use iced_widget::button::Status;
    let bg = match status {
        Status::Hovered => Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.10,
        },
        Status::Pressed => Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.16,
        },
        _ => Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.05,
        },
    };
    iced_widget::button::Style {
        background: Some(Background::Color(bg)),
        text_color: style::TEXT,
        border: Border {
            color: style::BORDER_BRIGHT,
            width: 1.0,
            radius: radius.into(),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}
