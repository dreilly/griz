use super::*;
use griz_protocol::MAX_INPUT;

fn paste_confirmation(stream: &mut UnixStream, serial: u64) -> (u64, u64, u32, u32) {
    let event = until(
        stream,
        |event| matches!(event, Event::PasteConfirmationRequired { serial: event_serial, .. } if *event_serial == serial),
    );
    let Event::PasteConfirmationRequired {
        paste_id,
        terminal_id,
        byte_count,
        line_count,
        ..
    } = event
    else {
        unreachable!()
    };
    (paste_id, terminal_id, byte_count, line_count)
}

fn command_error(stream: &mut UnixStream, serial: u64) -> String {
    let event = until(
        stream,
        |event| matches!(event, Event::Error { serial: Some(event_serial), .. } if *event_serial == serial),
    );
    let Event::Error { message, .. } = event else {
        unreachable!()
    };
    message
}

#[test]
fn unsafe_paste_is_server_held_one_use_and_cancel_writes_nothing() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let terminal_id = initial.active_terminal.unwrap();

    command(
        &mut stream,
        1,
        CoreCommand::Paste {
            terminal_id,
            text: String::new(),
        },
    );
    assert_eq!(ack(&mut stream, 1).ack, 1);

    let text = "printf 'PASTE-CONFIRMED\\n'\n";
    command(
        &mut stream,
        2,
        CoreCommand::Paste {
            terminal_id,
            text: text.into(),
        },
    );
    let (paste_id, target, bytes, lines) = paste_confirmation(&mut stream, 2);
    assert_eq!(target, terminal_id);
    assert_eq!(bytes as usize, text.len());
    assert_eq!(lines, 2);
    command(&mut stream, 3, CoreCommand::ConfirmPaste { paste_id });
    until(&mut stream, |event| contains(event, "PASTE-CONFIRMED"));
    command(&mut stream, 4, CoreCommand::ConfirmPaste { paste_id });
    assert!(command_error(&mut stream, 4).contains("stale paste confirmation"));

    let cancelled = "printf 'MUST-NOT-APPEAR\\n'\n";
    command(
        &mut stream,
        5,
        CoreCommand::Paste {
            terminal_id,
            text: cancelled.into(),
        },
    );
    let (cancel_id, ..) = paste_confirmation(&mut stream, 5);
    command(
        &mut stream,
        6,
        CoreCommand::CancelPaste {
            paste_id: cancel_id,
        },
    );
    ack(&mut stream, 6);
    input(&mut stream, 7, terminal_id, b"printf 'AFTER-CANCEL\\n'\r");
    let event = until(&mut stream, |event| contains(event, "AFTER-CANCEL"));
    assert!(!contains(&event, "MUST-NOT-APPEAR"));
}

#[test]
fn a_new_paste_replaces_only_that_clients_pending_token() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let terminal_id = next_view(&mut stream).active_terminal.unwrap();
    for (serial, text) in [(1, "old\n"), (2, "new\n")] {
        command(
            &mut stream,
            serial,
            CoreCommand::Paste {
                terminal_id,
                text: text.into(),
            },
        );
    }
    let (new_id, ..) = paste_confirmation(&mut stream, 2);
    command(
        &mut stream,
        3,
        CoreCommand::ConfirmPaste {
            paste_id: new_id.saturating_sub(1),
        },
    );
    assert!(command_error(&mut stream, 3).contains("stale paste confirmation"));
    command(
        &mut stream,
        4,
        CoreCommand::CancelPaste { paste_id: new_id },
    );
    ack(&mut stream, 4);
}

#[test]
fn paste_validates_selected_target_utf8_size_and_client_binding() {
    let service = Service::start();
    let mut first = service.attach(80, 24);
    let mut second = service.attach(80, 24);
    let initial = next_view(&mut first);
    next_view(&mut second);
    let terminal_id = initial.active_terminal.unwrap();

    command(
        &mut first,
        1,
        CoreCommand::Paste {
            terminal_id,
            text: "printf 'FIRST-CLIENT\\n'\n".into(),
        },
    );
    let (first_paste, ..) = paste_confirmation(&mut first, 1);
    command(
        &mut second,
        1,
        CoreCommand::ConfirmPaste {
            paste_id: first_paste,
        },
    );
    assert!(command_error(&mut second, 1).contains("stale paste confirmation"));

    command(
        &mut first,
        2,
        CoreCommand::ConfirmPaste {
            paste_id: first_paste,
        },
    );
    until(&mut first, |event| contains(event, "FIRST-CLIENT"));

    let large_unicode = format!("{}\n", "界".repeat((MAX_INPUT - 1) / "界".len()));
    assert_eq!(large_unicode.len(), MAX_INPUT);
    command(
        &mut first,
        3,
        CoreCommand::Paste {
            terminal_id,
            text: large_unicode,
        },
    );
    let (large_id, _, byte_count, _) = paste_confirmation(&mut first, 3);
    assert_eq!(byte_count as usize, MAX_INPUT);
    command(
        &mut first,
        4,
        CoreCommand::CancelPaste { paste_id: large_id },
    );
    ack(&mut first, 4);

    command(
        &mut first,
        5,
        CoreCommand::Paste {
            terminal_id,
            text: "界".repeat(MAX_INPUT / "界".len() + 1),
        },
    );
    assert!(command_error(&mut first, 5).contains("exceeds"));

    command(
        &mut first,
        6,
        CoreCommand::CreateTerminal {
            workspace_id: initial.active_workspace,
            cwd: None,
        },
    );
    let created = ack(&mut first, 6);
    let other_id = created.active_terminal.unwrap();
    command(
        &mut first,
        7,
        CoreCommand::Paste {
            terminal_id,
            text: "not selected".into(),
        },
    );
    assert!(command_error(&mut first, 7).contains("not selected"));

    command(&mut first, 8, CoreCommand::SelectTerminal { terminal_id });
    ack(&mut first, 8);
    command(
        &mut first,
        9,
        CoreCommand::Paste {
            terminal_id,
            text: "printf 'NEVER-REROUTE\\n'\n".into(),
        },
    );
    let (stale, ..) = paste_confirmation(&mut first, 9);
    command(
        &mut first,
        10,
        CoreCommand::SelectTerminal {
            terminal_id: other_id,
        },
    );
    ack(&mut first, 10);
    command(
        &mut first,
        11,
        CoreCommand::ConfirmPaste { paste_id: stale },
    );
    assert!(command_error(&mut first, 11).contains("stale paste confirmation"));
}

#[test]
fn bracketed_mode_avoids_prompt_and_confirmation_reencodes_current_modes() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let terminal_id = initial.active_terminal.unwrap();

    input(&mut stream, 1, terminal_id, b"printf '\\033[?2004h'\r");
    until(
        &mut stream,
        |event| matches!(event, Event::View { state } if state.ack >= 1 && state.terminal.as_ref().is_some_and(|terminal| terminal.screen.bracketed_paste)),
    );
    command(
        &mut stream,
        2,
        CoreCommand::Paste {
            terminal_id,
            text: "safe\nmultiline".into(),
        },
    );
    let event = until(&mut stream, |event| {
        matches!(event, Event::PasteConfirmationRequired { serial: 2, .. })
            || matches!(event, Event::View { state } if state.ack >= 2)
    });
    assert!(
        matches!(event, Event::View { .. }),
        "bracketed paste prompted: {event:?}"
    );

    input(&mut stream, 3, terminal_id, b"\x03printf '\\033[?2004l'\r");
    until(
        &mut stream,
        |event| matches!(event, Event::View { state } if state.ack >= 3 && state.terminal.as_ref().is_some_and(|terminal| !terminal.screen.bracketed_paste)),
    );
    command(
        &mut stream,
        4,
        CoreCommand::Paste {
            terminal_id,
            text: "mode\nchanges".into(),
        },
    );
    let (paste_id, ..) = paste_confirmation(&mut stream, 4);
    input(&mut stream, 5, terminal_id, b"printf '\\033[?2004h'\r");
    until(
        &mut stream,
        |event| matches!(event, Event::View { state } if state.ack >= 5 && state.terminal.as_ref().is_some_and(|terminal| terminal.screen.bracketed_paste)),
    );
    command(&mut stream, 6, CoreCommand::ConfirmPaste { paste_id });
    assert_eq!(ack(&mut stream, 6).ack, 6);
}

#[test]
fn shell_replacement_and_disconnect_invalidate_pending_paste() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let terminal_id = initial.active_terminal.unwrap();

    command(
        &mut stream,
        1,
        CoreCommand::Paste {
            terminal_id,
            text: "exit\n".into(),
        },
    );
    let (paste_id, ..) = paste_confirmation(&mut stream, 1);
    input(&mut stream, 2, terminal_id, b"exit\r");
    until(
        &mut stream,
        |event| matches!(event, Event::View { state } if state.terminal.as_ref().is_some_and(|terminal| terminal.exited.is_some())),
    );
    command(&mut stream, 3, CoreCommand::StartShell { terminal_id });
    ack(&mut stream, 3);
    command(&mut stream, 4, CoreCommand::ConfirmPaste { paste_id });
    assert!(command_error(&mut stream, 4).contains("stale paste confirmation"));

    command(
        &mut stream,
        5,
        CoreCommand::Paste {
            terminal_id,
            text: "printf 'DISCONNECTED\\n'\n".into(),
        },
    );
    let (disconnected_id, ..) = paste_confirmation(&mut stream, 5);
    drop(stream);

    let mut reattached = service.attach(80, 24);
    next_view(&mut reattached);
    command(
        &mut reattached,
        1,
        CoreCommand::ConfirmPaste {
            paste_id: disconnected_id,
        },
    );
    assert!(command_error(&mut reattached, 1).contains("stale paste confirmation"));
}
