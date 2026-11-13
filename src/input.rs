//! Convert Rune's logical key event and per-dispatch metadata to bounded IPC.
//! Escape-sequence encoding belongs to the core-owned live Ghostty terminal.
use griz_protocol::{KeyAction, KeyInput, KeyModifiers, MAX_KEY_TEXT, PhysicalKey, TerminalKey};
use rune::{Key, KeyEvent, KeyPlatform, PhysicalKeyCode, Tx};

pub fn key(event: &KeyEvent, tx: &Tx) -> Option<KeyInput> {
    let modifiers = event.mods;
    // Native Command/Super shortcuts remain available to AppKit and Griz. In particular,
    // an unclaimed Cmd chord must never become terminal input.
    if modifiers.logo || (KeyPlatform::current() == KeyPlatform::Mac && modifiers.cmd) {
        return None;
    }

    let (key, text) = match &event.key {
        Key::Text(text) => (TerminalKey::Character, text.clone()),
        // Headless tests and scripts can express a non-platform Control chord this way.
        // Native primary-modifier commands were rejected above.
        Key::Command(character) => (TerminalKey::Character, character.to_string()),
        Key::Enter => (TerminalKey::Enter, String::new()),
        Key::Backspace => (TerminalKey::Backspace, String::new()),
        Key::Delete => (TerminalKey::Delete, String::new()),
        Key::Left => (TerminalKey::Left, String::new()),
        Key::Right => (TerminalKey::Right, String::new()),
        Key::Up => (TerminalKey::Up, String::new()),
        Key::Down => (TerminalKey::Down, String::new()),
        Key::Home => (TerminalKey::Home, String::new()),
        Key::End => (TerminalKey::End, String::new()),
        Key::PageUp => (TerminalKey::PageUp, String::new()),
        Key::PageDown => (TerminalKey::PageDown, String::new()),
        Key::Insert => (TerminalKey::Insert, String::new()),
        Key::PrintScreen => (TerminalKey::PrintScreen, String::new()),
        Key::ScrollLock => (TerminalKey::ScrollLock, String::new()),
        Key::Pause => (TerminalKey::Pause, String::new()),
        Key::Escape => (TerminalKey::Escape, String::new()),
        Key::Tab => (TerminalKey::Tab, String::new()),
        Key::Function(number @ 1..=24) => {
            (TerminalKey::Function { number: *number }, String::new())
        }
        Key::ContextMenu => (TerminalKey::ContextMenu, String::new()),
        // Modifier-only events are opt-in Rune events, not terminal text keys. Rune's logical
        // app callback does not deliver ordinary key releases, so none are fabricated here.
        Key::Modifier { .. } | Key::Function(_) => return None,
        _ => return None,
    };
    if text.len() > MAX_KEY_TEXT {
        return None;
    }

    let input = KeyInput {
        action: if tx.key_repeat() {
            KeyAction::Repeat
        } else {
            KeyAction::Press
        },
        key,
        physical: tx.physical_key().and_then(physical_key),
        unshifted_codepoint: tx.key_base(),
        text,
        modifiers: KeyModifiers {
            shift: modifiers.shift,
            alt: modifiers.alt,
            ctrl: modifiers.ctrl,
            super_key: modifiers.logo,
        },
    };
    input.validate().is_ok().then_some(input)
}

/// Map Rune/winit's physical code to the bundled Ghostty `GhosttyKey` value.
/// Unknown platform keys stay absent rather than being guessed from their labels.
fn physical_key(key: PhysicalKeyCode) -> Option<PhysicalKey> {
    use PhysicalKeyCode as K;
    let value = match key {
        K::Backquote => 1,
        K::Backslash => 2,
        K::BracketLeft => 3,
        K::BracketRight => 4,
        K::Comma => 5,
        K::Digit0 => 6,
        K::Digit1 => 7,
        K::Digit2 => 8,
        K::Digit3 => 9,
        K::Digit4 => 10,
        K::Digit5 => 11,
        K::Digit6 => 12,
        K::Digit7 => 13,
        K::Digit8 => 14,
        K::Digit9 => 15,
        K::Equal => 16,
        K::IntlBackslash => 17,
        K::IntlRo => 18,
        K::IntlYen => 19,
        K::KeyA => 20,
        K::KeyB => 21,
        K::KeyC => 22,
        K::KeyD => 23,
        K::KeyE => 24,
        K::KeyF => 25,
        K::KeyG => 26,
        K::KeyH => 27,
        K::KeyI => 28,
        K::KeyJ => 29,
        K::KeyK => 30,
        K::KeyL => 31,
        K::KeyM => 32,
        K::KeyN => 33,
        K::KeyO => 34,
        K::KeyP => 35,
        K::KeyQ => 36,
        K::KeyR => 37,
        K::KeyS => 38,
        K::KeyT => 39,
        K::KeyU => 40,
        K::KeyV => 41,
        K::KeyW => 42,
        K::KeyX => 43,
        K::KeyY => 44,
        K::KeyZ => 45,
        K::Minus => 46,
        K::Period => 47,
        K::Quote => 48,
        K::Semicolon => 49,
        K::Slash => 50,
        K::AltLeft => 51,
        K::AltRight => 52,
        K::Backspace => 53,
        K::CapsLock => 54,
        K::ContextMenu => 55,
        K::ControlLeft => 56,
        K::ControlRight => 57,
        K::Enter => 58,
        K::SuperLeft => 59,
        K::SuperRight => 60,
        K::ShiftLeft => 61,
        K::ShiftRight => 62,
        K::Space => 63,
        K::Tab => 64,
        K::Convert => 65,
        K::KanaMode => 66,
        K::NonConvert => 67,
        K::Delete => 68,
        K::End => 69,
        K::Help => 70,
        K::Home => 71,
        K::Insert => 72,
        K::PageDown => 73,
        K::PageUp => 74,
        K::ArrowDown => 75,
        K::ArrowLeft => 76,
        K::ArrowRight => 77,
        K::ArrowUp => 78,
        K::NumLock => 79,
        K::Numpad0 => 80,
        K::Numpad1 => 81,
        K::Numpad2 => 82,
        K::Numpad3 => 83,
        K::Numpad4 => 84,
        K::Numpad5 => 85,
        K::Numpad6 => 86,
        K::Numpad7 => 87,
        K::Numpad8 => 88,
        K::Numpad9 => 89,
        K::NumpadAdd => 90,
        K::NumpadBackspace => 91,
        K::NumpadClear => 92,
        K::NumpadClearEntry => 93,
        K::NumpadComma => 94,
        K::NumpadDecimal => 95,
        K::NumpadDivide => 96,
        K::NumpadEnter => 97,
        K::NumpadEqual => 98,
        K::NumpadMemoryAdd => 99,
        K::NumpadMemoryClear => 100,
        K::NumpadMemoryRecall => 101,
        K::NumpadMemoryStore => 102,
        K::NumpadMemorySubtract => 103,
        K::NumpadMultiply | K::NumpadStar => 104,
        K::NumpadParenLeft => 105,
        K::NumpadParenRight => 106,
        K::NumpadSubtract => 107,
        K::Escape => 120,
        K::F1 => 121,
        K::F2 => 122,
        K::F3 => 123,
        K::F4 => 124,
        K::F5 => 125,
        K::F6 => 126,
        K::F7 => 127,
        K::F8 => 128,
        K::F9 => 129,
        K::F10 => 130,
        K::F11 => 131,
        K::F12 => 132,
        K::F13 => 133,
        K::F14 => 134,
        K::F15 => 135,
        K::F16 => 136,
        K::F17 => 137,
        K::F18 => 138,
        K::F19 => 139,
        K::F20 => 140,
        K::F21 => 141,
        K::F22 => 142,
        K::F23 => 143,
        K::F24 => 144,
        K::F25 => 145,
        K::Fn => 146,
        K::FnLock => 147,
        K::PrintScreen => 148,
        K::ScrollLock => 149,
        K::Pause => 150,
        K::BrowserBack => 151,
        K::BrowserFavorites => 152,
        K::BrowserForward => 153,
        K::BrowserHome => 154,
        K::BrowserRefresh => 155,
        K::BrowserSearch => 156,
        K::BrowserStop => 157,
        K::Eject => 158,
        K::LaunchApp1 => 159,
        K::LaunchApp2 => 160,
        K::LaunchMail => 161,
        K::MediaPlayPause => 162,
        K::MediaSelect => 163,
        K::MediaStop => 164,
        K::MediaTrackNext => 165,
        K::MediaTrackPrevious => 166,
        K::Power => 167,
        K::Sleep => 168,
        K::AudioVolumeDown => 169,
        K::AudioVolumeMute => 170,
        K::AudioVolumeUp => 171,
        K::WakeUp => 172,
        K::Copy => 173,
        K::Cut => 174,
        K::Paste => 175,
        _ => return None,
    };
    PhysicalKey::new(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Mods, Tx};

    #[test]
    fn logical_named_function_unicode_and_native_shortcut_gates() {
        let tx = Tx::default();
        for named in [
            Key::Enter,
            Key::Backspace,
            Key::Delete,
            Key::Left,
            Key::Right,
            Key::Up,
            Key::Down,
            Key::Home,
            Key::End,
            Key::PageUp,
            Key::PageDown,
            Key::Insert,
            Key::PrintScreen,
            Key::ScrollLock,
            Key::Pause,
            Key::Escape,
            Key::Tab,
            Key::ContextMenu,
        ] {
            assert!(key(&named.into(), &tx).is_some());
        }
        for number in 1..=24 {
            assert!(key(&Key::Function(number).into(), &tx).is_some());
        }

        let unicode = key(&Key::Text("界".into()).into(), &tx).unwrap();
        assert_eq!(unicode.key, TerminalKey::Character);
        assert_eq!(unicode.text, "界");
        assert_eq!(unicode.action, KeyAction::Press);

        let function = key(&Key::Function(24).into(), &tx).unwrap();
        assert_eq!(function.key, TerminalKey::Function { number: 24 });
        assert!(key(&Key::Function(25).into(), &tx).is_none());

        let command = KeyEvent {
            key: Key::Text("x".into()),
            mods: Mods {
                cmd: true,
                logo: true,
                ..Mods::default()
            },
        };
        assert!(key(&command, &tx).is_none());
    }
}
