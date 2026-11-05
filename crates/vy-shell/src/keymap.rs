//! Keys as the page sees them: USB HID usage codes (SPEC-A.2.1,
//! SPEC-10.3.1) from `winit`'s physical key codes, and modifier bits
//! (SPEC-A.5.1) from its modifier state. Only compiled with the window.

use vy_abi::modifier;
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};

/// The USB HID usage code of a physical key, or `None` for a key 1.0
/// does not name (media keys, international keys beyond `IntlBackslash`).
pub fn hid_code(key: PhysicalKey) -> Option<i32> {
    let PhysicalKey::Code(code) = key else {
        return None;
    };
    use KeyCode::*;
    let hid = match code {
        KeyA => 0x04,
        KeyB => 0x05,
        KeyC => 0x06,
        KeyD => 0x07,
        KeyE => 0x08,
        KeyF => 0x09,
        KeyG => 0x0A,
        KeyH => 0x0B,
        KeyI => 0x0C,
        KeyJ => 0x0D,
        KeyK => 0x0E,
        KeyL => 0x0F,
        KeyM => 0x10,
        KeyN => 0x11,
        KeyO => 0x12,
        KeyP => 0x13,
        KeyQ => 0x14,
        KeyR => 0x15,
        KeyS => 0x16,
        KeyT => 0x17,
        KeyU => 0x18,
        KeyV => 0x19,
        KeyW => 0x1A,
        KeyX => 0x1B,
        KeyY => 0x1C,
        KeyZ => 0x1D,
        Digit1 => 0x1E,
        Digit2 => 0x1F,
        Digit3 => 0x20,
        Digit4 => 0x21,
        Digit5 => 0x22,
        Digit6 => 0x23,
        Digit7 => 0x24,
        Digit8 => 0x25,
        Digit9 => 0x26,
        Digit0 => 0x27,
        Enter => 0x28,
        Escape => 0x29,
        Backspace => 0x2A,
        Tab => 0x2B,
        Space => 0x2C,
        Minus => 0x2D,
        Equal => 0x2E,
        BracketLeft => 0x2F,
        BracketRight => 0x30,
        Backslash => 0x31,
        Semicolon => 0x33,
        Quote => 0x34,
        Backquote => 0x35,
        Comma => 0x36,
        Period => 0x37,
        Slash => 0x38,
        CapsLock => 0x39,
        F1 => 0x3A,
        F2 => 0x3B,
        F3 => 0x3C,
        F4 => 0x3D,
        F5 => 0x3E,
        F6 => 0x3F,
        F7 => 0x40,
        F8 => 0x41,
        F9 => 0x42,
        F10 => 0x43,
        F11 => 0x44,
        F12 => 0x45,
        PrintScreen => 0x46,
        ScrollLock => 0x47,
        Pause => 0x48,
        Insert => 0x49,
        Home => 0x4A,
        PageUp => 0x4B,
        Delete => 0x4C,
        End => 0x4D,
        PageDown => 0x4E,
        ArrowRight => 0x4F,
        ArrowLeft => 0x50,
        ArrowDown => 0x51,
        ArrowUp => 0x52,
        NumLock => 0x53,
        NumpadDivide => 0x54,
        NumpadMultiply => 0x55,
        NumpadSubtract => 0x56,
        NumpadAdd => 0x57,
        NumpadEnter => 0x58,
        Numpad1 => 0x59,
        Numpad2 => 0x5A,
        Numpad3 => 0x5B,
        Numpad4 => 0x5C,
        Numpad5 => 0x5D,
        Numpad6 => 0x5E,
        Numpad7 => 0x5F,
        Numpad8 => 0x60,
        Numpad9 => 0x61,
        Numpad0 => 0x62,
        NumpadDecimal => 0x63,
        IntlBackslash => 0x64,
        ContextMenu => 0x65,
        ControlLeft => 0xE0,
        ShiftLeft => 0xE1,
        AltLeft => 0xE2,
        SuperLeft => 0xE3,
        ControlRight => 0xE4,
        ShiftRight => 0xE5,
        AltRight => 0xE6,
        SuperRight => 0xE7,
        _ => return None,
    };
    Some(hid)
}

/// Modifier bits of SPEC-A.5.1 from winit's state.
pub fn modifier_bits(state: ModifiersState) -> i32 {
    let mut bits = 0;
    if state.shift_key() {
        bits |= modifier::SHIFT;
    }
    if state.control_key() {
        bits |= modifier::CTRL;
    }
    if state.alt_key() {
        bits |= modifier::ALT;
    }
    if state.super_key() {
        bits |= modifier::META;
    }
    bits
}

/// True when a key pressed with `modifiers` (SPEC-A.5.1 bits) may commit
/// the text winit reports for it: no modifier, Shift, AltGr (which
/// Windows and X11 report as Ctrl+Alt: `@ { [ ] } \ $ |` on a Swedish
/// keyboard), and Option on macOS (`@` is Option+2 there). Ctrl alone and
/// Meta (Cmd, Win) are shortcut chords and commit nothing.
pub fn produces_text(modifiers: i32) -> bool {
    let ctrl = modifiers & modifier::CTRL != 0;
    let alt = modifiers & modifier::ALT != 0;
    if modifiers & modifier::META != 0 {
        return false;
    }
    match (ctrl, alt) {
        (false, false) | (true, true) => true,
        (false, true) => cfg!(target_os = "macos"),
        (true, false) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC-A.5.1: AltGr (Ctrl+Alt) and plain keys commit text; Ctrl and
    /// Meta chords do not; Alt alone only on macOS (Option).
    #[test]
    fn spec_a_5_1_altgr_commits_text() {
        assert!(produces_text(0));
        assert!(produces_text(modifier::SHIFT));
        assert!(produces_text(modifier::CTRL | modifier::ALT));
        assert!(produces_text(
            modifier::CTRL | modifier::ALT | modifier::SHIFT
        ));
        assert!(!produces_text(modifier::CTRL));
        assert!(!produces_text(modifier::META));
        assert!(!produces_text(modifier::META | modifier::ALT));
        assert_eq!(produces_text(modifier::ALT), cfg!(target_os = "macos"));
    }

    /// SPEC-10.3.1, SPEC-A.5.1: the codes the spec names and the shell's
    /// shortcuts use are the HID codes of their physical keys; the
    /// modifier bits are Shift 1, Ctrl 2, Alt 4, Meta 8.
    #[test]
    fn spec_10_3_1_keys_are_hid_codes() {
        let code = |k| hid_code(PhysicalKey::Code(k)).unwrap();
        assert_eq!(code(KeyCode::Tab), vy_abi::key::TAB);
        assert_eq!(code(KeyCode::Enter), vy_abi::key::ENTER);
        assert_eq!(code(KeyCode::Escape), vy_abi::key::ESCAPE);
        assert_eq!(code(KeyCode::ShiftLeft), vy_abi::key::SHIFT_LEFT);
        assert_eq!(code(KeyCode::ShiftRight), vy_abi::key::SHIFT_RIGHT);
        assert_eq!(code(KeyCode::KeyL), crate::shell::hid::L);
        assert_eq!(code(KeyCode::KeyT), crate::shell::hid::T);
        assert_eq!(code(KeyCode::KeyW), crate::shell::hid::W);
        assert_eq!(code(KeyCode::F5), crate::shell::hid::F5);
        assert_eq!(code(KeyCode::ArrowLeft), crate::shell::hid::LEFT);
        assert_eq!(code(KeyCode::ArrowRight), crate::shell::hid::RIGHT);
        assert_eq!(code(KeyCode::Equal), crate::shell::hid::EQUAL);
        assert_eq!(code(KeyCode::Minus), crate::shell::hid::MINUS);
        assert_eq!(code(KeyCode::Digit0), crate::shell::hid::DIGIT0);
        assert_eq!(code(KeyCode::NumpadAdd), crate::shell::hid::KP_PLUS);
        assert_eq!(code(KeyCode::Space), 0x2C);
        assert_eq!(code(KeyCode::PageDown), 78);
        assert_eq!(hid_code(PhysicalKey::Code(KeyCode::MediaPlayPause)), None);
        assert_eq!(
            hid_code(PhysicalKey::Unidentified(
                winit::keyboard::NativeKeyCode::Unidentified
            )),
            None
        );
        assert_eq!(modifier_bits(ModifiersState::empty()), 0);
        assert_eq!(
            modifier_bits(ModifiersState::SHIFT | ModifiersState::CONTROL),
            modifier::SHIFT | modifier::CTRL
        );
        assert_eq!(modifier_bits(ModifiersState::ALT), modifier::ALT);
        assert_eq!(modifier_bits(ModifiersState::SUPER), modifier::META);
    }
}
