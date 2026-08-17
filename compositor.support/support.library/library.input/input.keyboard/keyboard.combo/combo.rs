use compositor_support_library_input_keyboard_enum::Key;
use smithay::input::keyboard::ModifiersState;

/// Represents an exact required combination of modifiers and a pressed key.
pub struct KeyCombo {
    pub modifiers: ModifiersState,
    pub key: Option<Key>,
}

impl KeyCombo {
    /// Compares the required combo against the current input event.
    pub fn matches(&self, modifiers: &ModifiersState, key: Option<Key>) -> bool {
        let modifier_match = self.modifiers.logo == modifiers.logo
            && self.modifiers.shift == modifiers.shift
            && self.modifiers.ctrl == modifiers.ctrl
            && self.modifiers.alt == modifiers.alt;

        if !modifier_match {
            return false;
        }

        if self.key.is_none() {
            if !key.is_none() {
                match key.unwrap() {
                    Key::Super => { if self.modifiers.logo { return true; } }
                    Key::Shift => { if self.modifiers.shift { return true; } }
                    Key::Ctrl => { if self.modifiers.ctrl { return true; } }
                    Key::Alt => { if self.modifiers.alt { return true; } }
                    _ => return false,
                }
                return false;
            }
        } else {
            if key.is_none() {
                return false;
            }
            if self.key.unwrap() != key.unwrap() {
                return false;
            }
        }

        return true;
    }
}
