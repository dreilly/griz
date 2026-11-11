use super::*;
use griz_protocol::WorkspaceAccent;

#[test]
fn workspace_creation_sets_the_requested_accent_atomically_for_every_client() {
    let service = Service::start();
    let mut first = service.attach(80, 24);
    next_view(&mut first);
    let mut observer = service.attach(80, 24);
    let original = next_view(&mut observer);
    for (index, accent) in WorkspaceAccent::ALL.into_iter().enumerate() {
        let serial = index as u64 + 1;
        command(
            &mut first,
            serial,
            CoreCommand::CreateWorkspace {
                name: format!("Project {index}"),
                accent,
            },
        );
        let created = ack(&mut first, serial);
        let id = created.active_workspace;
        assert_eq!(
            created
                .workspaces
                .iter()
                .find(|w| w.id == id)
                .unwrap()
                .accent,
            accent
        );
        let seen = view(until(
            &mut observer,
            |event| matches!(event, Event::View { state } if state.workspaces.iter().any(|w| w.id == id)),
        ));
        assert_eq!(
            seen.workspaces.iter().find(|w| w.id == id).unwrap().accent,
            accent
        );
        assert_eq!(seen.workspaces[0].accent, WorkspaceAccent::Green);
        assert_eq!(seen.active_workspace, original.active_workspace);
        assert_eq!(seen.active_terminal, original.active_terminal);
    }
    let mut reconnected = service.attach(80, 24);
    let restored = next_view(&mut reconnected);
    assert_eq!(
        restored.workspaces.last().unwrap().accent,
        WorkspaceAccent::Graphite
    );
}

#[test]
fn workspace_accents_sync_across_clients_and_reconnect_without_changing_selections() {
    let service = Service::start();
    let mut first = service.attach(80, 24);
    let initial = next_view(&mut first);
    assert_eq!(initial.workspaces[0].accent, WorkspaceAccent::Green);
    command(
        &mut first,
        1,
        CoreCommand::CreateWorkspace {
            name: "Project".into(),
            accent: Default::default(),
        },
    );
    let created = ack(&mut first, 1);
    let workspace_id = created.active_workspace;
    let project_terminal = created.active_terminal;
    let project = created
        .workspaces
        .iter()
        .find(|workspace| workspace.id == workspace_id)
        .unwrap()
        .clone();
    assert_eq!(project.accent, WorkspaceAccent::Green);
    command(
        &mut first,
        2,
        CoreCommand::SelectWorkspace {
            workspace_id: initial.active_workspace,
        },
    );
    let baseline = ack(&mut first, 2);
    let mut second = service.attach(103, 37);
    next_view(&mut second);
    command(
        &mut second,
        1,
        CoreCommand::SelectWorkspace { workspace_id },
    );
    ack(&mut second, 1);
    for (index, accent) in WorkspaceAccent::ALL.into_iter().enumerate() {
        let serial = index as u64 + 3;
        command(
            &mut first,
            serial,
            CoreCommand::SetWorkspaceAccent {
                workspace_id,
                accent,
            },
        );
        let updated = ack(&mut first, serial);
        assert_eq!(updated.active_workspace, baseline.active_workspace);
        assert_eq!(updated.active_terminal, baseline.active_terminal);
        assert_eq!(updated.workspaces[0], baseline.workspaces[0]);
        let mut expected = project.clone();
        expected.accent = accent;
        assert_eq!(updated.workspaces[1], expected);
        let other = view(until(&mut second, |event| {
            matches!(
                event,
                Event::View { state } if state.workspaces.iter().any(|workspace| workspace.id == workspace_id && workspace.accent == accent)
            )
        }));
        assert_eq!(other.active_workspace, workspace_id);
        assert_eq!(other.active_terminal, project_terminal);
        assert_eq!(other.workspaces[1], expected);
    }
    let mut reconnected = service.attach(80, 24);
    let restored = next_view(&mut reconnected);
    assert_eq!(restored.workspaces[1].accent, WorkspaceAccent::Graphite);
    // Renaming and adding tabs leave the workspace color untouched.
    command(
        &mut first,
        9,
        CoreCommand::RenameWorkspace {
            workspace_id,
            name: "Renamed".into(),
        },
    );
    let renamed = ack(&mut first, 9);
    assert_eq!(renamed.workspaces[1].name, "Renamed");
    assert_eq!(renamed.workspaces[1].accent, WorkspaceAccent::Graphite);
    command(
        &mut first,
        10,
        CoreCommand::CreateTerminal {
            workspace_id,
            cwd: None,
        },
    );
    let extended = ack(&mut first, 10);
    assert_eq!(extended.workspaces[1].terminals.len(), 2);
    assert_eq!(extended.workspaces[1].accent, WorkspaceAccent::Graphite);
    // A stale target must never recolor some other workspace.
    command(
        &mut first,
        11,
        CoreCommand::SetWorkspaceAccent {
            workspace_id: u64::MAX,
            accent: WorkspaceAccent::Pink,
        },
    );
    until(&mut first, |event| {
        matches!(
            event,
            Event::Error { serial: Some(11), message } if message.contains("unknown workspace")
        )
    });
    let rejected = ack(&mut first, 11);
    assert_eq!(rejected.workspaces, extended.workspaces);
    assert_eq!(rejected.active_workspace, extended.active_workspace);
    assert_eq!(rejected.active_terminal, extended.active_terminal);
}
