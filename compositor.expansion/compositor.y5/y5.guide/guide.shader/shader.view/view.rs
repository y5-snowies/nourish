//! The inline shader editor: this world's background variables, adjustable
//! against the LIVE desktop.
//!
//! It exists because the Settings World tab's preview is a 320px wgpu pane that
//! cannot run a multipass graph at all — so for exactly the shaders with the most
//! to tune, the only honest preview is the desktop itself. A small screen-space
//! panel over it is that preview.
//!
//! Variables ONLY. No bundle picker and no preview pane: everything here writes
//! one float into one param slot, which the next frame's push carries. Nothing it
//! can do rebuilds the pipeline, which is why there is no debounce anywhere in
//! this path — the expensive edit simply is not offered.
//!
//! Styled inline in the guide's dark-translucent idiom rather than through the
//! settings `style`/`control` crates: those live in `compositor.extension`, and a
//! `compositor.y5` surface must not reach across for presentation.

use compositor_configurator_settings_surface_message::message::{ShaderProp, ShaderPropKind};
use compositor_monitor_selection_font_base::font::MATERIAL_FAMILY;
use compositor_monitor_selection_font_base::font_map;
use compositor_support_iced_core_engine_base::{IcedUi, Renderer};
use iced_core::{Alignment, Background, Border, Color, Element, Length, Padding, Theme};
use iced_widget::{button, column, container, row, scrollable, slider, text, toggler};

type El<'a> = Element<'a, ShaderMessage, Theme, Renderer>;

const ACCENT: Color = Color::from_rgba(0.27, 0.78, 0.88, 1.0);
const MUTED: Color = Color::from_rgba(0.62, 0.68, 0.74, 1.0);
const PANEL: Color = Color::from_rgba(0.06, 0.07, 0.10, 0.94);

/// What the panel shows, pushed in whole by the reconciler.
///
/// A snapshot rather than a set of incremental edits: the values it displays are
/// owned by the world's `Two` slot and can move without this panel's involvement
/// (the Settings tab edits the same slot, and selecting another bundle replaces
/// the list outright). Re-pushing the resolved state is the only shape that
/// cannot drift from what is running.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderSnapshot {
    /// The bundle name, or empty for the built-in parallax.
    pub name: String,
    /// One compact line of what the bundle is — ownership and placement. Empty
    /// when there is no graph to describe.
    pub about: String,
    pub props: Vec<ShaderProp>,
}

#[derive(Clone, Debug)]
pub enum ShaderMessage {
    /// Push the resolved state in (compositor → surface). Never forwarded.
    Sync(ShaderSnapshot),
    /// One variable moved: the FULL name/value list, as the settings panel sends
    /// it, so the receiver needs no notion of which one changed.
    SetParams(Vec<(String, f32)>),
    Close,
}

#[derive(Default)]
pub struct ShaderEditor {
    pub snapshot: ShaderSnapshot,
}

impl IcedUi for ShaderEditor {
    type Message = ShaderMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            ShaderMessage::Sync(s) => self.snapshot = s,
            // Mirror locally so the control tracks the drag without waiting for
            // the next reconciler push — one frame of lag on a slider reads as
            // the control fighting the pointer.
            ShaderMessage::SetParams(values) => {
                for (name, v) in values {
                    if let Some(p) = self.snapshot.props.iter_mut().find(|p| p.name == name) {
                        p.value = v;
                    }
                }
            }
            ShaderMessage::Close => {}
        }
    }

    fn view(&self) -> Element<'_, Self::Message, Theme, Renderer> {
        let s = &self.snapshot;
        let title = match s.name.is_empty() {
            true => "Built-in parallax".to_string(),
            false => s.name.clone(),
        };
        let close = button(
            text(font_map::Close).font(MATERIAL_FAMILY).size(14).center(),
        )
        .padding(Padding::from([2, 6]))
        .on_press(ShaderMessage::Close)
        .style(|_t: &Theme, _s| button::Style {
            background: None,
            text_color: MUTED,
            ..Default::default()
        });
        let mut head = column![
            row![
                text(title).size(13).width(Length::Fill),
                close,
            ].align_y(Alignment::Center),
        ].spacing(2);
        if !s.about.is_empty() {
            head = head.push(text(s.about.clone()).size(10).color(MUTED));
        }
        let body: El<'_> = match s.props.is_empty() {
            true => text("This shader exposes no variables.").size(11).color(MUTED).into(),
            false => {
                let mut col = column![].spacing(8);
                let mut shown: Option<&str> = None;
                for p in &s.props {
                    let group = (!p.group.is_empty()).then_some(p.group.as_str());
                    if group.is_some() && group != shown {
                        col = col.push(text(p.group.to_uppercase()).size(9).color(MUTED));
                    }
                    shown = group;
                    col = col.push(variable(p, &s.props));
                }
                scrollable(col).height(Length::Fill).into()
            }
        };
        container(column![head, body].spacing(12))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(14)
            .style(|_t| container::Style {
                background: Some(Background::Color(PANEL)),
                border: Border {
                    color: Color::from_rgba(1.0, 1.0, 1.0, 0.14),
                    width: 1.0,
                    radius: 12.0.into(),
                },
                ..Default::default()
            })
            .into()
    }
}

/// One variable's row. Same control kinds as the Settings World tab, and for the
/// same reason: the kind is how the SHADER asked to be edited, so the two panels
/// showing it differently would make one of them wrong.
fn variable<'a>(p: &ShaderProp, all: &[ShaderProp]) -> El<'a> {
    let base: Vec<(String, f32)> = all.iter().map(|q| (q.name.clone(), q.value)).collect();
    let name = p.name.clone();
    let set = move |b: Vec<(String, f32)>, n: String| {
        move |v: f32| {
            ShaderMessage::SetParams(
                b.iter().map(|(k, x)| (k.clone(), if *k == n { v } else { *x })).collect(),
            )
        }
    };
    let control: El<'a> = match p.kind {
        ShaderPropKind::Float | ShaderPropKind::Color => {
            slider(p.min..=p.max, p.value, set(base.clone(), name.clone()))
                .step(((p.max - p.min) / 100.0).max(0.0001))
                .width(Length::Fixed(150.0))
                .into()
        }
        ShaderPropKind::Int => slider(p.min..=p.max, p.value.round(), set(base.clone(), name.clone()))
            .step(1.0)
            .width(Length::Fixed(150.0))
            .into(),
        ShaderPropKind::Bool => {
            let send = set(base.clone(), name.clone());
            toggler(p.value > 0.5)
                .on_toggle(move |on| send(if on { 1.0 } else { 0.0 }))
                .into()
        }
        ShaderPropKind::Choice => {
            let selected = p.value.round().max(0.0) as usize;
            let mut r = row![].spacing(4);
            for (i, label) in p.choices.iter().enumerate() {
                let send = set(base.clone(), name.clone());
                let on = i == selected;
                r = r.push(
                    button(text(label.clone()).size(10))
                        .padding(Padding::from([3, 8]))
                        .on_press(send(i as f32))
                        .style(move |_t: &Theme, _s| button::Style {
                            background: Some(Background::Color(Color::from_rgba(
                                1.0, 1.0, 1.0, if on { 0.12 } else { 0.0 },
                            ))),
                            text_color: if on { ACCENT } else { MUTED },
                            border: Border {
                                color: Color::from_rgba(1.0, 1.0, 1.0, 0.14),
                                width: 1.0,
                                radius: 6.0.into(),
                            },
                            ..Default::default()
                        }),
                );
            }
            r.into()
        }
    };
    let value: El<'a> = match p.kind {
        ShaderPropKind::Choice => column![].width(Length::Fixed(40.0)).into(),
        ShaderPropKind::Int => text(format!("{}", p.value.round() as i32))
            .size(11).color(ACCENT).width(Length::Fixed(40.0)).into(),
        _ => text(format!("{:.2}", p.value))
            .size(11).color(ACCENT).width(Length::Fixed(40.0)).into(),
    };
    row![text(p.label.clone()).size(11).width(Length::Fill), control, value]
        .align_y(Alignment::Center)
        .spacing(8)
        .into()
}
