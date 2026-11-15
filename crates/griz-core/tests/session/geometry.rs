use super::*;
use griz_protocol::PreviewTarget;

fn screen_geometry(state: &ViewState) -> (u16, u16, u16, u16) {
    let screen = &state.terminal.as_ref().unwrap().screen;
    (
        screen.cols,
        screen.rows,
        screen.cell_width,
        screen.cell_height,
    )
}

fn input_line(stream: &mut UnixStream, serial: u64, terminal_id: u64, command: &[u8]) {
    let mut line = command.to_vec();
    line.push(b'\r');
    input(stream, serial, terminal_id, &line);
}

fn assert_pty_geometry(
    stream: &mut UnixStream,
    serial: u64,
    terminal_id: u64,
    rows: u16,
    cols: u16,
    pixel_width: u16,
    pixel_height: u16,
) {
    input_line(
        stream,
        serial,
        terminal_id,
        br#"python3 -c 'import fcntl,struct,termios; print(*struct.unpack("HHHH", fcntl.ioctl(0, termios.TIOCGWINSZ, b"12345678")))' | { read r c x y; printf 'PTY-WINSIZE:%s,%s,%s,%s\n' "$r" "$c" "$x" "$y"; }"#,
    );
    let expected = format!("PTY-WINSIZE:{rows},{cols},{pixel_width},{pixel_height}");
    until(stream, |event| contains(event, &expected));
}

#[test]
fn cell_geometry_updates_vt_reports_and_pty_pixels_and_survives_resize_and_restart() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let terminal_id = initial.active_terminal.unwrap();
    assert_eq!(screen_geometry(&initial), (80, 24, 8, 16));

    command(
        &mut stream,
        1,
        CoreCommand::SetCellSize {
            terminal_id,
            width: 10,
            height: 20,
        },
    );
    assert_eq!(screen_geometry(&ack(&mut stream, 1)), (80, 24, 10, 20));
    assert_pty_geometry(&mut stream, 2, terminal_id, 24, 80, 800, 480);

    input_line(
        &mut stream,
        3,
        terminal_id,
        br#"stty -echo -icanon min 0 time 5; printf '\033[16t'; reply=$(dd bs=64 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); stty sane; printf 'CSI16:%s\n' "$reply""#,
    );
    until(&mut stream, |event| {
        contains(event, "CSI16:1b5b363b32303b313074")
    });

    command(
        &mut stream,
        4,
        CoreCommand::Resize {
            terminal_id,
            cols: 103,
            rows: 37,
        },
    );
    assert_eq!(screen_geometry(&ack(&mut stream, 4)), (103, 37, 10, 20));
    assert_pty_geometry(&mut stream, 5, terminal_id, 37, 103, 1030, 740);

    input(&mut stream, 6, terminal_id, b"exit 0\r");
    until(
        &mut stream,
        |event| matches!(event, Event::View { state } if state.terminal.as_ref().is_some_and(|terminal| terminal.exited.is_some())),
    );
    command(&mut stream, 7, CoreCommand::StartShell { terminal_id });
    assert_eq!(screen_geometry(&ack(&mut stream, 7)), (103, 37, 10, 20));
    assert_pty_geometry(&mut stream, 8, terminal_id, 37, 103, 1030, 740);
}

#[test]
fn cell_geometry_releases_a_synchronized_hold_and_publishes_the_completed_frame() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let terminal_id = initial.active_terminal.unwrap();

    input(&mut stream, 1, terminal_id, b"stty -echo\r");
    ack(&mut stream, 1);
    input(
        &mut stream,
        2,
        terminal_id,
        b"printf '\\033[?2026h\\033[2J\\033[Hcell-geometry-complete'\r",
    );
    let held = ack(&mut stream, 2);
    assert!(!text(&held.terminal.unwrap().screen).contains("cell-geometry-complete"));
    // Input acknowledgement can precede the PTY reader parsing the held bytes.
    std::thread::sleep(Duration::from_millis(100));

    command(
        &mut stream,
        3,
        CoreCommand::SetCellSize {
            terminal_id,
            width: 9,
            height: 18,
        },
    );
    let released = ack(&mut stream, 3);
    assert_eq!(screen_geometry(&released), (80, 24, 9, 18));
    let released_text = text(&released.terminal.unwrap().screen);
    assert!(
        released_text.contains("cell-geometry-complete"),
        "released screen: {released_text:?}"
    );
}

#[test]
fn latest_live_client_cell_geometry_wins() {
    let service = Service::start();
    let mut first = service.attach(80, 24);
    let mut second = service.attach(80, 24);
    let terminal_id = next_view(&mut first).active_terminal.unwrap();
    next_view(&mut second);

    command(
        &mut first,
        1,
        CoreCommand::SetCellSize {
            terminal_id,
            width: 9,
            height: 18,
        },
    );
    assert_eq!(screen_geometry(&ack(&mut first, 1)), (80, 24, 9, 18));
    command(
        &mut second,
        1,
        CoreCommand::SetCellSize {
            terminal_id,
            width: 11,
            height: 22,
        },
    );
    assert_eq!(screen_geometry(&ack(&mut second, 1)), (80, 24, 11, 22));
    let updated = view(until(
        &mut first,
        |event| matches!(event, Event::View { state } if state.terminal.as_ref().is_some_and(|terminal| terminal.screen.cell_width == 11 && terminal.screen.cell_height == 22)),
    ));
    assert_eq!(screen_geometry(&updated), (80, 24, 11, 22));
}

#[test]
fn previews_hidden_panes_and_invalid_cell_sizes_cannot_set_geometry() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let workspace_id = initial.active_workspace;
    let first_id = initial.active_terminal.unwrap();

    command(
        &mut stream,
        1,
        CoreCommand::CreateTerminal {
            workspace_id,
            cwd: None,
        },
    );
    let second_id = ack(&mut stream, 1).active_terminal.unwrap();
    command(
        &mut stream,
        2,
        CoreCommand::SelectTerminal {
            terminal_id: first_id,
        },
    );
    ack(&mut stream, 2);
    command(
        &mut stream,
        3,
        CoreCommand::SetPreview {
            target: Some(PreviewTarget::Terminal {
                workspace_id,
                terminal_id: second_id,
            }),
        },
    );
    ack(&mut stream, 3);
    command(
        &mut stream,
        4,
        CoreCommand::SetCellSize {
            terminal_id: second_id,
            width: 12,
            height: 24,
        },
    );
    until(
        &mut stream,
        |event| matches!(event, Event::Error { serial: Some(4), message } if message.contains("live layout")),
    );
    ack(&mut stream, 4);

    for (serial, width, height) in [(5, 0, 16), (6, 8, 513)] {
        command(
            &mut stream,
            serial,
            CoreCommand::SetCellSize {
                terminal_id: first_id,
                width,
                height,
            },
        );
        until(
            &mut stream,
            |event| matches!(event, Event::Error { serial: Some(found), message } if *found == serial && message.contains("between 1 and 512")),
        );
        ack(&mut stream, serial);
    }

    command(
        &mut stream,
        7,
        CoreCommand::SelectTerminal {
            terminal_id: second_id,
        },
    );
    assert_eq!(screen_geometry(&ack(&mut stream, 7)), (80, 24, 8, 16));
}
