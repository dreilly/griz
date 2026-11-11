use super::*;

#[test]
fn same_cell_pointer_jitter_reaches_the_program_as_a_click_not_a_drag() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let terminal_id = next_view(&mut stream).active_terminal.unwrap();
    input(
        &mut stream, 1, terminal_id,
        b"printf '\\033[?1003h\\033[?1006h'; stty raw -echo; dd bs=1 count=18 2>/dev/null | od -An -tx1; stty sane; printf '\\nmouse-click-complete\\n'\r",
    );
    until(&mut stream, |event| {
        matches!(event, Event::View { state }
        if state.terminal.as_ref().is_some_and(|t| t.screen.mouse_tracking))
    });
    for (serial, action, x, y) in [
        (2, MouseAction::Press, 20.0, 24.0),
        (3, MouseAction::Motion, 20.5, 24.5),
        (4, MouseAction::Release, 20.5, 24.5),
    ] {
        command(
            &mut stream,
            serial,
            CoreCommand::Mouse {
                terminal_id,
                event: MouseInput {
                    action,
                    button: Some(MouseButton::Left),
                    x,
                    y,
                    cell_width: 8,
                    cell_height: 16,
                    shift: false,
                    alt: false,
                    ctrl: false,
                },
            },
        );
    }
    let clicked = view(until(&mut stream, |event| {
        contains(event, "mouse-click-complete")
    }));
    let screen = text(&clicked.terminal.unwrap().screen);
    let words = screen.split_whitespace().collect::<Vec<_>>();
    assert!(
        words.windows(18).any(|window| window
            == [
                "1b", "5b", "3c", "30", "3b", "33", "3b", "32", "4d", "1b", "5b", "3c", "30", "3b",
                "33", "3b", "32", "6d",
            ]),
        "{screen}"
    );
}

#[test]
fn mouse_is_encoded_by_the_addressed_vt_and_routed_to_its_pty() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let workspace_id = initial.active_workspace;
    let first = initial.active_terminal.unwrap();

    input(
        &mut stream,
        1,
        first,
        b"printf '\\033[?1000h\\033[?1006h'; stty raw -echo; dd bs=1 count=9 2>/dev/null | od -An -tx1; stty sane; printf '\\nmouse-routed\\n'\r",
    );
    until(
        &mut stream,
        |event| matches!(event, Event::View { state } if state.terminal.as_ref().is_some_and(|terminal| terminal.screen.mouse_tracking)),
    );

    command(
        &mut stream,
        2,
        CoreCommand::CreateTerminal {
            workspace_id,
            cwd: None,
        },
    );
    let second = ack(&mut stream, 2).active_terminal.unwrap();

    command(
        &mut stream,
        3,
        CoreCommand::Mouse {
            terminal_id: first,
            event: MouseInput {
                action: MouseAction::Press,
                button: Some(MouseButton::Left),
                x: 8.0,
                y: 16.0,
                cell_width: 8,
                cell_height: 16,
                shift: false,
                alt: false,
                ctrl: false,
            },
        },
    );
    assert_eq!(ack(&mut stream, 3).active_terminal, Some(second));

    command(
        &mut stream,
        4,
        CoreCommand::SelectTerminal { terminal_id: first },
    );
    let routed = view(until(&mut stream, |event| contains(event, "mouse-routed")));
    assert_eq!(routed.active_terminal, Some(first));
    let screen = text(&routed.terminal.unwrap().screen);
    let words = screen.split_whitespace().collect::<Vec<_>>();
    assert!(
        words
            .windows(9)
            .any(|window| window == ["1b", "5b", "3c", "30", "3b", "32", "3b", "32", "4d"]),
        "{screen}"
    );
}
