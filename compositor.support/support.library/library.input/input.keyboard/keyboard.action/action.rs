pub use compositor_support_library_input_keyboard_combo::KeyCombo;

pub struct ShortcutHandler<S> {
    pub combo: KeyCombo,
    pub action: Box<dyn Fn(&mut S) -> bool>,
}

/// Parse combinations like `shortcut!(Super + Shift + Right)`.
#[macro_export]
macro_rules! shortcut {
    ($k:ident) => {
        KeyCombo {
            modifiers: smithay::input::keyboard::ModifiersState::default(),
            key: Some(Key::$k),
        }
    };
    ($m:ident + $($rest:tt)+) => {{
        let mut combo = shortcut!($($rest)+);
        match stringify!($m) {
            "Super" | "Logo" => combo.modifiers.logo = true,
            "Ctrl" | "Control" => combo.modifiers.ctrl = true,
            "Alt" => combo.modifiers.alt = true,
            "Shift" => combo.modifiers.shift = true,
            _ => panic!(concat!("Unknown modifier: ", stringify!($m))),
        }
        combo
    }};
}
