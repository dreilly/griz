use super::*;
use griz_protocol::WorkspaceAccent;

#[test]
fn fresh_core_uses_the_first_clients_default_accent_in_its_first_view() {
    for accent in WorkspaceAccent::ALL {
        // A new core (including a restart) should initialize from the attaching GUI.
        let service = Service::start();
        let mut first = service.attach_with_accent(80, 24, accent);
        let initial = next_view(&mut first);
        assert_eq!(initial.workspaces.len(), 1);
        assert_eq!(initial.workspaces[0].accent, accent);
        assert_eq!(initial.workspaces[0].terminals.len(), 1);
        assert_eq!(initial.active_workspace, initial.workspaces[0].id);
        assert_eq!(
            initial.active_terminal,
            Some(initial.workspaces[0].terminals[0].id)
        );

        // Another window with a different default cannot overwrite this choice.
        let other_accent = if accent == WorkspaceAccent::Pink {
            WorkspaceAccent::Blue
        } else {
            WorkspaceAccent::Pink
        };
        let mut observer = service.attach_with_accent(0, 0, other_accent);
        let observed = next_view(&mut observer);
        assert_eq!(observed.workspaces[0].accent, accent);
        assert_eq!(
            observed.workspaces[0].terminals[0].shell_pid,
            initial.workspaces[0].terminals[0].shell_pid
        );

        // Explicit edits also survive later attachments and reconnects.
        command(
            &mut first,
            1,
            CoreCommand::SetWorkspaceAccent {
                workspace_id: initial.active_workspace,
                accent: other_accent,
            },
        );
        assert_eq!(ack(&mut first, 1).workspaces[0].accent, other_accent);
        drop(first);
        drop(observer);
        let mut reconnected = service.attach_with_accent(80, 24, accent);
        assert_eq!(
            next_view(&mut reconnected).workspaces[0].accent,
            other_accent
        );
    }
}

#[test]
fn attaching_to_an_emptied_core_does_not_recreate_the_initial_workspace() {
    let service = Service::start();
    let mut first = service.attach_with_accent(80, 24, WorkspaceAccent::Purple);
    let initial = next_view(&mut first);
    command(
        &mut first,
        1,
        CoreCommand::CloseWorkspace {
            workspace_id: initial.active_workspace,
        },
    );
    assert!(ack(&mut first, 1).workspaces.is_empty());
    let mut reconnected = service.attach_with_accent(80, 24, WorkspaceAccent::Orange);
    assert!(next_view(&mut reconnected).workspaces.is_empty());
}

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
