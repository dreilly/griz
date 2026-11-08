use super::*;

fn label(path: &std::path::Path) -> String {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    match path.strip_prefix(home) {
        Ok(relative) if relative.as_os_str().is_empty() => "~".into(),
        Ok(relative) => format!("~/{}", relative.display()),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

#[test]
fn unnamed_workspace_uses_live_shell_directory_and_starts_its_terminal_there() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    assert_eq!(initial.workspaces[0].name, label(&service.dir));
    let terminal = initial.active_terminal.unwrap();
    let cwd = service.dir.join("code with spaces");
    fs::create_dir(&cwd).unwrap();
    let script = format!(
        "cd '{}'; printf 'directory-ready-%s\\n' yes\r",
        cwd.display()
    );
    input(&mut stream, 1, terminal, script.as_bytes());
    until(&mut stream, |event| contains(event, "directory-ready-yes"));
    command(
        &mut stream,
        2,
        CoreCommand::CreateWorkspace { name: "   ".into() },
    );
    let created = ack(&mut stream, 2);
    let cwd = cwd.canonicalize().unwrap();
    let workspace = created
        .workspaces
        .iter()
        .find(|w| w.id == created.active_workspace)
        .unwrap();
    assert_eq!(workspace.name, label(&cwd));
    assert_eq!(workspace.terminals[0].initial_cwd, cwd.to_string_lossy());
    input(
        &mut stream,
        3,
        created.active_terminal.unwrap(),
        b"printf 'inherited-directory:<%s>\\n' \"${PWD##*/}\"\r",
    );
    until(&mut stream, |event| {
        contains(event, "inherited-directory:<code with spaces>")
    });
}

#[test]
fn blank_rename_uses_the_target_workspaces_directory_not_the_active_workspace() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let original = next_view(&mut stream).active_workspace;
    command(
        &mut stream,
        1,
        CoreCommand::CreateWorkspace {
            name: "  Custom name  ".into(),
        },
    );
    let created = ack(&mut stream, 1);
    let workspace_id = created.active_workspace;
    assert_eq!(created.workspaces[1].name, "Custom name");
    let cwd = service.dir.join("other-project");
    fs::create_dir(&cwd).unwrap();
    let script = format!("cd '{}'; printf 'rename-ready-%s\\n' yes\r", cwd.display());
    input(
        &mut stream,
        2,
        created.active_terminal.unwrap(),
        script.as_bytes(),
    );
    until(&mut stream, |event| contains(event, "rename-ready-yes"));
    command(
        &mut stream,
        3,
        CoreCommand::SelectWorkspace {
            workspace_id: original,
        },
    );
    ack(&mut stream, 3);
    command(
        &mut stream,
        4,
        CoreCommand::RenameWorkspace {
            workspace_id,
            name: "".into(),
        },
    );
    let renamed = ack(&mut stream, 4);
    assert_eq!(renamed.active_workspace, original);
    assert_eq!(
        renamed.workspaces[1].name,
        label(&cwd.canonicalize().unwrap())
    );
    assert_eq!(renamed.workspaces[0].name, label(&service.dir));
}

#[test]
fn unnamed_workspace_falls_back_to_default_directory_without_a_selected_terminal() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    command(
        &mut stream,
        1,
        CoreCommand::CloseTerminal {
            terminal_id: initial.active_terminal.unwrap(),
        },
    );
    let empty = ack(&mut stream, 1);
    assert!(empty.terminal.is_none());
    assert!(empty.workspaces.is_empty());
    command(
        &mut stream,
        2,
        CoreCommand::CreateWorkspace { name: "".into() },
    );
    let created = ack(&mut stream, 2);
    assert_eq!(created.workspaces.len(), 1);
    assert_eq!(created.workspaces[0].name, label(&service.dir));
    assert_eq!(
        created.workspaces[0].terminals[0].initial_cwd,
        service.dir.to_string_lossy()
    );
}
