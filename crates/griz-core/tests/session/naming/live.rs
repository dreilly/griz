use super::*;

fn directory_view(stream: &mut UnixStream, directory: &str) -> ViewState {
    view(until(stream, |event| {
        matches!(event, Event::View { state } if
        state.workspaces.iter().any(|workspace| workspace.directory == directory))
    }))
}

#[test]
fn catalog_tracks_each_terminals_live_directory_even_when_unselected_and_silent() {
    let service = Service::start();
    let documents = service.dir.join("Documents");
    let project = service.dir.join("project");
    let moved = service.dir.join("moved 日本語");
    for path in [&documents, &project, &moved] {
        fs::create_dir(path).unwrap();
    }
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let workspace_id = initial.active_workspace;
    let first_id = initial.active_terminal.unwrap();
    input(
        &mut stream,
        1,
        first_id,
        format!("stty -echo; PS1=''; cd '{}'\r", documents.display()).as_bytes(),
    );
    let first = directory_view(&mut stream, &full_path(&documents.canonicalize().unwrap()));
    assert_eq!(
        first.workspaces[0].terminals[0].directory,
        first.workspaces[0].directory
    );
    let initial_cwd = first.workspaces[0].terminals[0].initial_cwd.clone();
    command(
        &mut stream,
        2,
        CoreCommand::CreateTerminal {
            workspace_id,
            cwd: None,
        },
    );
    let created = ack(&mut stream, 2);
    let second_id = created.active_terminal.unwrap();
    input(
        &mut stream,
        3,
        second_id,
        format!("cd '{}'\r", project.display()).as_bytes(),
    );
    let current = directory_view(&mut stream, &full_path(&project.canonicalize().unwrap()));
    assert_eq!(
        current.workspaces[0].terminals[0].directory,
        first.workspaces[0].directory
    );
    assert_eq!(
        current.workspaces[0].terminals[1].directory,
        current.workspaces[0].directory
    );
    assert_eq!(current.workspaces[0].terminals[0].initial_cwd, initial_cwd);
    assert_eq!(current.workspaces[0].terminals[1].initial_cwd, initial_cwd);

    // Another window starts on the first tab. Workspace labels differ, but each terminal's
    // directory is the same on both connections, regardless of their selected tab.
    let mut other = service.attach(80, 24);
    let other_view = next_view(&mut other);
    assert_eq!(other_view.active_terminal, Some(first_id));
    assert_eq!(other_view.workspaces[0].name, "Documents");
    assert_eq!(current.workspaces[0].name, "project");
    assert_eq!(
        other_view.workspaces[0].terminals,
        current.workspaces[0].terminals
    );

    // This input is addressed to the unselected, silent first terminal. Its directory change
    // must still publish a catalog update to the window viewing the second terminal.
    input(
        &mut stream,
        4,
        first_id,
        format!("cd '{}'\r", moved.display()).as_bytes(),
    );
    let moved_path = full_path(&moved.canonicalize().unwrap());
    let updated = view(until(&mut stream, |event| {
        matches!(event, Event::View { state }
        if state.workspaces[0].terminals[0].directory == moved_path)
    }));
    assert_eq!(updated.active_terminal, Some(second_id));
    assert_eq!(
        updated.workspaces[0].directory,
        current.workspaces[0].directory
    );
    assert_eq!(
        updated.workspaces[0].terminals[1].directory,
        current.workspaces[0].terminals[1].directory
    );
    assert_eq!(updated.workspaces[0].terminals[0].initial_cwd, initial_cwd);
}

#[test]
fn automatic_label_updates_without_shell_output_or_another_command() {
    let service = Service::start();
    let cwd = service.dir.join("project with spaces 日本語");
    fs::create_dir(&cwd).unwrap();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let terminal = initial.active_terminal.unwrap();
    assert!(initial.workspaces[0].custom_name.is_none());
    // Disable echo and prompts: there is no PTY output when the next cd runs.
    input(
        &mut stream,
        1,
        terminal,
        b"stty -echo; PS1=''; printf 'silent-ready-%s\\n' yes\r",
    );
    let ready = view(until(&mut stream, |event| {
        contains(event, "silent-ready-yes")
    }));
    input(
        &mut stream,
        2,
        terminal,
        format!("cd '{}'\r", cwd.display()).as_bytes(),
    );
    let updated = directory_view(&mut stream, &full_path(&cwd.canonicalize().unwrap()));
    assert_eq!(updated.workspaces[0].name, "project with spaces 日本語");
    assert!(updated.workspaces[0].custom_name.is_none());
    assert_eq!(
        updated.workspaces[0].terminals[0].initial_cwd,
        initial.workspaces[0].terminals[0].initial_cwd
    );
    assert_eq!(
        text(&updated.terminal.unwrap().screen),
        text(&ready.terminal.unwrap().screen)
    );
}

#[test]
fn labels_follow_each_clients_selected_terminal_and_closing_uses_the_fallback() {
    let service = Service::start();
    let alpha = service.dir.join("alpha");
    let beta = service.dir.join("beta");
    let gamma = service.dir.join("gamma");
    for directory in [&alpha, &beta, &gamma] {
        fs::create_dir(directory).unwrap();
    }
    let mut first = service.attach(80, 24);
    let initial = next_view(&mut first);
    let workspace_id = initial.active_workspace;
    let alpha_terminal = initial.active_terminal.unwrap();
    input(
        &mut first,
        1,
        alpha_terminal,
        format!("cd '{}'\r", alpha.display()).as_bytes(),
    );
    directory_view(&mut first, &full_path(&alpha.canonicalize().unwrap()));
    let mut second = service.attach(80, 24);
    assert_eq!(next_view(&mut second).workspaces[0].name, "alpha");
    command(
        &mut first,
        2,
        CoreCommand::CreateTerminal {
            workspace_id,
            cwd: Some(beta.to_string_lossy().into_owned()),
        },
    );
    let created = ack(&mut first, 2);
    let beta_terminal = created.active_terminal.unwrap();
    assert_eq!(created.workspaces[0].name, "beta");
    let other = view(until(
        &mut second,
        |event| matches!(event, Event::View { state } if state.workspaces[0].terminals.len() == 2),
    ));
    assert_eq!(other.active_terminal, Some(alpha_terminal));
    assert_eq!(other.workspaces[0].name, "alpha");
    input(
        &mut first,
        3,
        beta_terminal,
        format!("cd '{}'\r", gamma.display()).as_bytes(),
    );
    assert_eq!(
        directory_view(&mut first, &full_path(&gamma.canonicalize().unwrap())).workspaces[0].name,
        "gamma"
    );
    command(
        &mut second,
        1,
        CoreCommand::SelectTerminal {
            terminal_id: beta_terminal,
        },
    );
    assert_eq!(ack(&mut second, 1).workspaces[0].name, "gamma");
    command(
        &mut first,
        4,
        CoreCommand::SelectTerminal {
            terminal_id: alpha_terminal,
        },
    );
    assert_eq!(ack(&mut first, 4).workspaces[0].name, "alpha");
    command(
        &mut first,
        5,
        CoreCommand::CloseTerminal {
            terminal_id: alpha_terminal,
        },
    );
    let closed = ack(&mut first, 5);
    assert_eq!(closed.active_terminal, Some(beta_terminal));
    assert_eq!(closed.workspaces[0].name, "gamma");
}

#[test]
fn custom_names_stay_fixed_but_the_hover_path_updates_and_blank_restores_tracking() {
    let service = Service::start();
    let alpha = service.dir.join("alpha");
    let beta = service.dir.join("beta");
    fs::create_dir(&alpha).unwrap();
    fs::create_dir(&beta).unwrap();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let terminal = initial.active_terminal.unwrap();
    let workspace_id = initial.active_workspace;
    command(
        &mut stream,
        1,
        CoreCommand::RenameWorkspace {
            workspace_id,
            name: "Pinned".into(),
        },
    );
    assert_eq!(
        ack(&mut stream, 1).workspaces[0].custom_name.as_deref(),
        Some("Pinned")
    );
    input(
        &mut stream,
        2,
        terminal,
        format!("cd '{}'\r", alpha.display()).as_bytes(),
    );
    let updated = directory_view(&mut stream, &full_path(&alpha.canonicalize().unwrap()));
    assert_eq!(updated.workspaces[0].name, "Pinned");
    command(
        &mut stream,
        3,
        CoreCommand::RenameWorkspace {
            workspace_id,
            name: "  ".into(),
        },
    );
    let automatic = ack(&mut stream, 3);
    assert_eq!(automatic.workspaces[0].name, "alpha");
    assert!(automatic.workspaces[0].custom_name.is_none());
    input(
        &mut stream,
        4,
        terminal,
        format!("cd '{}'\r", beta.display()).as_bytes(),
    );
    assert_eq!(
        directory_view(&mut stream, &full_path(&beta.canonicalize().unwrap())).workspaces[0].name,
        "beta"
    );
}

#[test]
fn an_exited_shell_keeps_its_last_known_directory() {
    let service = Service::start();
    let cwd = service.dir.join("last-directory");
    fs::create_dir(&cwd).unwrap();
    let mut stream = service.attach(80, 24);
    let terminal = next_view(&mut stream).active_terminal.unwrap();
    input(
        &mut stream,
        1,
        terminal,
        format!("cd '{}'\r", cwd.display()).as_bytes(),
    );
    let updated = directory_view(&mut stream, &full_path(&cwd.canonicalize().unwrap()));
    input(&mut stream, 2, terminal, b"exit\r");
    let exited = view(until(
        &mut stream,
        |event| matches!(event, Event::View { state } if state.terminal.as_ref().is_some_and(|terminal| terminal.exited.is_some())),
    ));
    assert_eq!(exited.workspaces[0].name, updated.workspaces[0].name);
    assert_eq!(
        exited.workspaces[0].directory,
        updated.workspaces[0].directory
    );
    assert_eq!(
        exited.workspaces[0].terminals[0].directory,
        updated.workspaces[0].terminals[0].directory
    );
}
