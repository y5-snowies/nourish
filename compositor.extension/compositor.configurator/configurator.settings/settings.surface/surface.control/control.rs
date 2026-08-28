//! Button / slider / toggler styles for the settings HUD (paired with surface.style).
use iced_core::{Background, Border, Color, Shadow, Theme};
use iced_widget::button::{Status as BStatus, Style as Button};
use iced_widget::slider::{Handle, HandleShape, Rail, Status as SStatus, Style as Slider};
use iced_widget::toggler::{Status as TStatus, Style as Toggler};
use iced_widget::pick_list::{Status as PStatus, Style as PickList};
use iced_widget::text_input::{Status as IStatus, Style as TextInput};
use iced_widget::overlay::menu::Style as Menu;
use compositor_configurator_settings_surface_style::style::{rgba, ACCENT, LINE, MUTED, TEXT};

fn bg(c: Color) -> Background {
    Background::Color(c)
}

/// Primary accent (filled) button — APPLY, Keep, command actions.
pub fn accent(_t: &Theme, _s: BStatus) -> Button {
    Button { background: Some(bg(ACCENT)), text_color: Color::BLACK,
        border: Border { color: Color::TRANSPARENT, width: 0.0, radius: 3.0.into() }, ..Default::default() }
}

/// Outlined flat action / field button.
pub fn action(_t: &Theme, _s: BStatus) -> Button {
    Button { background: Some(bg(rgba(1.0, 1.0, 1.0, 0.03))), text_color: TEXT,
        border: Border { color: LINE, width: 1.0, radius: 3.0.into() }, ..Default::default() }
}

/// Greyed-out disabled button — faint fill, faint border, muted text. Use on a
/// button with no `on_press` so it both looks and behaves inert.
pub fn disabled(_t: &Theme, _s: BStatus) -> Button {
    Button { background: Some(bg(rgba(1.0, 1.0, 1.0, 0.02))), text_color: rgba(1.0, 1.0, 1.0, 0.25),
        border: Border { color: rgba(1.0, 1.0, 1.0, 0.06), width: 1.0, radius: 3.0.into() }, ..Default::default() }
}

/// A sidebar module row — cyan text + lit fill when active.
pub fn sidebar_item(active: bool) -> impl Fn(&Theme, BStatus) -> Button {
    move |_t, _s| Button {
        background: Some(bg(if active { rgba(0.27, 0.78, 0.88, 0.10) } else { Color::TRANSPARENT })),
        text_color: if active { ACCENT } else { MUTED },
        border: Border::default(),
        ..Default::default()
    }
}

/// A top tab (World/Layout) button.
pub fn tab(active: bool) -> impl Fn(&Theme, BStatus) -> Button {
    move |_t, _s| Button {
        background: Some(bg(Color::TRANSPARENT)),
        text_color: if active { Color::WHITE } else { MUTED },
        border: Border::default(),
        ..Default::default()
    }
}

/// Glowing cyan slider: filled left rail + round white handle.
pub fn slider(_t: &Theme, _s: SStatus) -> Slider {
    Slider {
        rail: Rail {
            backgrounds: (bg(ACCENT), bg(rgba(1.0, 1.0, 1.0, 0.08))),
            width: 4.0,
            border: Border { color: Color::TRANSPARENT, width: 0.0, radius: 2.0.into() },
        },
        handle: Handle {
            shape: HandleShape::Circle { radius: 7.0 },
            background: bg(Color::WHITE),
            border_width: 2.0,
            border_color: ACCENT,
        },
    }
}

/// Dropdown (closed state): dark field, cyan handle + hairline border.
pub fn picklist(_t: &Theme, _s: PStatus) -> PickList {
    PickList {
        text_color: TEXT,
        placeholder_color: MUTED,
        handle_color: ACCENT,
        background: bg(rgba(1.0, 1.0, 1.0, 0.03)),
        border: Border { color: LINE, width: 1.0, radius: 3.0.into() },
    }
}

/// Dropdown (open list): dark panel, cyan selected row.
pub fn menu(_t: &Theme) -> Menu {
    Menu {
        background: bg(rgba(0.055, 0.082, 0.106, 1.0)),
        border: Border { color: rgba(0.27, 0.78, 0.88, 0.35), width: 1.0, radius: 3.0.into() },
        text_color: TEXT,
        selected_text_color: Color::BLACK,
        selected_background: bg(ACCENT),
        shadow: Shadow::default(),
    }
}

/// Text field. Greys itself out when the widget is `Disabled` (i.e. rendered with
/// no `.on_input(..)`), so a field can be visually disabled just by omitting its
/// input handler — no layout shift and no separate "disabled field" widget.
pub fn field(_t: &Theme, s: IStatus) -> TextInput {
    let off = matches!(s, IStatus::Disabled);
    TextInput {
        background: bg(rgba(1.0, 1.0, 1.0, if off { 0.02 } else { 0.03 })),
        border: Border {
            color: if off { rgba(1.0, 1.0, 1.0, 0.06) } else { LINE },
            width: 1.0,
            radius: 3.0.into(),
        },
        placeholder: rgba(1.0, 1.0, 1.0, if off { 0.15 } else { 0.35 }),
        value: if off { rgba(1.0, 1.0, 1.0, 0.25) } else { TEXT },
        selection: ACCENT,
    }
}

/// Pill toggle — cyan when on, faint when off.
pub fn toggler(_t: &Theme, s: TStatus) -> Toggler {
    let on = matches!(s, TStatus::Active { is_toggled: true } | TStatus::Hovered { is_toggled: true });
    Toggler {
        background: bg(if on { ACCENT } else { rgba(1.0, 1.0, 1.0, 0.10) }),
        background_border_width: 0.0,
        background_border_color: Color::TRANSPARENT,
        foreground: bg(Color::WHITE),
        foreground_border_width: 0.0,
        foreground_border_color: Color::TRANSPARENT,
        text_color: None,
        border_radius: None,
        padding_ratio: 0.2,
    }
}
