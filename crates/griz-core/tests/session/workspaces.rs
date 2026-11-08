use super::*;

#[test]
fn close_workspace_updates_clients_preserves_selections_and_resizes_fallback() {
    check_workspace_close(false);
}

#[test]
fn closing_final_tab_removes_workspace_preserves_selections_and_resizes_fallback() {
    check_workspace_close(true);
}

fn check_workspace_close(close_final_tab: bool) {
    let service = Service::start();
    let mut first = service.attach(80, 24);
    let initial = next_view(&mut first);
    let daily = initial.active_workspace;
    command(
        &mut first,
        1,
        CoreCommand::CreateTerminal {
            workspace_id: daily,
            cwd: None,
        },
    );
    let selected = ack(&mut first, 1).active_terminal;
    command(
        &mut first,
        2,
        CoreCommand::CreateWorkspace {
            name: "Project".into(),
        },
    );
    let created = ack(&mut first, 2);
    let project = created.active_workspace;
    let project_terminal = created.active_terminal.unwrap();
    let mut second = service.attach(103, 37);
    let second_initial = next_view(&mut second);
    let second_selected = second_initial.active_terminal;
    assert_eq!(second_initial.active_workspace, daily);
    command(
        &mut first,
        3,
        if close_final_tab {
            CoreCommand::CloseTerminal {
                terminal_id: project_terminal,
            }
        } else {
            CoreCommand::CloseWorkspace {
                workspace_id: project,
            }
        },
    );
    let closed = ack(&mut first, 3);
    assert_eq!(closed.workspaces.len(), 1);
    assert_eq!(closed.active_workspace, daily);
    assert_eq!(closed.active_terminal, selected);
    let screen = closed.terminal.unwrap().screen;
    assert_eq!((screen.cols, screen.rows), (80, 24));
    let other = view(until(
        &mut second,
        |event| matches!(event, Event::View { state } if state.workspaces.len() == 1),
    ));
    assert_eq!(other.active_workspace, daily);
    assert_eq!(other.active_terminal, second_selected);
    // A stale close must not delete a different workspace or alter selections.
    command(
        &mut first,
        4,
        CoreCommand::CloseWorkspace {
            workspace_id: project,
        },
    );
    until(
        &mut first,
        |event| matches!(event, Event::Error { serial: Some(4), message } if message.contains("unknown workspace")),
    );
    let rejected = ack(&mut first, 4);
    assert_eq!(rejected.workspaces.len(), 1);
    assert_eq!(rejected.active_terminal, selected);
}

#[test]
fn closing_final_tab_in_inactive_workspace_preserves_requester_and_switches_viewers() {
    let service = Service::start();
    let mut first = service.attach(80, 24);
    let initial = next_view(&mut first);
    let original = initial.active_workspace;
    command(
        &mut first,
        1,
        CoreCommand::CreateWorkspace {
            name: "Other".into(),
        },
    );
    let created = ack(&mut first, 1);
    let workspace_id = created.active_workspace;
    let terminal_id = created.active_terminal.unwrap();
    let mut second = service.attach(80, 24);
    next_view(&mut second);
    command(
        &mut second,
        1,
        CoreCommand::SelectWorkspace { workspace_id },
    );
    ack(&mut second, 1);
    command(
        &mut first,
        2,
        CoreCommand::SelectWorkspace {
            workspace_id: original,
        },
    );
    ack(&mut first, 2);
    command(&mut first, 3, CoreCommand::CloseTerminal { terminal_id });
    let closed = ack(&mut first, 3);
    assert_eq!(closed.workspaces.len(), 1);
    assert_eq!(closed.active_workspace, original);
    assert_eq!(closed.active_terminal, initial.active_terminal);
    let other = view(until(
        &mut second,
        |event| matches!(event, Event::View { state } if state.workspaces.len() == 1),
    ));
    assert_eq!(other.active_workspace, original);
    assert_eq!(other.active_terminal, initial.active_terminal);
    // Repeating a stale tab close must never remove the fallback workspace.
    command(&mut first, 4, CoreCommand::CloseTerminal { terminal_id });
    until(
        &mut first,
        |event| matches!(event, Event::Error { serial: Some(4), message } if message.contains("unknown terminal")),
    );
    assert_eq!(ack(&mut first, 4).workspaces.len(), 1);
}

#[test]
fn close_last_workspace_allows_empty_attach_and_recreation_for_all_clients() {
    let service = Service::start();
    let mut first = service.attach(80, 24);
    let daily = next_view(&mut first).active_workspace;
    let mut second = service.attach(80, 24);
    next_view(&mut second);
    command(
        &mut first,
        1,
        CoreCommand::CloseWorkspace {
            workspace_id: daily,
        },
    );
    let empty = ack(&mut first, 1);
    assert!(empty.workspaces.is_empty());
    assert_eq!(empty.active_workspace, 0);
    assert_eq!(empty.active_terminal, None);
    assert!(empty.terminal.is_none());
    let other = view(until(
        &mut second,
        |event| matches!(event, Event::View { state } if state.workspaces.is_empty()),
    ));
    assert_eq!(other.active_workspace, 0);
    assert_eq!(other.active_terminal, None);
    let mut third = service.attach(80, 24);
    assert!(next_view(&mut third).workspaces.is_empty());
    command(
        &mut first,
        2,
        CoreCommand::CreateWorkspace {
            name: "Fresh".into(),
        },
    );
    let fresh = ack(&mut first, 2);
    assert_eq!(fresh.workspaces.len(), 1);
    assert!(fresh.active_workspace > daily);
    assert!(fresh.active_terminal.is_some());
    for client in [&mut second, &mut third] {
        let other = view(until(
            client,
            |event| matches!(event, Event::View { state } if state.workspaces.len() == 1),
        ));
        assert_eq!(other.active_workspace, fresh.active_workspace);
        assert_eq!(other.active_terminal, fresh.active_terminal);
    }
    // Closing the only tab removes the recreated workspace for every client.
    command(
        &mut first,
        3,
        CoreCommand::CloseTerminal {
            terminal_id: fresh.active_terminal.unwrap(),
        },
    );
    let empty = ack(&mut first, 3);
    assert!(empty.workspaces.is_empty());
    assert_eq!(empty.active_workspace, 0);
    assert_eq!(empty.active_terminal, None);
    assert!(empty.terminal.is_none());
    for client in [&mut second, &mut third] {
        let other = view(until(
            client,
            |event| matches!(event, Event::View { state } if state.workspaces.is_empty()),
        ));
        assert_eq!(other.active_workspace, 0);
        assert_eq!(other.active_terminal, None);
        assert!(other.terminal.is_none());
    }
    let mut reattached = service.attach(80, 24);
    assert!(next_view(&mut reattached).workspaces.is_empty());
}

#[test]
fn close_inactive_workspace_ends_all_its_shells_and_foreground_jobs_only() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let survivor = initial.active_terminal.unwrap();
    let daily = initial.active_workspace;
    command(
        &mut stream,
        1,
        CoreCommand::CreateWorkspace {
            name: "Disposable".into(),
        },
    );
    let created = ack(&mut stream, 1);
    let workspace_id = created.active_workspace;
    let first = created.active_terminal.unwrap();
    let mut pids = vec![created.terminal.unwrap().shell_pid as i32];
    command(
        &mut stream,
        2,
        CoreCommand::CreateTerminal {
            workspace_id,
            cwd: None,
        },
    );
    let created = ack(&mut stream, 2);
    let second = created.active_terminal.unwrap();
    pids.push(created.terminal.unwrap().shell_pid as i32);
    let pid_file = service.dir.join("workspace-foreground.pid");
    let script = format!("sh -c 'echo $$ > {}; exec sleep 30'\r", pid_file.display());
    input(&mut stream, 3, second, script.as_bytes());
    ack(&mut stream, 3);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !pid_file.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    pids.push(
        fs::read_to_string(pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap(),
    );
    command(
        &mut stream,
        4,
        CoreCommand::SelectWorkspace {
            workspace_id: daily,
        },
    );
    ack(&mut stream, 4);
    command(&mut stream, 5, CoreCommand::CloseWorkspace { workspace_id });
    let closed = ack(&mut stream, 5);
    assert_eq!(closed.active_workspace, daily);
    assert_eq!(closed.active_terminal, Some(survivor));
    assert_eq!(closed.workspaces.len(), 1);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        // SAFETY: signal 0 only checks these isolated test processes.
        if pids.iter().all(|pid| unsafe { libc::kill(*pid, 0) } == -1) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    for pid in pids {
        // SAFETY: signal 0 does not modify the process.
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "process {pid} survived workspace close"
        );
    }
    input(&mut stream, 6, first, b"printf wrong\r");
    until(
        &mut stream,
        |event| matches!(event, Event::Error { serial: Some(6), message } if message.contains("unknown terminal")),
    );
    ack(&mut stream, 6);
    input(
        &mut stream,
        7,
        survivor,
        b"printf 'workspace-survivor-%s\\n' alive\r",
    );
    until(&mut stream, |event| {
        contains(event, "workspace-survivor-alive")
    });
}
