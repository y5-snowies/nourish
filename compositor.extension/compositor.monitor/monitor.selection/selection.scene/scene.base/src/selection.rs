use crate::ui::Message::{ExecuteScaleToFit, ExecuteSelection};
use crate::ui::{Message, Overlay};
use iced_core::{
    Alignment, Background, Border, Color, Element, Font, Length, Padding, Shadow, Theme, alignment,
};
use iced_wgpu::Renderer;
use iced_widget::{Container, Row, Space, button, column, container, mouse_area, row, text};
use std::collections::HashSet;
use compositor_monitor_selection_font_base::font::MATERIAL_FAMILY;
use compositor_monitor_selection_font_base::font_map;
use crate::selection::SelectionAction::ScaleToFit;
//

// The message struct should now send multiple actions at once.(eg aligntop + alignbottom) as a vector.
//
// To send multiple actions:
// Hold shift, and click on the align/distribute/stack buttons.
//
// when clicking on align with shift:
//  all distribute and stack toggles are no longer 'active' if they were previously toggled.
//  it adds the toggle for the selected button so the message that will be sent later will include that direction.
//
// when clicking on distribute with shift:
//  similarly.
//
// when clicking on stack with shift:
//  similarly, but stack allows only a single button to be toggled at once.
//
// when 1 or more toggles are active, pan slides from the right nicely, it should anchor to the right side of the container. there, a commit button that changes icons when alt is pressed. on click, sends the repeated message.
//
// clicking without shift: if there is active toggle, stops it and no-op. otherwise, default behaviour, send the message. it does work though for the commit button.
//
// clicking outside any button while shift is not clicked: stops all toggles.
// unfocus: stops all toggles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectionAction {
    AlignTop,
    AlignBottom,
    AlignLeft,
    AlignVerticalCenter,
    AlignHorizontalCenter,
    AlignRight,
    DistributeHorizontal,
    DistributeVertical,
    StackHorizontal,
    StackVertical,
    ScaleToFit(ScaleToFitOption),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScaleToFitOption {
    pub perceived: bool,
    pub max: bool,
    pub vertical: bool,
    pub horizontal: bool
}
//

/// How aggressively the close button should dismiss the selected windows,
/// chosen by the modifiers held at click time:
///
/// - `Request` (no modifier): ask the window to close via the `xdg_toplevel.close`
///   protocol event. Targets the *surface*, not the process, so windows that
///   share a client process (Chrome windows, a terminal's windows) close just
///   the chosen one and the app runs its own teardown.
/// - `Discard` (Shift): the same polite close, but first mark the window so it
///   leaves NO placeholder tile behind. Only meaningful for windows that were
///   not themselves restored from a placeholder (those keep their placeholder).
/// - `Terminate` (Alt): SIGTERM the owning process (systemd scope stop, else
///   `kill -TERM`). The former default.
/// - `Kill` (Alt+Shift): SIGKILL the owning process (`kill -9`). The former Alt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseMode {
    Request,
    Discard,
    Terminate,
    Kill,
}

impl CloseMode {
    /// Derive the close strength from the modifier state held at click time.
    pub fn from_modifiers(alt: bool, shift: bool) -> Self {
        match (alt, shift) {
            (false, false) => CloseMode::Request,
            (false, true) => CloseMode::Discard,
            (true, false) => CloseMode::Terminate,
            (true, true) => CloseMode::Kill,
        }
    }

    /// Whether this mode kills the owning process (Alt held) rather than asking
    /// the window to close — drives the destructive skull/red affordance.
    /// `Discard` is a polite close (it only skips the placeholder), so it keeps
    /// the normal styling.
    pub fn is_destructive(self) -> bool {
        matches!(self, CloseMode::Terminate | CloseMode::Kill)
    }
}

#[derive(Debug, Default, Clone)]
pub struct SelectionState {
    /// Multi-select: any combination of AlignLeft/Center/Right active.
    pub align_toggles: HashSet<SelectionAction>,
    /// Multi-select: any combination of DistributeHorizontal/Vertical active.
    pub distribute_toggles: HashSet<SelectionAction>,
    /// Single-select: at most one of StackHorizontal/StackVertical.
    pub stack_toggle: Option<SelectionAction>,

    /// True when at least one of shift was held during the relevant click.
    /// Used to know whether to "stop all" on a click-outside.
    pub shift_held: bool,

    /// True when alt is currently held — for the commit button's icon swap.
    pub alt_held: bool,
}

impl SelectionState {
    pub fn has_any_toggle(&self) -> bool {
        !self.align_toggles.is_empty()
            || !self.distribute_toggles.is_empty()
            || self.stack_toggle.is_some()
    }

    pub fn is_active(&self, action: SelectionAction) -> bool {
        match action {
            SelectionAction::AlignTop
            | SelectionAction::AlignBottom
            | SelectionAction::AlignVerticalCenter
            | SelectionAction::AlignLeft
            | SelectionAction::AlignHorizontalCenter
            | SelectionAction::AlignRight => self.align_toggles.contains(&action),
            SelectionAction::DistributeHorizontal | SelectionAction::DistributeVertical => {
                self.distribute_toggles.contains(&action)
            }
            SelectionAction::StackHorizontal | SelectionAction::StackVertical => {
                self.stack_toggle == Some(action)
            }
            SelectionAction::ScaleToFit(_) => false,
        }
    }

    pub fn clear(&mut self) {
        self.align_toggles.clear();
        self.distribute_toggles.clear();
        self.stack_toggle = None;
    }

    pub fn collect_actions(&self) -> Vec<SelectionAction> {
        let mut out = Vec::new();
        out.extend(self.align_toggles.iter().copied());
        out.extend(self.distribute_toggles.iter().copied());
        out.extend(self.stack_toggle);
        out
    }
}
pub fn category(action: SelectionAction) -> SelectionCategory {
    match action {
        SelectionAction::AlignTop
        | SelectionAction::AlignBottom
        | SelectionAction::AlignVerticalCenter
        | SelectionAction::AlignLeft
        | SelectionAction::AlignHorizontalCenter
        | SelectionAction::AlignRight => SelectionCategory::Align,
        SelectionAction::DistributeHorizontal | SelectionAction::DistributeVertical => {
            SelectionCategory::Distribute
        }
        SelectionAction::StackHorizontal | SelectionAction::StackVertical => {
            SelectionCategory::Stack
        }
        SelectionAction::ScaleToFit(_) => SelectionCategory::ScaleToFit,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionCategory {
    Align,
    Distribute,
    Stack,
    ScaleToFit,
}

impl Overlay {
    pub fn selection(&self) -> Container<'_, Message> {
        let icon_button =
            |glyph: &'static str, action: Message| -> Element<'_, Message, Theme, Renderer> {
                let kind = tip_kind(&action);
                let is_active = match &action {
                    Message::SelectionClicked(a) => self.selection.is_active(*a),
                    _ => false,
                };

                let label = text(glyph)
                    .font(MATERIAL_FAMILY)
                    .size(20)
                    .center()
                    .style(|_theme| text::Style {
                        color: Some(Color::from_rgb(0.15, 0.18, 0.24)),
                    });

                let btn = button(
                    container(label)
                        .center_x(Length::Fill)
                        .center_y(Length::Fill)
                        .width(Length::Fixed(36.0))
                        .height(Length::Fixed(36.0)),
                )
                .padding(0)
                .on_press(action)
                .style(move |_theme, status| {
                    let (bg, border_color, border_width) = match (status, is_active) {
                        (button::Status::Hovered, _) => (
                            Color::from_rgb(0.96, 0.97, 0.99),
                            Color::from_rgb(0.4, 0.55, 0.95),
                            1.5,
                        ),
                        (button::Status::Pressed, _) => (
                            Color::from_rgb(0.92, 0.94, 0.98),
                            Color::from_rgb(0.3, 0.45, 0.9),
                            1.5,
                        ),
                        (_, true) => (
                            Color::from_rgb(0.94, 0.96, 1.0),
                            Color::from_rgb(0.4, 0.55, 0.95),
                            1.5,
                        ),
                        _ => (
                            Color::from_rgb(0.98, 0.98, 0.99),
                            Color::from_rgba(0.0, 0.0, 0.0, 0.06),
                            1.0,
                        ),
                    };

                    button::Style {
                        snap: true,
                        background: Some(Background::Color(bg)),
                        text_color: Color::from_rgb(0.15, 0.18, 0.24),
                        border: Border {
                            color: border_color,
                            width: border_width,
                            radius: 6.0.into(),
                        },
                        shadow: Shadow::default(),
                    }
                });

                let el: Element<'_, Message, Theme, Renderer> = btn.into();
                match kind {
                    Some(k) => mouse_area(el)
                        .on_enter(Message::Hover(Some(k)))
                        .on_exit(Message::Hover(None))
                        .into(),
                    None => el,
                }
            };

        let press_action = |f| {
            if self.selection.shift_held {
                Message::SelectionClicked(f)
            } else if self.selection.has_any_toggle() {
                Message::SelectionClicked(f)
            } else {
                Message::ExecuteSelection(vec![f], self.selection.alt_held)
            }
        };

        // --- Close: dismiss every selected window. The strength is chosen by
        // the modifiers held at render time (baked into the message, since the
        // view re-renders on Shift/AltChanged), each with its own glyph:
        //   none      -> ask the window to close (xdg_toplevel.close protocol)  [door]
        //   Shift     -> close AND leave no placeholder tile                    [bin]
        //   Alt       -> SIGTERM the owning process                             [power-off]
        //   Alt+Shift -> SIGKILL the owning process                             [skull]
        // Holding Alt (either process-killing variant) deepens the red to
        // signal the destructive intent. ---
        let mode = CloseMode::from_modifiers(self.selection.alt_held, self.selection.shift_held);
        let destructive = mode.is_destructive();
        let close_glyph = match mode {
            CloseMode::Request => font_map::WindowClosed,
            CloseMode::Discard => font_map::Delete,
            CloseMode::Terminate => font_map::PowerOff,
            CloseMode::Kill => font_map::Skull,
        };
        let close_button: Element<'_, Message, Theme, Renderer> = button(
            container(
                text(close_glyph)
                    .font(MATERIAL_FAMILY)
                    .size(20)
                    .center()
                    .style(|_theme| text::Style { color: Some(Color::WHITE) }),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .width(Length::Fixed(36.0))
            .height(Length::Fixed(36.0)),
        )
        .padding(0)
        .on_press(Message::CloseSelected(mode))
        .style(move |_theme, status| {
            let bg = match (status, destructive) {
                (button::Status::Hovered, _) => Color::from_rgb(0.86, 0.20, 0.22),
                (button::Status::Pressed, _) => Color::from_rgb(0.74, 0.12, 0.14),
                (_, true) => Color::from_rgb(0.80, 0.12, 0.14),
                _ => Color::from_rgb(0.90, 0.32, 0.34),
            };
            button::Style {
                snap: true,
                background: Some(Background::Color(bg)),
                text_color: Color::WHITE,
                border: Border { radius: 6.0.into(), ..Default::default() },
                shadow: Shadow::default(),
            }
        })
        .into();

        let alignment_group = column![
            row![
                icon_button(
                    font_map::AlignVerticalTop,
                    press_action(SelectionAction::AlignTop)
                ),
                icon_button(
                    font_map::AlignVerticalCenter,
                    press_action(SelectionAction::AlignVerticalCenter)
                ),
                icon_button(
                    font_map::AlignVerticalBottom,
                    press_action(SelectionAction::AlignBottom)
                ),
            ]
            .spacing(2),
            row![
                icon_button(
                    font_map::AlignHorizontalLeft,
                    press_action(SelectionAction::AlignLeft)
                ),
                icon_button(
                    font_map::AlignHorizontalCenter,
                    press_action(SelectionAction::AlignHorizontalCenter)
                ),
                icon_button(
                    font_map::AlignHorizontalRight,
                    press_action(SelectionAction::AlignRight)
                ),
            ]
            .spacing(2),
        ]
        .spacing(2);

        // --- Distribute: vertical on top, horizontal below (mirroring alignment) ---
        let distribute_group = column![
            row![icon_button(
                font_map::VerticalDistribute,
                press_action(SelectionAction::DistributeVertical)
            ),]
            .spacing(2),
            row![icon_button(
                font_map::HorizontalDistribute,
                press_action(SelectionAction::DistributeHorizontal)
            ),]
            .spacing(2),
        ]
        .spacing(2);

        // --- Stack: vertical on top, horizontal below ---
        let stack_group = column![
            row![icon_button(
                font_map::AlignSpaceEven,
                press_action(SelectionAction::StackVertical)
            ),]
            .spacing(2),
            row![icon_button(
                font_map::AlignJustifySpaceEven,
                press_action(SelectionAction::StackHorizontal)
            ),]
            .spacing(2),
        ]
        .spacing(2);
        let separator = || {
            container(
                Space::new()
                    .width(Length::Fixed(1.0))
                    .height(Length::Fixed(58.0)), // was 20.0
            )
            .style(|_theme| container::Style {
                background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.1))),
                ..Default::default()
            })
        };

        // ... groups built as before ...
        let mut toolbar_left: Row<Message> = row![
            alignment_group,
            separator(),
            distribute_group,
            separator(),
            stack_group,
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        {
            if self.selection_count == 1 {
                toolbar_left = toolbar_left.push(
                    column![
                        row![icon_button(
                            font_map::FitPageHeight,
                            ExecuteScaleToFit(ScaleToFitOption{
                                max: true,
                                perceived: self.selection.alt_held,
                                vertical: true,
                                horizontal: false,
                            })
                        )],
                        row![icon_button(
                            font_map::FitPageWidth,
                            ExecuteScaleToFit(ScaleToFitOption{
                                max: true,
                                perceived: self.selection.alt_held,
                                vertical: false,
                                horizontal: true
                            })
                        )],
                    ]
                    .spacing(2),
                );
                toolbar_left = toolbar_left.push(
                    column![
                        row![icon_button(
                            font_map::AspectRatio,
                            ExecuteScaleToFit(ScaleToFitOption{
                                max: self.selection.shift_held,
                                perceived: self.selection.alt_held,
                                vertical: true,
                                horizontal: true,
                            })
                        )],
                    ]
                    .spacing(2),
                );
            }
        }

        // Destructive close action sits at the end of the toolbar, fenced off
        // from the layout tools by a separator.
        toolbar_left = toolbar_left.push(separator());
        toolbar_left = toolbar_left.push(
            mouse_area(close_button)
                .on_enter(Message::Hover(Some(TipKind::Close)))
                .on_exit(Message::Hover(None)),
        );

        // The commit panel — only present when toggles exist.
        let commit_panel: Element<'_, Message, Theme, Renderer> = if self.selection.has_any_toggle()
        {
            let glyph = if self.selection.alt_held {
                font_map::PublishedWithChanges // e.g., a "repeat" or "settings" icon
            } else {
                font_map::Commit // checkmark or play
            };

            let action = if self.selection.has_any_toggle() {
                let actions = self.selection.collect_actions();
                Some(Message::ExecuteSelection(actions, self.selection.alt_held))
            } else {
                None
            };

            let commit_button = button(text(glyph).font(MATERIAL_FAMILY).size(20).center().style(
                |_theme| text::Style {
                    color: Some(Color::WHITE),
                },
            ))
            .padding(0)
            .on_press_maybe(action)
            .style(|_theme, status| {
                let bg = match status {
                    button::Status::Hovered => Color::from_rgb(0.25, 0.50, 0.95),
                    button::Status::Pressed => Color::from_rgb(0.15, 0.40, 0.85),
                    _ => Color::from_rgb(0.30, 0.55, 1.0),
                };
                button::Style {
                    snap: true,
                    background: Some(Background::Color(bg)),
                    text_color: Color::WHITE,
                    border: Border {
                        radius: 6.0.into(),
                        ..Default::default()
                    },
                    shadow: Shadow::default(),
                }
            });

            // Wrap in fixed-size container so it animates in cleanly.
            let panel = container(commit_button)
                .width(Length::Fixed(48.0))
                .height(Length::Fixed(74.0)); // was 36.0
            mouse_area(panel)
                .on_enter(Message::Hover(Some(TipKind::Commit)))
                .on_exit(Message::Hover(None))
                .into()
        } else {
            Space::new()
                .width(Length::Fixed(0.0))
                .height(Length::Fixed(0.0))
                .into()
        };

        // Main row: toolbar on the left, commit panel anchored to the right.
        let toolbar = row![
            toolbar_left,
            // Pushes commit to the right edge.
            Space::new().width(Length::Fill).height(Length::Shrink),
            commit_panel,
        ]
        .align_y(Alignment::Center);

        container(toolbar)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(alignment::Vertical::Center)
            .padding(Padding {
                top: 0.0,
                right: 16.0,
                bottom: 0.0,
                left: 16.0,
            })
    }

    /// The description + live modifier state for the currently-hovered button,
    /// or `None` when nothing is hovered. The host pushes this into the separate
    /// tooltip surface. Recomputed each frame, so the text swaps as Alt/Shift are
    /// pressed while hovering.
    pub fn hovered_tip(&self) -> Option<(String, bool, bool)> {
        let kind = self.hovered?;
        let (alt, shift) = (self.selection.alt_held, self.selection.shift_held);
        let text = match kind {
            TipKind::Action(a) => describe_action(a, shift),
            TipKind::Close => describe_close(alt, shift).to_string(),
            TipKind::Commit => "Apply batched actions".to_string(),
        };
        Some((text, alt, shift))
    }

    pub fn handle_shift_click(&mut self, action: SelectionAction) {
        match category(action) {
            SelectionCategory::Align => {
                // Adding align: clear distribute and stack.
                self.selection.distribute_toggles.clear();
                self.selection.stack_toggle = None;
                // Toggle this align action.
                if !self.selection.align_toggles.insert(action) {
                    self.selection.align_toggles.remove(&action);
                }
            }
            SelectionCategory::Distribute => {
                self.selection.align_toggles.clear();
                self.selection.stack_toggle = None;
                if !self.selection.distribute_toggles.insert(action) {
                    self.selection.distribute_toggles.remove(&action);
                }
            }
            SelectionCategory::Stack => {
                self.selection.align_toggles.clear();
                self.selection.distribute_toggles.clear();
                // Stack: single-select. Clicking same one again clears it.
                if self.selection.stack_toggle == Some(action) {
                    self.selection.stack_toggle = None;
                } else {
                    self.selection.stack_toggle = Some(action);
                }
            }
            SelectionCategory::ScaleToFit => {
                self.selection.align_toggles.clear();
                self.selection.distribute_toggles.clear();
                self.selection.stack_toggle = None;
            }
        }
    }

    pub fn handle_plain_click(&mut self, action: SelectionAction) {
        if self.selection.has_any_toggle() {
            // Spec: "clicking without shift: if there is active toggle, stops it and no-op."
            self.selection.clear();
        } else {
            // No toggles active — immediate execution.
            // Defer to next update tick by emitting the execute message.
            // (Or call the handler directly here, but Tasks are cleaner.)
            // For now, just clear and let the handler see the action.
        }
    }
}

/// Identifies which toolbar button the pointer is over, so the host can render
/// the matching (modifier-aware) tooltip text in the separate tip surface.
#[derive(Debug, Clone, Copy)]
pub enum TipKind {
    Action(SelectionAction),
    Close,
    Commit,
}

/// Derive the tip identity from a button's press message.
fn tip_kind(action: &Message) -> Option<TipKind> {
    match action {
        Message::SelectionClicked(a) => Some(TipKind::Action(*a)),
        Message::ExecuteSelection(actions, _) => actions.first().map(|a| TipKind::Action(*a)),
        Message::ExecuteScaleToFit(o) => Some(TipKind::Action(SelectionAction::ScaleToFit(*o))),
        Message::CloseSelected(_) => Some(TipKind::Close),
        _ => None,
    }
}

/// Action description, swapped when Shift is held (Shift turns the layout
/// buttons into "add to batch" instead of execute-now).
fn describe_action(a: SelectionAction, shift: bool) -> String {
    use SelectionAction::*;
    let (plain, batched) = match a {
        AlignTop => ("Align top edges", "Add: align top"),
        AlignBottom => ("Align bottom edges", "Add: align bottom"),
        AlignVerticalCenter => ("Align vertical centers", "Add: align v-center"),
        AlignLeft => ("Align left edges", "Add: align left"),
        AlignHorizontalCenter => ("Align horizontal centers", "Add: align h-center"),
        AlignRight => ("Align right edges", "Add: align right"),
        DistributeVertical => ("Distribute vertically", "Add: distribute vertical"),
        DistributeHorizontal => ("Distribute horizontally", "Add: distribute horizontal"),
        StackVertical => ("Stack vertically (no gaps)", "Add: stack vertical"),
        StackHorizontal => ("Stack horizontally (no gaps)", "Add: stack horizontal"),
        ScaleToFit(o) => return describe_scale(o),
    };
    if shift { batched } else { plain }.to_string()
}

/// Scale-to-fit description; `perceived` (set from Alt at build time) swaps the
/// suffix.
fn describe_scale(o: ScaleToFitOption) -> String {
    let what = match (o.vertical, o.horizontal) {
        (true, true) => "Fit to aspect ratio",
        (true, false) => "Fit to page height",
        (false, true) => "Fit to page width",
        _ => "Scale to fit",
    };
    if o.perceived {
        format!("{what} (perceived size)")
    } else {
        what.to_string()
    }
}

/// Close-button description, escalating with Shift / Alt / Alt+Shift.
fn describe_close(alt: bool, shift: bool) -> &'static str {
    match CloseMode::from_modifiers(alt, shift) {
        CloseMode::Request => "Close window",
        CloseMode::Discard => "Close window, leave no placeholder",
        CloseMode::Terminate => "Terminate process (SIGTERM)",
        CloseMode::Kill => "Force kill (SIGKILL)",
    }
}
