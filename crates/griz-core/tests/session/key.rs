use super::*;
use griz_protocol::{KeyAction, KeyInput, KeyModifiers, TerminalKey};

fn key(key: TerminalKey) -> KeyInput {
    KeyInput {
        action: KeyAction::Press,
        key,
        physical: None,
        unshifted_codepoint: None,
        text: String::new(),
        modifiers: KeyModifiers::default(),
    }
}

#[test]
fn structured_key_uses_the_selected_live_terminals_current_modes_without_resizing() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let terminal_id = initial.active_terminal.unwrap();

    input(
        &mut stream,
        1,
        terminal_id,
        b"printf '\\033[?1h'; stty raw -echo; printf 'key-mode-%s\\r\\n' ready; dd bs=1 count=3 2>/dev/null | od -An -tx1; stty sane; printf '\\nkey-mode-%s\\n' done\r",
    );
    until(&mut stream, |event| contains(event, "key-mode-ready"));
    command(
        &mut stream,
        2,
        CoreCommand::Key {
            terminal_id,
            event: key(TerminalKey::Up),
        },
    );
    let encoded = view(until(&mut stream, |event| contains(event, "key-mode-done")));
    let rendered = text(&encoded.terminal.as_ref().unwrap().screen);
    assert!(rendered.contains("1b  4f  41"), "screen={rendered:?}");
    assert_eq!(encoded.active_terminal, Some(terminal_id));
    assert_eq!(
        (
            encoded.terminal.as_ref().unwrap().screen.cols,
            encoded.terminal.as_ref().unwrap().screen.rows
        ),
        (80, 24),
        "key input must not reuse or alter client screen dimensions"
    );
}

#[test]
fn ctrl_v_reaches_the_pty_under_pi_kitty_keyboard_flags() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let terminal_id = next_view(&mut stream).active_terminal.unwrap();
    input(
        &mut stream,
        1,
        terminal_id,
        b"printf '\\033[>7u'; stty raw -echo; printf 'pi-paste-%s\\r\\n' ready; dd bs=1 count=8 2>/dev/null | od -An -tx1; printf '\\033[<u'; stty sane; printf '\\npi-paste-%s\\n' done\r",
    );
    until(&mut stream, |event| contains(event, "pi-paste-ready"));
    let mut event = key(TerminalKey::Character);
    event.text = "v".into();
    event.physical = griz_protocol::PhysicalKey::new(41);
    event.unshifted_codepoint = Some('v');
    event.modifiers.ctrl = true;
    command(&mut stream, 2, CoreCommand::Key { terminal_id, event });
    let encoded = view(until(&mut stream, |event| contains(event, "pi-paste-done")));
    let rendered = text(&encoded.terminal.unwrap().screen);
    assert!(
        rendered.contains("1b  5b  31  31  38  3b  35  75"),
        "screen={rendered:?}"
    );
}

#[test]
fn structured_key_rejects_unselected_and_exited_targets_without_rerouting() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let first = initial.active_terminal.unwrap();
    command(
        &mut stream,
        1,
        CoreCommand::CreateTerminal {
            workspace_id: initial.active_workspace,
            cwd: None,
        },
    );
    let second = ack(&mut stream, 1).active_terminal.unwrap();

    command(
        &mut stream,
        2,
        CoreCommand::Key {
            terminal_id: first,
            event: key(TerminalKey::Enter),
        },
    );
    until(
        &mut stream,
        |event| matches!(event, Event::Error { serial: Some(2), message } if message.contains("not selected")),
    );
    assert_eq!(ack(&mut stream, 2).active_terminal, Some(second));

    input(&mut stream, 3, second, b"exit\r");
    until(
        &mut stream,
        |event| matches!(event, Event::View { state } if state.terminal.as_ref().is_some_and(|terminal| terminal.exited.is_some())),
    );
    command(
        &mut stream,
        4,
        CoreCommand::Key {
            terminal_id: second,
            event: key(TerminalKey::Enter),
        },
    );
    until(
        &mut stream,
        |event| matches!(event, Event::Error { serial: Some(4), message } if message.contains("has exited")),
    );
    assert_eq!(ack(&mut stream, 4).active_terminal, Some(second));
}
