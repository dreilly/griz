//! Basic xterm-compatible input. Keyboard-protocol extensions are deliberately not advertised.
use rune::{Key, KeyEvent};

pub fn encode(event: &KeyEvent, application_cursor: bool) -> Option<Vec<u8>> {
    let m = event.mods;
    // Let the platform handle Command shortcuts (Quit, Close, Paste, etc.).
    if m.logo || (cfg!(target_os = "macos") && m.cmd) {
        return None;
    }
    let ctrl_text = match &event.key {
        Key::Text(t) if m.ctrl && !m.alt => t.chars().next().filter(|_| t.chars().count() == 1),
        Key::Command(c) if m.ctrl => Some(*c),
        _ => None,
    };
    if let Some(c) = ctrl_text {
        let c = c.to_ascii_uppercase();
        if ('@'..='_').contains(&c) || c == ' ' {
            return Some(vec![(c as u8) & 0x1f]);
        }
        if c == '?' {
            return Some(vec![0x7f]);
        }
    }
    let modifier = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.ctrl);
    let arrow = match event.key {
        Key::Up => Some('A'),
        Key::Down => Some('B'),
        Key::Right => Some('C'),
        Key::Left => Some('D'),
        Key::Home => Some('H'),
        Key::End => Some('F'),
        _ => None,
    };
    if let Some(final_) = arrow {
        return Some(if modifier > 1 {
            format!("\x1b[1;{modifier}{final_}").into_bytes()
        } else {
            format!("\x1b{}{final_}", if application_cursor { 'O' } else { '[' }).into_bytes()
        });
    }
    let numbered = match event.key {
        Key::Insert => Some(2),
        Key::Delete => Some(3),
        Key::PageUp => Some(5),
        Key::PageDown => Some(6),
        _ => None,
    };
    if let Some(n) = numbered {
        return Some(
            if modifier > 1 {
                format!("\x1b[{n};{modifier}~")
            } else {
                format!("\x1b[{n}~")
            }
            .into_bytes(),
        );
    }
    let mut bytes = match &event.key {
        Key::Text(t) => t.as_bytes().to_vec(),
        Key::Enter => vec![b'\r'],
        Key::Tab if m.shift => b"\x1b[Z".to_vec(),
        Key::Tab => vec![b'\t'],
        Key::Backspace => vec![0x7f],
        Key::Escape => vec![0x1b],
        Key::Function(n @ 1..=4) => format!("\x1bO{}", (b'P' + n - 1) as char).into_bytes(),
        Key::Function(n @ 5..=12) => {
            let codes = [15, 17, 18, 19, 20, 21, 23, 24];
            format!("\x1b[{}~", codes[(n - 5) as usize]).into_bytes()
        }
        _ => return None,
    };
    // Option-generated Unicode is already text on macOS. Elsewhere Alt is Meta.
    if m.alt && !cfg!(target_os = "macos") {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    // Don't allow clipboard text to terminate bracketed paste or inject escape sequences.
    let text = text
        .replace('\x1b', "")
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    if bracketed {
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.replace('\n', "\r").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::Mods;
    #[test]
    fn common_keys_and_cursor_mode() {
        assert_eq!(encode(&Key::Enter.into(), false), Some(vec![b'\r']));
        assert_eq!(encode(&Key::Up.into(), false).unwrap(), b"\x1b[A");
        assert_eq!(encode(&Key::Up.into(), true).unwrap(), b"\x1bOA");
        let ctrl_c = KeyEvent {
            key: Key::Text("c".into()),
            mods: Mods {
                ctrl: true,
                ..Mods::default()
            },
        };
        assert_eq!(encode(&ctrl_c, false), Some(vec![3]));
    }
    #[test]
    fn bracketed_paste_cannot_be_escaped_by_clipboard_contents() {
        assert_eq!(paste("a\r\nb\x1b", true), b"\x1b[200~a\nb\x1b[201~");
    }
}
