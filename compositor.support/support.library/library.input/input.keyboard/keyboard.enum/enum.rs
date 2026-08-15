use smithay::input::keyboard::{Keysym, keysyms::*};

/// Exhaustive mapping of all keyboard keys.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Key {
    A,B,C,D,E,F,G,H,I,J,K,L,M,N,O,P,Q,R,S,T,U,V,W,X,Y,Z,
    Num0,Num1,Num2,Num3,Num4,Num5,Num6,Num7,Num8,Num9,
    Up,Down,Left,Right,Super,Shift,Ctrl,Alt,
    Return,Space,Tab,Escape,Backspace,Delete,Insert,
    Home,End,PageUp,PageDown,PrintScreen,ScrollLock,Pause,CapsLock,NumLock,
    Grave,Minus,Equal,BracketLeft,BracketRight,BackSlash,Semicolon,Apostrophe,Comma,Period,Slash,
    Kp0,Kp1,Kp2,Kp3,Kp4,Kp5,Kp6,Kp7,Kp8,Kp9,
    KpAdd,KpSubtract,KpMultiply,KpDivide,KpEnter,KpDecimal,
    F1,F2,F3,F4,F5,F6,F7,F8,F9,F10,F11,F12,F13,F14,F15,F16,F17,F18,F19,F20,F21,F22,F23,F24,
    AudioPlay,AudioPause,AudioStop,AudioNext,AudioPrev,AudioMute,AudioRaiseVolume,AudioLowerVolume,AudioMicMute,
    MonBrightnessUp,MonBrightnessDown,
    SwitchVt1,SwitchVt2,SwitchVt3,SwitchVt4,SwitchVt5,SwitchVt6,
    SwitchVt7,SwitchVt8,SwitchVt9,SwitchVt10,SwitchVt11,SwitchVt12,
}

impl Key {
    pub fn from_keysym(sym: Keysym) -> Option<Self> {
        match sym.raw() {
            KEY_a|KEY_A => Some(Key::A), KEY_b|KEY_B => Some(Key::B), KEY_c|KEY_C => Some(Key::C), KEY_d|KEY_D => Some(Key::D), KEY_e|KEY_E => Some(Key::E),
            KEY_f|KEY_F => Some(Key::F), KEY_g|KEY_G => Some(Key::G), KEY_h|KEY_H => Some(Key::H), KEY_i|KEY_I => Some(Key::I), KEY_j|KEY_J => Some(Key::J),
            KEY_k|KEY_K => Some(Key::K), KEY_l|KEY_L => Some(Key::L), KEY_m|KEY_M => Some(Key::M), KEY_n|KEY_N => Some(Key::N), KEY_o|KEY_O => Some(Key::O),
            KEY_p|KEY_P => Some(Key::P), KEY_q|KEY_Q => Some(Key::Q), KEY_r|KEY_R => Some(Key::R), KEY_s|KEY_S => Some(Key::S), KEY_t|KEY_T => Some(Key::T),
            KEY_u|KEY_U => Some(Key::U), KEY_v|KEY_V => Some(Key::V), KEY_w|KEY_W => Some(Key::W), KEY_x|KEY_X => Some(Key::X), KEY_y|KEY_Y => Some(Key::Y), KEY_z|KEY_Z => Some(Key::Z),
            KEY_1|KEY_exclam => Some(Key::Num1), KEY_2|KEY_at => Some(Key::Num2), KEY_3|KEY_numbersign => Some(Key::Num3), KEY_4|KEY_dollar => Some(Key::Num4), KEY_5|KEY_percent => Some(Key::Num5),
            KEY_6|KEY_asciicircum => Some(Key::Num6), KEY_7|KEY_ampersand => Some(Key::Num7), KEY_8|KEY_asterisk => Some(Key::Num8), KEY_9|KEY_parenleft => Some(Key::Num9), KEY_0|KEY_parenright => Some(Key::Num0),
            KEY_Up => Some(Key::Up), KEY_Down => Some(Key::Down), KEY_Left => Some(Key::Left), KEY_Right => Some(Key::Right),
            KEY_Super_L|KEY_Super_R|KEY_Meta_L|KEY_Meta_R => Some(Key::Super), KEY_Shift_L|KEY_Shift_R => Some(Key::Shift), KEY_Control_L|KEY_Control_R => Some(Key::Ctrl), KEY_Alt_L|KEY_Alt_R => Some(Key::Alt),
            KEY_Return => Some(Key::Return), KEY_space => Some(Key::Space), KEY_Tab => Some(Key::Tab), KEY_Escape => Some(Key::Escape), KEY_BackSpace => Some(Key::Backspace),
            KEY_Delete => Some(Key::Delete), KEY_Insert => Some(Key::Insert), KEY_Home => Some(Key::Home), KEY_End => Some(Key::End),
            KEY_Page_Up => Some(Key::PageUp), KEY_Page_Down => Some(Key::PageDown), KEY_Print => Some(Key::PrintScreen), KEY_Scroll_Lock => Some(Key::ScrollLock), KEY_Pause => Some(Key::Pause), KEY_Caps_Lock => Some(Key::CapsLock), KEY_Num_Lock => Some(Key::NumLock),
            KEY_grave|KEY_asciitilde => Some(Key::Grave), KEY_minus|KEY_underscore => Some(Key::Minus), KEY_equal|KEY_plus => Some(Key::Equal),
            KEY_bracketleft|KEY_braceleft => Some(Key::BracketLeft), KEY_bracketright|KEY_braceright => Some(Key::BracketRight), KEY_backslash|KEY_bar => Some(Key::BackSlash),
            KEY_semicolon|KEY_colon => Some(Key::Semicolon), KEY_apostrophe|KEY_quotedbl => Some(Key::Apostrophe), KEY_comma|KEY_less => Some(Key::Comma), KEY_period|KEY_greater => Some(Key::Period), KEY_slash|KEY_question => Some(Key::Slash),
            KEY_KP_0 => Some(Key::Kp0), KEY_KP_1 => Some(Key::Kp1), KEY_KP_2 => Some(Key::Kp2), KEY_KP_3 => Some(Key::Kp3), KEY_KP_4 => Some(Key::Kp4), KEY_KP_5 => Some(Key::Kp5), KEY_KP_6 => Some(Key::Kp6), KEY_KP_7 => Some(Key::Kp7), KEY_KP_8 => Some(Key::Kp8), KEY_KP_9 => Some(Key::Kp9),
            KEY_KP_Add => Some(Key::KpAdd), KEY_KP_Subtract => Some(Key::KpSubtract), KEY_KP_Multiply => Some(Key::KpMultiply), KEY_KP_Divide => Some(Key::KpDivide), KEY_KP_Enter => Some(Key::KpEnter), KEY_KP_Decimal => Some(Key::KpDecimal),
            KEY_F1 => Some(Key::F1), KEY_F2 => Some(Key::F2), KEY_F3 => Some(Key::F3), KEY_F4 => Some(Key::F4), KEY_F5 => Some(Key::F5), KEY_F6 => Some(Key::F6), KEY_F7 => Some(Key::F7), KEY_F8 => Some(Key::F8), KEY_F9 => Some(Key::F9), KEY_F10 => Some(Key::F10), KEY_F11 => Some(Key::F11), KEY_F12 => Some(Key::F12),
            KEY_F13 => Some(Key::F13), KEY_F14 => Some(Key::F14), KEY_F15 => Some(Key::F15), KEY_F16 => Some(Key::F16), KEY_F17 => Some(Key::F17), KEY_F18 => Some(Key::F18), KEY_F19 => Some(Key::F19), KEY_F20 => Some(Key::F20), KEY_F21 => Some(Key::F21), KEY_F22 => Some(Key::F22), KEY_F23 => Some(Key::F23), KEY_F24 => Some(Key::F24),
            KEY_XF86AudioPlay => Some(Key::AudioPlay), KEY_XF86AudioPause => Some(Key::AudioPause), KEY_XF86AudioStop => Some(Key::AudioStop), KEY_XF86AudioNext => Some(Key::AudioNext), KEY_XF86AudioPrev => Some(Key::AudioPrev),
            KEY_XF86AudioMute => Some(Key::AudioMute), KEY_XF86AudioRaiseVolume => Some(Key::AudioRaiseVolume), KEY_XF86AudioLowerVolume => Some(Key::AudioLowerVolume), KEY_XF86AudioMicMute => Some(Key::AudioMicMute),
            KEY_XF86MonBrightnessUp => Some(Key::MonBrightnessUp), KEY_XF86MonBrightnessDown => Some(Key::MonBrightnessDown),
            KEY_XF86Switch_VT_1 => Some(Key::SwitchVt1), KEY_XF86Switch_VT_2 => Some(Key::SwitchVt2), KEY_XF86Switch_VT_3 => Some(Key::SwitchVt3), KEY_XF86Switch_VT_4 => Some(Key::SwitchVt4),
            KEY_XF86Switch_VT_5 => Some(Key::SwitchVt5), KEY_XF86Switch_VT_6 => Some(Key::SwitchVt6), KEY_XF86Switch_VT_7 => Some(Key::SwitchVt7), KEY_XF86Switch_VT_8 => Some(Key::SwitchVt8),
            KEY_XF86Switch_VT_9 => Some(Key::SwitchVt9), KEY_XF86Switch_VT_10 => Some(Key::SwitchVt10), KEY_XF86Switch_VT_11 => Some(Key::SwitchVt11), KEY_XF86Switch_VT_12 => Some(Key::SwitchVt12),
            _ => None,
        }
    }
    pub fn vt_number(self) -> Option<i32> {
        Some(match self {
            Key::SwitchVt1 => 1, Key::SwitchVt2 => 2, Key::SwitchVt3 => 3, Key::SwitchVt4 => 4,
            Key::SwitchVt5 => 5, Key::SwitchVt6 => 6, Key::SwitchVt7 => 7, Key::SwitchVt8 => 8,
            Key::SwitchVt9 => 9, Key::SwitchVt10 => 10, Key::SwitchVt11 => 11, Key::SwitchVt12 => 12,
            _ => return None,
        })
    }
}
