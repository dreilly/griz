use super::*;
use griz_protocol::PreviewTarget;

fn terminal_text(state: &ViewState, terminal_id: u64) -> Option<String> {
    state
        .terminals
        .iter()
        .find(|frame| frame.id == terminal_id)
        .map(|frame| text(&frame.screen))
}

fn assert_not_published(state: &ViewState, terminal_id: u64, needles: &[&str]) {
    let mut screens = state
        .terminals
        .iter()
        .filter(|frame| frame.id == terminal_id)
        .map(|frame| &frame.screen)
        .collect::<Vec<_>>();
    if let Some(frame) = state
        .terminal
        .as_ref()
        .filter(|frame| frame.id == terminal_id)
    {
        screens.push(&frame.screen);
    }
    if let Some(preview) = &state.preview {
        screens.extend(
            preview
                .terminals
                .iter()
                .filter(|frame| frame.id == terminal_id)
                .map(|frame| &frame.screen),
        );
    }
    assert!(
        !screens.is_empty(),
        "terminal {terminal_id} was not published"
    );
    for screen in screens {
        let content = text(screen);
        for needle in needles {
            assert!(
                !content.contains(needle),
                "held content {needle:?} leaked through a published frame:\n{content}"
            );
        }
    }
}

fn disable_echo(stream: &mut UnixStream, terminal_id: u64, serial: u64) {
    input(stream, serial, terminal_id, b"stty -echo\r");
    ack(stream, serial);
    // Input acknowledgements may intentionally precede silent PTY work. Give the
    // real shell time to apply the tty flag before sending token-bearing input.
    std::thread::sleep(Duration::from_millis(100));
}

#[test]
fn held_cells_do_not_leak_through_service_publication_paths() {
    let service = Service::start();
    let held_dir = service.dir.join("held-directory");
    fs::create_dir(&held_dir).unwrap();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let workspace_id = initial.active_workspace;
    let first = initial.active_terminal.unwrap();
    disable_echo(&mut stream, first, 1);

    command(
        &mut stream,
        2,
        CoreCommand::SplitTerminal {
            terminal_id: first,
            direction: SplitDirection::Vertical,
        },
    );
    let split = ack(&mut stream, 2);
    let second = split.active_terminal.unwrap();
    command(
        &mut stream,
        3,
        CoreCommand::SelectTerminal { terminal_id: first },
    );
    ack(&mut stream, 3);
    command(
        &mut stream,
        4,
        CoreCommand::SetPreview {
            target: Some(PreviewTarget::Workspace { workspace_id }),
        },
    );
    ack(&mut stream, 4);

    input(
        &mut stream,
        5,
        first,
        format!(
            "printf '\\033[?2026h\\033[2J\\033[Hheld-intermediate'; cd '{}'; read gate; printf '\\033[2J\\033[Hheld-final\\033[?2026l'\r",
            held_dir.display()
        )
        .as_bytes(),
    );

    // Directory polling publishes the catalog while the shell is held. Its frames,
    // including the split and preview copies, must stay on the prior completed screen.
    let directory = view(until(&mut stream, |event| {
        matches!(event, Event::View { state }
            if state.workspaces[0].directory.ends_with("held-directory"))
    }));
    assert_not_published(&directory, first, &["held-intermediate", "held-final"]);

    command(
        &mut stream,
        6,
        CoreCommand::SetPreview {
            target: Some(PreviewTarget::Terminal {
                workspace_id,
                terminal_id: first,
            }),
        },
    );
    let requester = ack(&mut stream, 6);
    assert_not_published(&requester, first, &["held-intermediate", "held-final"]);

    command(
        &mut stream,
        7,
        CoreCommand::RenameWorkspace {
            workspace_id,
            name: "Held catalog".into(),
        },
    );
    let catalog = ack(&mut stream, 7);
    assert_not_published(&catalog, first, &["held-intermediate", "held-final"]);

    command(
        &mut stream,
        8,
        CoreCommand::Scroll {
            terminal_id: first,
            rows: -1,
        },
    );
    let scrolled = ack(&mut stream, 8);
    assert_not_published(&scrolled, first, &["held-intermediate", "held-final"]);

    let first_screen = scrolled
        .terminals
        .iter()
        .find(|frame| frame.id == first)
        .unwrap();
    command(
        &mut stream,
        9,
        CoreCommand::Resize {
            terminal_id: first,
            cols: first_screen.screen.cols,
            rows: first_screen.screen.rows,
        },
    );
    let same_size = ack(&mut stream, 9);
    assert_not_published(&same_size, first, &["held-intermediate", "held-final"]);

    input(
        &mut stream,
        10,
        second,
        b"printf 'other-terminal-output\\n'\r",
    );
    let other_output = view(until(&mut stream, |event| {
        matches!(event, Event::View { state }
            if terminal_text(state, second).is_some_and(|content| content.contains("other-terminal-output")))
    }));
    assert_not_published(&other_output, first, &["held-intermediate", "held-final"]);

    // A fresh observer has no per-client frame yet, so it receives the terminal's
    // last completed live frame (initially blank if no frame ever completed), never held cells.
    let mut observer = service.attach(0, 0);
    let attached = next_view(&mut observer);
    assert_not_published(&attached, first, &["held-intermediate", "held-final"]);

    input(&mut stream, 11, first, b"\r");
    let released = view(until(&mut stream, |event| {
        matches!(event, Event::View { state }
            if terminal_text(state, first).is_some_and(|content| content.contains("held-final")))
    }));
    assert!(
        !terminal_text(&released, first)
            .unwrap()
            .contains("held-intermediate"),
        "release must publish the latest completed redraw"
    );
}

#[test]
fn stalled_and_exited_holds_recover_and_future_holds_work() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let terminal_id = initial.active_terminal.unwrap();
    disable_echo(&mut stream, terminal_id, 1);

    let started = Instant::now();
    input(
        &mut stream,
        2,
        terminal_id,
        b"printf '\\033[?2026h\\033[?2026h\\033[2J\\033[Hstalled-hold'\r",
    );
    until(&mut stream, |event| contains(event, "stalled-hold"));
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(700),
        "stalled content published before bounded timeout recovery: {elapsed:?}"
    );

    input(
        &mut stream,
        3,
        terminal_id,
        b"printf '\\033[?2026h\\033[2J\\033[Hfuture-hold\\033[?2026l'\r",
    );
    until(&mut stream, |event| contains(event, "future-hold"));

    input(
        &mut stream,
        4,
        terminal_id,
        b"printf '\\033[?2026h\\033[2J\\033[Hexit-held'; exit\r",
    );
    let exited = view(until(&mut stream, |event| {
        matches!(event, Event::View { state }
            if state.terminal.as_ref().is_some_and(|frame|
                frame.exited.is_some() && text(&frame.screen).contains("exit-held")))
    }));
    assert!(exited.terminal.unwrap().exited.is_some());
}
