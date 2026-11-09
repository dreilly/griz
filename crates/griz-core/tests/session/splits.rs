use super::*;
use griz_protocol::MAX_PANES_PER_TAB;

fn frame_contains(state: &ViewState, terminal_id: u64, needle: &str) -> bool {
    state
        .terminals
        .iter()
        .find(|terminal| terminal.id == terminal_id)
        .is_some_and(|terminal| text(&terminal.screen).contains(needle))
}

#[test]
fn queued_selection_and_split_can_repeatedly_split_the_original_pane() {
    for direction in [SplitDirection::Horizontal, SplitDirection::Vertical] {
        let service = Service::start();
        let mut stream = service.attach(80, 24);
        let initial = next_view(&mut stream);
        let original = initial.active_terminal.unwrap();
        let tab_id = initial.workspaces[0].tabs[0].id;
        let mut serial = 1;
        for pane_count in 2..=MAX_PANES_PER_TAB {
            // Match the GUI: send selection and split in order without waiting
            // for a selection ack, even though the previous split focused a new pane.
            command(
                &mut stream,
                serial,
                CoreCommand::SelectTerminal {
                    terminal_id: original,
                },
            );
            serial += 1;
            command(
                &mut stream,
                serial,
                CoreCommand::SplitTerminal {
                    terminal_id: original,
                    direction,
                },
            );
            let expanded = ack(&mut stream, serial);
            serial += 1;
            assert_ne!(expanded.active_terminal, Some(original));
            assert_eq!(expanded.workspaces[0].tabs.len(), 1);
            assert_eq!(expanded.workspaces[0].tabs[0].id, tab_id);
            assert_eq!(
                expanded.workspaces[0].tabs[0].layout.terminal_ids().len(),
                pane_count
            );
            assert_eq!(expanded.terminals.len(), pane_count);
        }
    }
}

#[test]
fn nested_splits_publish_all_frames_per_client_and_preserve_pane_sizes_on_focus() {
    let service = Service::start();
    let mut first = service.attach(80, 24);
    let mut second = service.attach(100, 30);
    let initial = next_view(&mut first);
    let first_id = initial.active_terminal.unwrap();
    next_view(&mut second);

    command(
        &mut first,
        1,
        CoreCommand::SplitTerminal {
            terminal_id: first_id,
            direction: SplitDirection::Horizontal,
        },
    );
    let split = ack(&mut first, 1);
    let second_id = split.active_terminal.unwrap();
    assert_ne!(second_id, first_id);
    assert_eq!(
        split
            .terminals
            .iter()
            .map(|frame| frame.id)
            .collect::<Vec<_>>(),
        vec![first_id, second_id]
    );
    assert_eq!(
        split.workspaces[0].tabs[0].layout,
        PaneLayout::Split {
            direction: SplitDirection::Horizontal,
            first: Box::new(PaneLayout::Leaf {
                terminal_id: first_id,
            }),
            second: Box::new(PaneLayout::Leaf {
                terminal_id: second_id,
            }),
        }
    );
    let observer = view(until(
        &mut second,
        |event| matches!(event, Event::View { state } if state.workspaces[0].terminals.len() == 2),
    ));
    assert_eq!(observer.active_terminal, Some(first_id));
    assert_eq!(
        observer
            .terminals
            .iter()
            .map(|frame| frame.id)
            .collect::<Vec<_>>(),
        vec![first_id, second_id]
    );

    command(
        &mut first,
        2,
        CoreCommand::Resize {
            terminal_id: first_id,
            cols: 41,
            rows: 11,
        },
    );
    ack(&mut first, 2);
    command(
        &mut first,
        3,
        CoreCommand::Resize {
            terminal_id: second_id,
            cols: 52,
            rows: 12,
        },
    );
    ack(&mut first, 3);
    command(
        &mut first,
        4,
        CoreCommand::SelectTerminal {
            terminal_id: first_id,
        },
    );
    let focused = ack(&mut first, 4);
    let first_frame = focused
        .terminals
        .iter()
        .find(|frame| frame.id == first_id)
        .unwrap();
    let second_frame = focused
        .terminals
        .iter()
        .find(|frame| frame.id == second_id)
        .unwrap();
    assert_eq!((first_frame.screen.cols, first_frame.screen.rows), (41, 11));
    assert_eq!(
        (second_frame.screen.cols, second_frame.screen.rows),
        (52, 12)
    );

    command(
        &mut first,
        5,
        CoreCommand::SelectTerminal {
            terminal_id: second_id,
        },
    );
    ack(&mut first, 5);
    command(
        &mut first,
        6,
        CoreCommand::SplitTerminal {
            terminal_id: second_id,
            direction: SplitDirection::Vertical,
        },
    );
    let nested = ack(&mut first, 6);
    let third_id = nested.active_terminal.unwrap();
    assert_eq!(nested.workspaces[0].tabs[0].id, first_id);
    assert_eq!(
        nested.workspaces[0].tabs[0].layout,
        PaneLayout::Split {
            direction: SplitDirection::Horizontal,
            first: Box::new(PaneLayout::Leaf {
                terminal_id: first_id,
            }),
            second: Box::new(PaneLayout::Split {
                direction: SplitDirection::Vertical,
                first: Box::new(PaneLayout::Leaf {
                    terminal_id: second_id,
                }),
                second: Box::new(PaneLayout::Leaf {
                    terminal_id: third_id,
                }),
            }),
        }
    );
    assert_eq!(nested.terminals.len(), 3);
    let observer = view(until(
        &mut second,
        |event| matches!(event, Event::View { state } if state.terminals.len() == 3),
    ));
    assert_eq!(observer.active_terminal, Some(first_id));

    input(
        &mut first,
        7,
        second_id,
        b"printf 'visible-sibling-%s\\n' output\r",
    );
    let output = view(until(&mut first, |event| {
        matches!(event, Event::View { state } if state.active_terminal == Some(third_id)
            && frame_contains(state, second_id, "visible-sibling-output"))
    }));
    assert_eq!(output.terminal.as_ref().unwrap().id, third_id);
}

#[test]
fn closing_split_leaves_collapses_the_tree_and_keeps_the_tab_identity() {
    let service = Service::start();
    let mut first = service.attach(80, 24);
    let initial = next_view(&mut first);
    let first_id = initial.active_terminal.unwrap();
    command(
        &mut first,
        1,
        CoreCommand::SplitTerminal {
            terminal_id: first_id,
            direction: SplitDirection::Horizontal,
        },
    );
    let second_id = ack(&mut first, 1).active_terminal.unwrap();
    command(
        &mut first,
        2,
        CoreCommand::SplitTerminal {
            terminal_id: second_id,
            direction: SplitDirection::Vertical,
        },
    );
    let third_id = ack(&mut first, 2).active_terminal.unwrap();
    let mut second = service.attach(80, 24);
    assert_eq!(next_view(&mut second).active_terminal, Some(first_id));

    command(
        &mut first,
        3,
        CoreCommand::SelectTerminal {
            terminal_id: second_id,
        },
    );
    ack(&mut first, 3);
    command(
        &mut first,
        4,
        CoreCommand::CloseTerminal {
            terminal_id: second_id,
        },
    );
    let collapsed = ack(&mut first, 4);
    assert_eq!(collapsed.active_terminal, Some(third_id));
    assert_eq!(collapsed.workspaces[0].tabs[0].id, first_id);
    assert_eq!(
        collapsed.workspaces[0].tabs[0].layout,
        PaneLayout::Split {
            direction: SplitDirection::Horizontal,
            first: Box::new(PaneLayout::Leaf {
                terminal_id: first_id,
            }),
            second: Box::new(PaneLayout::Leaf {
                terminal_id: third_id,
            }),
        }
    );

    command(
        &mut first,
        5,
        CoreCommand::CloseTerminal {
            terminal_id: first_id,
        },
    );
    let stable = ack(&mut first, 5);
    assert_eq!(stable.active_terminal, Some(third_id));
    assert_eq!(stable.workspaces[0].tabs[0].id, first_id);
    assert_eq!(
        stable.workspaces[0].tabs[0].layout,
        PaneLayout::Leaf {
            terminal_id: third_id,
        }
    );
    let observer = view(until(
        &mut second,
        |event| matches!(event, Event::View { state } if state.workspaces[0].terminals.len() == 1),
    ));
    assert_eq!(observer.active_terminal, Some(third_id));
    assert_eq!(observer.terminals.len(), 1);

    command(
        &mut first,
        6,
        CoreCommand::CloseTerminal {
            terminal_id: third_id,
        },
    );
    let empty = ack(&mut first, 6);
    assert!(empty.workspaces.is_empty());
    assert!(empty.terminals.is_empty());
    assert!(empty.terminal.is_none());
}

#[test]
fn split_inherits_cwd_rejects_invalid_targets_and_enforces_the_pane_limit() {
    let service = Service::start();
    let cwd = service.dir.join("split cwd");
    fs::create_dir(&cwd).unwrap();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let workspace_id = initial.active_workspace;
    let first_id = initial.active_terminal.unwrap();

    command(
        &mut stream,
        1,
        CoreCommand::SplitTerminal {
            terminal_id: u64::MAX,
            direction: SplitDirection::Horizontal,
        },
    );
    until(
        &mut stream,
        |event| matches!(event, Event::Error { serial: Some(1), message } if message.contains("unknown terminal")),
    );
    assert_eq!(ack(&mut stream, 1).workspaces[0].terminals.len(), 1);

    command(
        &mut stream,
        2,
        CoreCommand::CreateTerminal {
            workspace_id,
            cwd: None,
        },
    );
    let other_tab = ack(&mut stream, 2).active_terminal.unwrap();
    command(
        &mut stream,
        3,
        CoreCommand::SplitTerminal {
            terminal_id: first_id,
            direction: SplitDirection::Horizontal,
        },
    );
    until(
        &mut stream,
        |event| matches!(event, Event::Error { serial: Some(3), message } if message.contains("not selected")),
    );
    let rejected = ack(&mut stream, 3);
    assert_eq!(rejected.active_terminal, Some(other_tab));
    assert_eq!(rejected.workspaces[0].tabs.len(), 2);

    command(
        &mut stream,
        4,
        CoreCommand::SelectTerminal {
            terminal_id: first_id,
        },
    );
    ack(&mut stream, 4);
    input(
        &mut stream,
        5,
        first_id,
        format!("cd '{}'; printf 'split-cwd-ready\\n'\r", cwd.display()).as_bytes(),
    );
    until(&mut stream, |event| contains(event, "split-cwd-ready"));
    command(
        &mut stream,
        6,
        CoreCommand::SplitTerminal {
            terminal_id: first_id,
            direction: SplitDirection::Vertical,
        },
    );
    let split = ack(&mut stream, 6);
    let mut selected = split.active_terminal.unwrap();
    let inherited = split.workspaces[0]
        .terminals
        .iter()
        .find(|terminal| terminal.id == selected)
        .unwrap();
    assert_eq!(
        PathBuf::from(&inherited.initial_cwd),
        cwd.canonicalize().unwrap()
    );

    let mut serial = 7;
    let mut pane_count = 2;
    while pane_count < MAX_PANES_PER_TAB {
        command(
            &mut stream,
            serial,
            CoreCommand::SplitTerminal {
                terminal_id: selected,
                direction: SplitDirection::Horizontal,
            },
        );
        let expanded = ack(&mut stream, serial);
        selected = expanded.active_terminal.unwrap();
        pane_count = expanded.workspaces[0].tabs[0].layout.terminal_ids().len();
        serial += 1;
    }
    command(
        &mut stream,
        serial,
        CoreCommand::SplitTerminal {
            terminal_id: selected,
            direction: SplitDirection::Horizontal,
        },
    );
    until(&mut stream, |event| {
        matches!(event, Event::Error { serial: Some(found), message }
            if *found == serial && message.contains("pane limit"))
    });
    let limited = ack(&mut stream, serial);
    assert_eq!(
        limited.workspaces[0].tabs[0].layout.terminal_ids().len(),
        MAX_PANES_PER_TAB
    );
    assert_eq!(limited.active_terminal, Some(selected));
}
