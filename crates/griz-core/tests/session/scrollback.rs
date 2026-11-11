use super::*;

fn scroll(stream: &mut UnixStream, serial: u64, rows: i32) -> ViewState {
    command(
        stream,
        serial,
        CoreCommand::Scroll {
            terminal_id: 1,
            rows,
        },
    );
    ack(stream, serial)
}

#[test]
fn scrollback_is_per_client_survives_output_and_typing_returns_to_live() {
    let service = Service::start();
    let mut first = service.attach(40, 6);
    next_view(&mut first);
    let mut second = service.attach(40, 6);
    next_view(&mut second);

    input(&mut first, 1, 1,
        b"stty -echo; i=0; while [ $i -lt 30 ]; do printf 'history-%02d\\n' \"$i\"; i=$((i+1)); done\n");
    until(&mut first, |event| contains(event, "history-29"));
    until(&mut second, |event| contains(event, "history-29"));

    let history = scroll(&mut first, 2, -12);
    let screen = &history.terminal.unwrap().screen;
    assert!(screen.scrollback_offset > 0);
    assert!(!text(screen).contains("history-29"));
    let old_top = text(screen).lines().next().unwrap().to_string();

    // Output from another client must neither snap this one down nor move its anchor.
    input(&mut second, 1, 1, b"printf 'fresh-output\\n'\n");
    until(&mut second, |event| contains(event, "fresh-output"));
    let update = view(until(&mut first, |event| {
        matches!(event, Event::View { state }
        if state.terminal.as_ref().is_some_and(|t| t.screen.scrollback_offset > screen.scrollback_offset))
    }));
    assert_eq!(
        text(&update.terminal.unwrap().screen)
            .lines()
            .next()
            .unwrap(),
        old_top
    );

    let down = scroll(&mut first, 3, i32::MAX);
    assert_eq!(down.terminal.as_ref().unwrap().screen.scrollback_offset, 0);
    assert!(text(&down.terminal.unwrap().screen).contains("fresh-output"));
    assert!(
        scroll(&mut first, 4, -10)
            .terminal
            .unwrap()
            .screen
            .scrollback_offset
            > 0
    );
    input(&mut first, 5, 1, b"printf 'typed-at-live\\n'\n");
    let live = view(until(&mut first, |event| contains(event, "typed-at-live")));
    assert_eq!(live.terminal.unwrap().screen.scrollback_offset, 0);
}

#[test]
fn scrolling_an_exited_shell_is_allowed_and_unknown_targets_are_rejected() {
    let service = Service::start();
    let mut stream = service.attach(40, 6);
    next_view(&mut stream);
    input(
        &mut stream,
        1,
        1,
        b"i=0; while [ $i -lt 20 ]; do echo line-$i; i=$((i+1)); done; exit\n",
    );
    until(&mut stream, |event| {
        matches!(event, Event::View { state }
        if state.terminal.as_ref().is_some_and(|t| t.exited.is_some()))
    });
    assert!(
        scroll(&mut stream, 2, -10)
            .terminal
            .unwrap()
            .screen
            .scrollback_offset
            > 0
    );
    command(
        &mut stream,
        3,
        CoreCommand::Scroll {
            terminal_id: 999,
            rows: -1,
        },
    );
    until(&mut stream, |event| {
        matches!(
            event,
            Event::Error {
                serial: Some(3),
                ..
            }
        )
    });
}
