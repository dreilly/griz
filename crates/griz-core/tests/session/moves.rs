use super::*;
use std::collections::BTreeMap;

fn nested_layout(first: u64, second: u64, third: u64) -> PaneLayout {
    PaneLayout::Split {
        direction: SplitDirection::Horizontal,
        first: Box::new(PaneLayout::Leaf { terminal_id: first }),
        second: Box::new(PaneLayout::Split {
            direction: SplitDirection::Vertical,
            first: Box::new(PaneLayout::Leaf {
                terminal_id: second,
            }),
            second: Box::new(PaneLayout::Leaf { terminal_id: third }),
        }),
    }
}

fn create_nested_tab(stream: &mut UnixStream) -> (ViewState, u64, u64, u64, u64) {
    let initial = next_view(stream);
    let first = initial.active_terminal.unwrap();
    let tab_id = initial.workspaces[0].tabs[0].id;
    command(
        stream,
        1,
        CoreCommand::SplitTerminal {
            terminal_id: first,
            direction: SplitDirection::Horizontal,
        },
    );
    let second = ack(stream, 1).active_terminal.unwrap();
    command(
        stream,
        2,
        CoreCommand::SplitTerminal {
            terminal_id: second,
            direction: SplitDirection::Vertical,
        },
    );
    let nested = ack(stream, 2);
    let third = nested.active_terminal.unwrap();
    (nested, tab_id, first, second, third)
}

#[test]
fn swap_moves_leaf_ids_preserves_tab_and_processes_and_selects_inactive_source() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let (before, tab_id, first, second, third) = create_nested_tab(&mut stream);
    assert_eq!(
        before.workspaces[0].tabs[0].layout,
        nested_layout(first, second, third)
    );
    let pids = before.workspaces[0]
        .terminals
        .iter()
        .map(|terminal| (terminal.id, terminal.shell_pid))
        .collect::<BTreeMap<_, _>>();

    command(
        &mut stream,
        3,
        CoreCommand::MovePane {
            terminal_id: first,
            target_terminal_id: third,
            placement: PanePlacement::Swap,
        },
    );
    let moved = ack(&mut stream, 3);

    assert_eq!(moved.active_terminal, Some(first));
    assert_eq!(moved.workspaces[0].tabs[0].id, tab_id);
    assert_eq!(
        moved.workspaces[0].tabs[0].layout,
        nested_layout(third, second, first)
    );
    assert_eq!(
        moved.workspaces[0]
            .terminals
            .iter()
            .map(|terminal| (terminal.id, terminal.shell_pid))
            .collect::<BTreeMap<_, _>>(),
        pids
    );
}

#[test]
fn directional_moves_collapse_the_source_branch_and_nest_around_the_target() {
    for placement in [
        PanePlacement::Left,
        PanePlacement::Right,
        PanePlacement::Top,
        PanePlacement::Bottom,
    ] {
        let service = Service::start();
        let mut stream = service.attach(80, 24);
        let (_, tab_id, first, second, third) = create_nested_tab(&mut stream);

        command(
            &mut stream,
            3,
            CoreCommand::MovePane {
                terminal_id: first,
                target_terminal_id: third,
                placement,
            },
        );
        let moved = ack(&mut stream, 3);
        let (direction, target_branch) = match placement {
            PanePlacement::Left => (
                SplitDirection::Vertical,
                PaneLayout::Split {
                    direction: SplitDirection::Vertical,
                    first: Box::new(PaneLayout::Leaf { terminal_id: first }),
                    second: Box::new(PaneLayout::Leaf { terminal_id: third }),
                },
            ),
            PanePlacement::Right => (
                SplitDirection::Vertical,
                PaneLayout::Split {
                    direction: SplitDirection::Vertical,
                    first: Box::new(PaneLayout::Leaf { terminal_id: third }),
                    second: Box::new(PaneLayout::Leaf { terminal_id: first }),
                },
            ),
            PanePlacement::Top => (
                SplitDirection::Vertical,
                PaneLayout::Split {
                    direction: SplitDirection::Horizontal,
                    first: Box::new(PaneLayout::Leaf { terminal_id: first }),
                    second: Box::new(PaneLayout::Leaf { terminal_id: third }),
                },
            ),
            PanePlacement::Bottom => (
                SplitDirection::Vertical,
                PaneLayout::Split {
                    direction: SplitDirection::Horizontal,
                    first: Box::new(PaneLayout::Leaf { terminal_id: third }),
                    second: Box::new(PaneLayout::Leaf { terminal_id: first }),
                },
            ),
            PanePlacement::Swap => unreachable!(),
        };
        assert_eq!(moved.active_terminal, Some(first));
        assert_eq!(moved.workspaces[0].tabs[0].id, tab_id);
        assert_eq!(
            moved.workspaces[0].tabs[0].layout,
            PaneLayout::Split {
                direction,
                first: Box::new(PaneLayout::Leaf {
                    terminal_id: second,
                }),
                second: Box::new(target_branch),
            }
        );
        assert_eq!(
            moved.workspaces[0].tabs[0].layout.terminal_ids(),
            match placement {
                PanePlacement::Left | PanePlacement::Top => vec![second, first, third],
                PanePlacement::Right | PanePlacement::Bottom => vec![second, third, first],
                PanePlacement::Swap => unreachable!(),
            }
        );
    }
}

#[test]
fn move_rejects_unselected_tabs_self_cross_tab_cross_workspace_and_stale_ids() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let workspace_id = initial.active_workspace;
    let first = initial.active_terminal.unwrap();
    command(
        &mut stream,
        1,
        CoreCommand::SplitTerminal {
            terminal_id: first,
            direction: SplitDirection::Horizontal,
        },
    );
    let split = ack(&mut stream, 1);
    let second = split.active_terminal.unwrap();
    let original_layout = split.workspaces[0].tabs[0].layout.clone();

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
        CoreCommand::MovePane {
            terminal_id: first,
            target_terminal_id: second,
            placement: PanePlacement::Left,
        },
    );
    until(
        &mut stream,
        |event| matches!(event, Event::Error { serial: Some(3), message } if message.contains("selected tab")),
    );
    assert_eq!(
        ack(&mut stream, 3).workspaces[0].tabs[0].layout,
        original_layout
    );

    command(
        &mut stream,
        4,
        CoreCommand::SelectTerminal { terminal_id: first },
    );
    ack(&mut stream, 4);
    command(
        &mut stream,
        5,
        CoreCommand::MovePane {
            terminal_id: first,
            target_terminal_id: first,
            placement: PanePlacement::Swap,
        },
    );
    until(
        &mut stream,
        |event| matches!(event, Event::Error { serial: Some(5), message } if message.contains("itself")),
    );
    ack(&mut stream, 5);

    command(
        &mut stream,
        6,
        CoreCommand::MovePane {
            terminal_id: first,
            target_terminal_id: other_tab,
            placement: PanePlacement::Right,
        },
    );
    until(
        &mut stream,
        |event| matches!(event, Event::Error { serial: Some(6), message } if message.contains("same tab")),
    );
    ack(&mut stream, 6);

    command(
        &mut stream,
        7,
        CoreCommand::CloseTerminal {
            terminal_id: other_tab,
        },
    );
    ack(&mut stream, 7);
    command(
        &mut stream,
        8,
        CoreCommand::MovePane {
            terminal_id: first,
            target_terminal_id: other_tab,
            placement: PanePlacement::Swap,
        },
    );
    until(
        &mut stream,
        |event| matches!(event, Event::Error { serial: Some(8), message } if message.contains("unknown terminal")),
    );
    ack(&mut stream, 8);
    command(
        &mut stream,
        9,
        CoreCommand::MovePane {
            terminal_id: other_tab,
            target_terminal_id: first,
            placement: PanePlacement::Swap,
        },
    );
    until(
        &mut stream,
        |event| matches!(event, Event::Error { serial: Some(9), message } if message.contains("unknown terminal")),
    );
    ack(&mut stream, 9);

    command(
        &mut stream,
        10,
        CoreCommand::CreateWorkspace {
            name: "Other".into(),
            accent: Default::default(),
        },
    );
    ack(&mut stream, 10);
    command(
        &mut stream,
        11,
        CoreCommand::MovePane {
            terminal_id: first,
            target_terminal_id: second,
            placement: PanePlacement::Bottom,
        },
    );
    until(
        &mut stream,
        |event| matches!(event, Event::Error { serial: Some(11), message } if message.contains("active workspace")),
    );
    let rejected = ack(&mut stream, 11);
    let old_workspace = rejected
        .workspaces
        .iter()
        .find(|workspace| workspace.id == workspace_id)
        .unwrap();
    assert_eq!(old_workspace.tabs[0].layout, original_layout);
}
