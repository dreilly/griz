use super::*;
use griz_protocol::PreviewTarget;

#[test]
fn previews_stream_hidden_split_output_without_selecting_or_resizing() {
    let service = Service::start();
    let mut stream = service.attach(90, 30);
    let initial = next_view(&mut stream);
    let original = initial.active_terminal;
    let original_workspace = initial.active_workspace;
    command(
        &mut stream,
        1,
        CoreCommand::CreateWorkspace {
            name: "Preview me".into(),
        },
    );
    let created = ack(&mut stream, 1);
    let workspace_id = created.active_workspace;
    let first = created.active_terminal.unwrap();
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
        CoreCommand::Resize {
            terminal_id: second,
            cols: 52,
            rows: 18,
        },
    );
    ack(&mut stream, 3);
    command(
        &mut stream,
        4,
        CoreCommand::SelectWorkspace {
            workspace_id: original_workspace,
        },
    );
    ack(&mut stream, 4);

    // Another client must not inherit this client's preview or remembered selection.
    let mut observer = service.attach(0, 0);
    next_view(&mut observer);
    let target = PreviewTarget::Workspace { workspace_id };
    command(
        &mut stream,
        5,
        CoreCommand::SetPreview {
            target: Some(target),
        },
    );
    let previewed = ack(&mut stream, 5);
    assert_eq!(previewed.active_workspace, original_workspace);
    assert_eq!(previewed.active_terminal, original);
    assert_eq!(previewed.terminals.len(), 1);
    let preview = previewed.preview.unwrap();
    assert_eq!(preview.target, target);
    assert_eq!(preview.selected, second);
    assert_eq!(
        preview.layout,
        split
            .workspaces
            .iter()
            .find(|w| w.id == workspace_id)
            .unwrap()
            .tabs[0]
            .layout
    );
    assert_eq!(preview.terminals.len(), 2);
    let screen = &preview
        .terminals
        .iter()
        .find(|t| t.id == second)
        .unwrap()
        .screen;
    assert_eq!((screen.cols, screen.rows), (52, 18));

    // This is output from a hidden tab, delivered after the command acknowledgement,
    // not just a still image fetched when the subscription was first established.
    input(
        &mut stream,
        6,
        second,
        b"sleep 0.1; printf 'preview-%s\\n' streaming\r",
    );
    let live = view(until(&mut stream, |event| {
        matches!(event, Event::View { state }
        if state.preview.as_ref().is_some_and(|p| p.terminals.iter().any(|t| text(&t.screen).contains("preview-streaming"))))
    }));
    assert_eq!(live.active_terminal, original);
    command(&mut observer, 1, CoreCommand::SetPreview { target: None });
    assert!(ack(&mut observer, 1).preview.is_none());

    // Explicit pane targets highlight that pane but do not change the workspace's
    // remembered pane (which is still the second one).
    command(
        &mut stream,
        7,
        CoreCommand::SetPreview {
            target: Some(PreviewTarget::Terminal {
                workspace_id,
                terminal_id: first,
            }),
        },
    );
    assert_eq!(ack(&mut stream, 7).preview.unwrap().selected, first);
    command(&mut stream, 8, CoreCommand::SetPreview { target: None });
    assert!(ack(&mut stream, 8).preview.is_none());
    command(
        &mut stream,
        9,
        CoreCommand::SelectWorkspace { workspace_id },
    );
    assert_eq!(ack(&mut stream, 9).active_terminal, Some(second));
}

#[test]
fn removed_and_mismatched_preview_targets_are_safe() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    command(
        &mut stream,
        1,
        CoreCommand::CreateWorkspace {
            name: "Temporary".into(),
        },
    );
    let created = ack(&mut stream, 1);
    let workspace_id = created.active_workspace;
    let terminal_id = created.active_terminal.unwrap();
    command(
        &mut stream,
        2,
        CoreCommand::SetPreview {
            target: Some(PreviewTarget::Terminal {
                workspace_id: initial.active_workspace,
                terminal_id,
            }),
        },
    );
    assert!(ack(&mut stream, 2).preview.is_none());
    command(
        &mut stream,
        3,
        CoreCommand::SetPreview {
            target: Some(PreviewTarget::Workspace { workspace_id }),
        },
    );
    assert!(ack(&mut stream, 3).preview.is_some());
    command(&mut stream, 4, CoreCommand::CloseWorkspace { workspace_id });
    let closed = ack(&mut stream, 4);
    assert!(closed.preview.is_none());
    assert_eq!(closed.active_workspace, initial.active_workspace);
}
