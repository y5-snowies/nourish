//! View tree for the launcher banner.
//!
//! ```text
//!   ╭─────────────────────────────────────────╮
//!   │  › chr                                  │  ← header (search chip when typing; reserved height)
//!   │                                         │
//!   │   [icon]   [ICON]   [icon]   [icon]     │  ← carousel (only real entries; no ghosts)
//!   │                                         │
//!   │     ◀ ▲ ▼ ▶  Google Chrome              │  ← footer (title + arrow hints when focused)
//!   ╰─────────────────────────────────────────╯
//! ```
//!
//! Design choices:
//! - Banner hugs its content. No outer scrim — the compositor owns
//!   the backdrop.
//! - No per-cell decoration except for the selected one; non-selected
//!   icons sit on the banner background directly.
//! - Selected cell: vivid accent fill + bright ring + soft glow. When
//!   focused, the glow intensifies and the directional arrow hints
//!   appear inline with the title.
//! - Carousel renders only as many cells as exist in `visible[]`,
//!   capped at `CAROUSEL_VISIBLE`. No ghost cells.
//! - Header and footer blocks reserve their heights so the banner
//!   doesn't resize across state changes.

use std::path::PathBuf;

use iced_core::alignment;
use iced_core::{
    Alignment, Background, Border, Color, ContentFit, Element, Length, Padding, Shadow, Theme,
    Vector,
};
use iced_widget::image::FilterMethod;
use iced_widget::{button, column, container, image, row, svg, text, Space};
use compositor_support_iced_core_engine_base::Renderer;

use crate::message::LauncherMessage;
use crate::model::{Application, Direction};
use crate::style;
use crate::ui::Launcher;

pub fn root(ui: &Launcher) -> Element<'_, LauncherMessage, Theme, Renderer> {
    let banner_body = column![header(ui), carousel(ui), footer(ui)]
        .spacing(style::ROW_GAP)
        .align_x(Alignment::Center);

    let banner = container(banner_body)
        .padding(Padding::from([style::BANNER_PAD_Y, style::BANNER_PAD_X]))
        .style(banner_style);

    // Fill the compositor-given surface and centre the banner inside
    // it. No scrim — the compositor decides whether to dim the
    // background behind the overlay.
    container(banner)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
}

// ─── Banner shell ───────────────────────────────────────────────────

fn banner_style(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(style::BANNER_BG)),
        border: Border {
            color: style::BANNER_BORDER,
            width: style::BANNER_BORDER_WIDTH,
            radius: style::BANNER_RADIUS.into(),
        },
        // Small offset + big blur reads as ambient depth, not a smear.
        shadow: Shadow {
            color: Color {
                a: 0.55,
                ..Color::BLACK
            },
            offset: Vector::new(0.0, 8.0),
            blur_radius: 36.0,
        },
        text_color: Some(style::TEXT),
        snap: true,
    }
}

// ─── Header: inline search chip ─────────────────────────────────────

fn header(ui: &Launcher) -> Element<'_, LauncherMessage, Theme, Renderer> {
    let content: Element<'_, _, _, _> = if ui.query().is_empty() {
        // Reserved-height empty space.
        Space::new().width(Length::Shrink).height(Length::Shrink).into()
    } else {
        container(
            text(format!("›  {}", ui.query()))
                .size(style::TEXT_SIZE_SEARCH)
                .style(|_: &Theme| iced_widget::text::Style {
                    color: Some(style::TEXT),
                }),
        )
        .padding(Padding::from([
            style::SEARCH_CHIP_PAD_Y,
            style::SEARCH_CHIP_PAD_X,
        ]))
        .style(search_chip_style)
        .into()
    };

    container(content)
        .width(Length::Fill)
        .height(Length::Fixed(style::HEADER_BLOCK_HEIGHT))
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center)
        .into()
}

fn search_chip_style(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(style::SEARCH_CHIP_BG)),
        border: Border {
            color: style::SEARCH_CHIP_BORDER,
            width: 1.0,
            radius: style::SEARCH_CHIP_RADIUS.into(),
        },
        shadow: Shadow::default(),
        text_color: Some(style::TEXT),
        snap: true,
    }
}

// ─── Carousel ───────────────────────────────────────────────────────

fn carousel(ui: &Launcher) -> Element<'_, LauncherMessage, Theme, Renderer> {
    let n_visible = ui.visible.len();
    let count = style::CAROUSEL_VISIBLE.min(n_visible.saturating_sub(ui.scroll_offset));

    // Handle the "no matches" case with a single muted line in place
    // of the carousel so the banner doesn't collapse to nothing.
    if count == 0 {
        return container(
            text("no matches")
                .size(style::TEXT_SIZE_TITLE)
                .style(|_: &Theme| iced_widget::text::Style {
                    color: Some(style::TEXT_DIM),
                }),
        )
        .height(Length::Fixed(style::CELL_PX))
        .align_y(alignment::Vertical::Center)
        .into();
    }

    let mut r = row![]
        .spacing(style::CELL_GAP)
        .align_y(Alignment::Center);

    for slot in 0..count {
        let idx = ui.scroll_offset + slot;
        let app_idx = ui.visible[idx];
        let app = &ui.apps[app_idx];
        let selected = idx == ui.cursor;
        r = r.push(icon_cell(app, selected, ui.is_focused()));
    }

    r.into()
}

fn icon_cell<'a>(
    app: &Application,
    selected: bool,
    focused: bool,
) -> Element<'a, LauncherMessage, Theme, Renderer> {
    let icon_size = if selected {
        style::ICON_PX_SELECTED
    } else {
        style::ICON_PX
    };
    let inner = render_icon(app.icon_path.as_ref(), icon_size);

    let mut cell = container(inner)
        .width(Length::Fixed(style::CELL_PX))
        .height(Length::Fixed(style::CELL_PX))
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center);

    if selected {
        cell = cell.style(move |_: &Theme| selected_cell_style(focused));
    }
    // Non-selected cells: no style override → transparent → icon sits
    // directly on the banner background.

    // Tap-to-launch: a touch/pen tap (or a mouse click) on a cell launches that app
    // immediately, skipping the keyboard focus→direction flow. The button is
    // transparent so only the cell's own styling shows. `direction` is a placement
    // hint the spawner currently ignores, so any value serves.
    button(cell)
        .padding(0)
        .style(|_: &Theme, _| button::Style { background: None, ..Default::default() })
        .on_press(LauncherMessage::Launch {
            id: app.id.clone(),
            bin: app.bin.clone(),
            args: app.args.clone(),
            direction: Direction::Down,
        })
        .into()
}

fn selected_cell_style(focused: bool) -> container::Style {
    let (bg_alpha, ring_alpha, glow_alpha, ring_width) = if focused {
        (0.50, 1.0, 0.70, 2.5)
    } else {
        (0.30, 0.85, 0.45, 2.0)
    };

    container::Style {
        background: Some(Background::Color(Color {
            a: bg_alpha,
            ..style::ACCENT
        })),
        border: Border {
            color: Color {
                a: ring_alpha,
                ..style::ACCENT_RING
            },
            width: ring_width,
            radius: style::CELL_RADIUS.into(),
        },
        shadow: Shadow {
            color: Color {
                a: glow_alpha,
                ..style::ACCENT
            },
            offset: Vector::new(0.0, 0.0),
            blur_radius: 24.0,
        },
        text_color: Some(style::TEXT),
        snap: true,
    }
}

// ─── Footer: title + directional arrow hints when focused ───────────

fn footer(ui: &Launcher) -> Element<'_, LauncherMessage, Theme, Renderer> {
    let title = text(ui.current_title())
        .size(style::TEXT_SIZE_TITLE)
        .style(|_: &Theme| iced_widget::text::Style {
            color: Some(style::TEXT),
        });

    let content: Element<'_, _, _, _> = if ui.is_focused() {
        // Arrow glyphs flank the title when focused. They sit at the
        // same height so the row is one clean line. The glyphs hint
        // at the actual key bindings.
        row![arrow_glyph("◀"), arrow_glyph("▲"), arrow_glyph("▼"), arrow_glyph("▶"), title]
            .spacing(style::ARROW_HORIZ_GAP)
            .align_y(Alignment::Center)
            .into()
    } else {
        title.into()
    };

    container(content)
        .width(Length::Fill)
        .height(Length::Fixed(style::FOOTER_BLOCK_HEIGHT))
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center)
        .into()
}

fn arrow_glyph<'a>(glyph: &'a str) -> Element<'a, LauncherMessage, Theme, Renderer> {
    text(glyph)
        .size(style::TEXT_SIZE_ARROW)
        .style(|_: &Theme| iced_widget::text::Style {
            color: Some(style::ACCENT_RING),
        })
        .into()
}

// ─── Icon image ─────────────────────────────────────────────────────

fn render_icon<'a>(
    icon_path: Option<&PathBuf>,
    size: f32,
) -> Element<'a, LauncherMessage, Theme, Renderer> {
    let Some(path) = icon_path else {
        return fallback_glyph(size);
    };

    let is_svg = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("svg"))
        .unwrap_or(false);

    if is_svg {
        svg(svg::Handle::from_path(path))
            .width(Length::Fixed(size))
            .height(Length::Fixed(size))
            .content_fit(ContentFit::Contain)
            .into()
    } else {
        // Linear filtering looks sharper than the nearest fallback
        // when the source PNG is close to our target size. The XDG
        // loader picks the smallest size ≥ ICON_PX_SELECTED, so
        // upscaling should be minimal.
        image(image::Handle::from_path(path))
            .width(Length::Fixed(size))
            .height(Length::Fixed(size))
            .content_fit(ContentFit::Contain)
            .filter_method(FilterMethod::Linear)
            .into()
    }
}

fn fallback_glyph<'a>(size: f32) -> Element<'a, LauncherMessage, Theme, Renderer> {
    container(Space::new().width(Length::Shrink).height(Length::Shrink))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(Color {
                a: 0.16,
                ..style::TEXT_FAINT
            })),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: (style::CELL_RADIUS - 4.0).into(),
            },
            shadow: Shadow::default(),
            text_color: None,
            snap: true,
        })
        .into()
}
