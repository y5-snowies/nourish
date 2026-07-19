//! The overview top menu bar (tactical HUD): a boxed logo + brand + `World`/`Layout`
//! tabs on the left; a live clock + `Settings` button on the right. Presentational —
//! selecting a tab emits [`OverviewMessage::Select`]; the clock is pushed in.
use iced_core::{Alignment, Background, Border, Color, Element, Length, Padding, Theme};
use iced_widget::{button, container, text, Column, Row, Space};
use compositor_support_iced_core_engine_base::{IcedUi, Renderer};

const ACCENT: Color = Color { r: 0.27, g: 0.78, b: 0.88, a: 1.0 };
const MUTED: Color = Color { r: 0.46, g: 0.54, b: 0.60, a: 1.0 };

/// Which menu section a click selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    World,
    Layout,
    Settings,
}

/// Messages the menu bar emits/receives. `Select` and `ToggleUser` are emitted by
/// clicks; `Clock` and `Battery` are pushed in by the overview per-frame.
#[derive(Debug, Clone)]
pub enum OverviewMessage {
    Select(Section),
    Clock(String),
    /// Battery indicator label (e.g. `"82%"`), or `None` to hide it (desktops).
    Battery(Option<String>),
    /// Toggle the logout popup (click the username brand). Actioned host-side.
    ToggleUser,
    /// Close the overview overlay (the touch-reachable ✕). Actioned host-side.
    Close,
}

/// The menu-bar UI instance.
pub struct OverviewMenu {
    selected: Section,
    clock: String,
    /// Battery indicator label, set on laptops only (`None` hides it).
    battery: Option<String>,
    /// The session user — shown as `[ {Initial} ] {user}` in the top-left brand.
    user: String,
}

impl OverviewMenu {
    /// Opens on `Layout` (the window grid) by default. `user` is the session
    /// username (e.g. "john"); the brand shows its initial in a box + the name.
    pub fn new(user: String) -> Self {
        Self { selected: Section::Layout, clock: String::new(), battery: None, user }
    }

    /// Uppercase first letter for the logo box (falls back to `U`).
    fn initial(&self) -> String {
        self.user.chars().next().map(|c| c.to_ascii_uppercase()).unwrap_or('U').to_string()
    }

    /// A tab: the selected one gets a cyan outlined "chip" highlight, the rest are
    /// plain muted text — the same treatment for World / Layout / Settings.
    fn tab<'a>(&self, label: &'a str, section: Section) -> Element<'a, OverviewMessage, Theme, Renderer> {
        let active = self.selected == section;
        button(text(label).size(13))
            .padding(Padding::from([6, 13]))
            .style(move |_t: &Theme, _s| button::Style {
                background: Some(Background::Color(if active {
                    Color { r: 0.27, g: 0.78, b: 0.88, a: 0.10 }
                } else {
                    Color::TRANSPARENT
                })),
                text_color: if active { ACCENT } else { MUTED },
                border: Border {
                    color: if active { ACCENT } else { Color::TRANSPARENT },
                    width: 1.0,
                    radius: 3.0.into(),
                },
                ..button::Style::default()
            })
            .on_press(OverviewMessage::Select(section))
            .into()
    }

    /// The username brand, rendered as a transparent button — clicking it toggles
    /// the floating logout popup (`OverviewMessage::ToggleUser`, actioned host-side).
    fn brand(&self) -> Element<'_, OverviewMessage, Theme, Renderer> {
        button(text(self.user.clone()).size(15).color(Color::WHITE))
            .padding(Padding::from([2, 4]))
            .style(|_t: &Theme, _s| button::Style {
                background: Some(Background::Color(Color::TRANSPARENT)),
                text_color: Color::WHITE,
                ..button::Style::default()
            })
            .on_press(OverviewMessage::ToggleUser)
            .into()
    }

    /// The touch-reachable close control (✕): dismisses the whole overlay.
    fn close(&self) -> Element<'_, OverviewMessage, Theme, Renderer> {
        button(text("✕").size(15))
            .padding(Padding::from([6, 12]))
            .style(|_t: &Theme, _s| button::Style {
                background: Some(Background::Color(Color::TRANSPARENT)),
                text_color: MUTED,
                ..button::Style::default()
            })
            .on_press(OverviewMessage::Close)
            .into()
    }
}

impl IcedUi for OverviewMenu {
    type Message = OverviewMessage;

    fn view(&self) -> Element<'_, Self::Message, Theme, Renderer> {
        let logo = container(text(self.initial()).size(14).color(ACCENT))
            .padding(Padding::from([1, 6]))
            .style(|_t: &Theme| container::Style {
                border: Border { color: ACCENT, width: 1.0, radius: 3.0.into() },
                ..Default::default()
            });
        let sep = container(Space::new())
            .width(Length::Fixed(1.0))
            .height(Length::Fixed(18.0))
            .style(|_t: &Theme| container::Style {
                background: Some(Background::Color(Color { r: 1.0, g: 1.0, b: 1.0, a: 0.14 })),
                ..Default::default()
            });
        let mut items: Vec<Element<'_, Self::Message, Theme, Renderer>> = vec![logo.into(), self.brand()];
        items.push(sep.into());
        items.push(self.tab("WORLD", Section::World));
        items.push(self.tab("LAYOUT", Section::Layout));
        items.push(Space::new().width(Length::Fill).into());
        if let Some(battery) = &self.battery {
            items.push(text(battery.clone()).size(13).color(MUTED).into());
        }
        items.push(text(self.clock.clone()).size(13).color(MUTED).into());
        items.push(self.tab("SETTINGS", Section::Settings));
        items.push(self.close());
        let bar = Row::with_children(items)
            .spacing(12)
            .align_y(Alignment::Center)
            .padding(Padding::from([0, 18]));

        container(bar)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_y(Length::Fill)
            .style(|_t: &Theme| container::Style {
                background: Some(Background::Color(Color { r: 0.027, g: 0.043, b: 0.059, a: 0.92 })),
                text_color: Some(Color::WHITE),
                ..Default::default()
            })
            .into()
    }

    fn update(&mut self, message: Self::Message) {
        match message {
            OverviewMessage::Select(section) => self.selected = section,
            OverviewMessage::Clock(c) => self.clock = c,
            OverviewMessage::Battery(b) => self.battery = b,
            // Opening/closing the popup is host-side (a separate surface).
            OverviewMessage::ToggleUser => {}
            // Closing the overlay is host-side (flips the visible flag).
            OverviewMessage::Close => {}
        }
    }
}

/// The floating logout popup, opened under the username brand as its own screen-space
/// surface. Stateless: its buttons route a [`LogoutMessage`] to the host, which either
/// ends the session (`Confirm`) or destroys this surface (`Cancel`).
pub struct LogoutPopup {
    user: String,
}

/// What the logout popup's buttons request from the host.
#[derive(Debug, Clone)]
pub enum LogoutMessage {
    /// End the session.
    Confirm,
    /// Dismiss the popup, staying logged in.
    Close,
}

impl LogoutPopup {
    pub fn new(user: String) -> Self {
        Self { user }
    }
}

impl IcedUi for LogoutPopup {
    type Message = LogoutMessage;

    fn view(&self) -> Element<'_, Self::Message, Theme, Renderer> {
        let hairline = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.14 };
        let red = Color { r: 0.88, g: 0.30, b: 0.32, a: 1.0 };
        let confirm = button(text("Log out").size(13))
            .width(Length::Fill)
            .padding(Padding::from([7, 12]))
            .style(move |_t: &Theme, _s| button::Style {
                background: Some(Background::Color(Color { r: 0.88, g: 0.30, b: 0.32, a: 0.14 })),
                text_color: red,
                border: Border { color: red, width: 1.0, radius: 3.0.into() },
                ..button::Style::default()
            })
            .on_press(LogoutMessage::Confirm);
        let close = button(text("Close").size(13))
            .width(Length::Fill)
            .padding(Padding::from([7, 12]))
            .style(move |_t: &Theme, _s| button::Style {
                background: Some(Background::Color(Color::TRANSPARENT)),
                text_color: MUTED,
                border: Border { color: hairline, width: 1.0, radius: 3.0.into() },
                ..button::Style::default()
            })
            .on_press(LogoutMessage::Close);
        let body = Column::with_children(vec![
            text(format!("Sign out of {} ?", self.user)).size(13).color(Color::WHITE).into(),
            confirm.into(),
            close.into(),
        ])
        .spacing(8)
        .padding(12);

        container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_t: &Theme| container::Style {
                background: Some(Background::Color(Color { r: 0.027, g: 0.043, b: 0.059, a: 0.97 })),
                border: Border { color: hairline, width: 1.0, radius: 5.0.into() },
                text_color: Some(Color::WHITE),
                ..Default::default()
            })
            .into()
    }

    fn update(&mut self, _message: Self::Message) {}
}
