//! The on-screen keyboard surface: a positional key grid (labels are US for now;
//! injection resolves each keycode through the live xkb layout, and per-layout
//! labels arrive via `SetLabels` in a later phase). Taps emit `OskMessage`s that
//! the compositor turns into injected key edges. Screen-space, so touch/pen taps
//! reach it through the emulated-pointer path (like the touch pane).
use iced_core::{Background, Border, Color, Element, Length, Theme};
use iced_widget::{button, column, container, row, text, Row, Space};
use compositor_support_iced_core_engine_base::{IcedUi, Renderer};
use compositor_monitor_selection_font_base::font::MATERIAL_FAMILY;
use compositor_monitor_selection_font_base::font_map;

type El<'a> = Element<'a, OskMessage, Theme, Renderer>;

/// The four sticky modifiers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModKind {
    Shift,
    Ctrl,
    Alt,
    Logo,
}

/// Messages between the OSK surface and the compositor. `Key`/`Mod`/`Globe`/`Close`
/// are surface → compositor (taps); `SetMods` is compositor → surface (highlight).
#[derive(Clone, Debug)]
pub enum OskMessage {
    /// Tap a key: inject this xkb keycode (evdev+8).
    Key(u32),
    /// Toggle a sticky modifier.
    Mod(ModKind),
    /// Cycle the active XKB language.
    Globe,
    /// Dismiss the keyboard.
    Close,
    /// Compositor → surface: reflect the live sticky-modifier state (highlight).
    SetMods { shift: bool, ctrl: bool, alt: bool, logo: bool },
    /// Compositor → surface: per-keycode labels for the active XKB layout (globe
    /// switch re-labels the keys with the correct script). Missing keys fall back to
    /// the built-in US labels.
    SetLabels(Vec<(u32, String)>),
    /// Compositor → surface: the active field's surrounding text (from the client's
    /// `set_surrounding_text`) to preview above the keys. A caret marks the cursor.
    SetPreview(String),
    /// Compositor → surface: briefly highlight the just-tapped key (`None` clears).
    SetFlash(Option<u32>),
}

#[derive(Default)]
pub struct Osk {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub logo: bool,
    /// Per-keycode labels pushed for the active layout.
    labels: Vec<(u32, String)>,
    /// Live preview of the focused field (from the client's surrounding text).
    preview: String,
    /// Keycode of a just-tapped key, highlighted briefly for tap feedback.
    flash: Option<u32>,
}

impl Osk {
    pub fn new() -> Self {
        Self::default()
    }

    fn label_for(&self, kc: u32, fallback: &str) -> String {
        self.labels
            .iter()
            .find(|(k, _)| *k == kc)
            .map(|(_, s)| s.clone())
            .unwrap_or_else(|| fallback.to_string())
    }
}

/// Ordered xkb keycodes of the character keys — the reconciler computes a label per
/// keycode via the live xkb layout and pushes them back as `SetLabels`.
pub fn char_keycodes() -> Vec<u32> {
    rows().into_iter().flatten().map(|k| k.0).collect()
}

/// One character key: xkb keycode + its unshifted/shifted labels (US placeholder).
struct K(u32, &'static str, &'static str);

/// The four character rows (numbers, QWERTY, ASDF, ZXCV), keyed by xkb keycode.
fn rows() -> [Vec<K>; 4] {
    [
        vec![K(10,"1","!"),K(11,"2","@"),K(12,"3","#"),K(13,"4","$"),K(14,"5","%"),K(15,"6","^"),K(16,"7","&"),K(17,"8","*"),K(18,"9","("),K(19,"0",")"),K(20,"-","_"),K(21,"=","+")],
        vec![K(24,"q","Q"),K(25,"w","W"),K(26,"e","E"),K(27,"r","R"),K(28,"t","T"),K(29,"y","Y"),K(30,"u","U"),K(31,"i","I"),K(32,"o","O"),K(33,"p","P"),K(34,"[","{"),K(35,"]","}")],
        vec![K(38,"a","A"),K(39,"s","S"),K(40,"d","D"),K(41,"f","F"),K(42,"g","G"),K(43,"h","H"),K(44,"j","J"),K(45,"k","K"),K(46,"l","L"),K(47,";",":"),K(48,"'","\"")],
        vec![K(52,"z","Z"),K(53,"x","X"),K(54,"c","C"),K(55,"v","V"),K(56,"b","B"),K(57,"n","N"),K(58,"m","M"),K(59,",","<"),K(60,".",">"),K(61,"/","?")],
    ]
}

/// A flat cell background/text style shared by every key. Reacts to `Pressed`
/// (and `Hovered`) so a tap flashes a brighter fill — immediate touch feedback.
fn cell_style(on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_t, s| {
        let base = if on {
            Color::from_rgba(0.30, 0.55, 1.0, 0.85)
        } else {
            Color::from_rgba(1.0, 1.0, 1.0, 0.08)
        };
        let bg = match s {
            button::Status::Pressed => Color::from_rgba(0.55, 0.78, 1.0, 0.98),
            button::Status::Hovered if on => Color::from_rgba(0.40, 0.65, 1.0, 0.92),
            button::Status::Hovered => Color::from_rgba(1.0, 1.0, 1.0, 0.16),
            _ => base,
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: Color::WHITE,
            border: Border { radius: 6.0.into(), ..Default::default() },
            ..Default::default()
        }
    }
}

/// A character key with a resolved label (pushed per-layout, else US fallback). `on`
/// highlights it (the brief press flash).
fn key<'a>(kc: u32, label: String, on: bool) -> El<'a> {
    button(text(label).size(18).center())
        .width(Length::FillPortion(2))
        .height(Length::Fill)
        .on_press(OskMessage::Key(kc))
        .style(cell_style(on))
        .into()
}

/// A labelled special key (backspace / enter / space / mods / globe / close). `mat`
/// = render the label in the Material icon font. `weight` = flex width portion.
fn special<'a>(label: &'a str, mat: bool, on: bool, weight: u16, msg: OskMessage) -> El<'a> {
    let mut t = text(label).size(if mat { 20 } else { 15 }).center();
    if mat {
        t = t.font(MATERIAL_FAMILY);
    }
    button(t)
        .width(Length::FillPortion(weight))
        .height(Length::Fill)
        .on_press(msg)
        .style(cell_style(on))
        .into()
}

impl IcedUi for Osk {
    type Message = OskMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            OskMessage::SetMods { shift, ctrl, alt, logo } => {
                self.shift = shift;
                self.ctrl = ctrl;
                self.alt = alt;
                self.logo = logo;
            }
            OskMessage::SetLabels(labels) => self.labels = labels,
            OskMessage::SetPreview(preview) => self.preview = preview,
            OskMessage::SetFlash(flash) => self.flash = flash,
            _ => {}
        }
    }

    fn view(&self) -> Element<'_, Self::Message, Theme, Renderer> {
        let mut col = column![].spacing(6).width(Length::Fill).height(Length::Fill);
        // Live preview of the focused field (surrounding text + a caret at the cursor).
        col = col.push(
            container(text(self.preview.clone()).size(15).color(Color::from_rgba(1.0, 1.0, 1.0, 0.85)))
                .width(Length::Fill)
                .height(Length::FillPortion(1))
                .padding([4, 8])
                .style(|_t| container::Style {
                    background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.06))),
                    border: Border { radius: 6.0.into(), ..Default::default() },
                    ..Default::default()
                }),
        );
        for r in rows() {
            let mut line: Vec<El> = r
                .iter()
                .map(|k| {
                    let fallback = if self.shift { k.2 } else { k.1 };
                    key(k.0, self.label_for(k.0, fallback), self.flash == Some(k.0))
                })
                .collect();
            // Backspace closes the top row (keycode 22).
            if line.len() == 12 {
                line.push(special("\u{232b}", false, self.flash == Some(22), 3, OskMessage::Key(22)));
            }
            col = col.push(Row::with_children(line).spacing(6).height(Length::Fill));
        }
        // Bottom row: Shift / Ctrl / Alt / Super / space / Globe / Enter / Close.
        let bottom: Vec<El> = vec![
            special("\u{21e7}", false, self.shift, 3, OskMessage::Mod(ModKind::Shift)),
            special("Ctrl", false, self.ctrl, 3, OskMessage::Mod(ModKind::Ctrl)),
            special("Alt", false, self.alt, 3, OskMessage::Mod(ModKind::Alt)),
            special("\u{2318}", false, self.logo, 3, OskMessage::Mod(ModKind::Logo)),
            special(font_map::Public, true, false, 3, OskMessage::Globe),
            special("space", false, self.flash == Some(65), 10, OskMessage::Key(65)),
            special("\u{23ce}", false, self.flash == Some(36), 4, OskMessage::Key(36)),
            special(font_map::Close, true, false, 3, OskMessage::Close),
        ];
        col = col.push(Row::with_children(bottom).spacing(6).height(Length::Fill));

        container(col)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(8)
            .style(|_t| container::Style {
                background: Some(Background::Color(Color::from_rgba(0.05, 0.06, 0.09, 0.92))),
                border: Border { radius: 10.0.into(), ..Default::default() },
                ..Default::default()
            })
            .into()
    }
}

// Silence unused-import lint if Space isn't used in a build variant.
#[allow(unused_imports)]
use iced_widget::Space as _Space;
