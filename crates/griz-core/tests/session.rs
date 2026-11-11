//! Exercise the real service, PTYs, shells, terminal parser, and local IPC.
use griz_protocol::{
    Command as CoreCommand, Event, MouseAction, MouseButton, MouseInput, PaneLayout, Request,
    Screen, SplitDirection, ViewState, receive, send,
};
use std::fs;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "session/accents.rs"]
mod accents;
#[path = "session/agents.rs"]
mod agents;
#[path = "session/input_latency.rs"]
mod input_latency;
#[path = "session/mouse.rs"]
mod mouse;
#[path = "session/naming.rs"]
mod naming;
#[path = "session/previews.rs"]
mod previews;
#[path = "session/scrollback.rs"]
mod scrollback;
#[path = "session/splits.rs"]
mod splits;
#[path = "session/workspaces.rs"]
mod workspaces;

static NEXT_SERVICE: AtomicUsize = AtomicUsize::new(0);

struct Service {
    process: Child,
    dir: PathBuf,
    path: PathBuf,
}

impl Service {
    fn start() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let index = NEXT_SERVICE.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "griz-core-test-{}-{unique:x}-{index}",
            std::process::id()
        ));
        fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let path = dir.join("core.sock");
        let process = Command::new(env!("CARGO_BIN_EXE_griz-core"))
            .args(["--shell", "/bin/sh", "--cwd", dir.to_str().unwrap()])
            .env("GRIZ_SOCKET", &path)
            .env("PS1", "griz$ ")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut service = Self { process, dir, path };
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if service.path.exists() {
                return service;
            }
            assert!(
                service.process.try_wait().unwrap().is_none(),
                "service exited during startup"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("service failed to bind");
    }

    fn attach(&self, cols: u16, rows: u16) -> UnixStream {
        let mut stream = UnixStream::connect(&self.path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(4)))
            .unwrap();
        send(&mut stream, &Request::Attach { cols, rows }).unwrap();
        stream
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        if let Ok(mut stream) = UnixStream::connect(&self.path) {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
            let _ = send(&mut stream, &Request::Shutdown);
            let _ = receive::<Event>(&mut stream);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if self.process.try_wait().ok().flatten().is_some() {
                let _ = fs::remove_dir_all(&self.dir);
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.process.kill();
        let _ = self.process.wait();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn command(stream: &mut UnixStream, serial: u64, command: CoreCommand) {
    send(stream, &Request::Command { serial, command }).unwrap();
}

fn input(stream: &mut UnixStream, serial: u64, terminal_id: u64, bytes: &[u8]) {
    command(
        stream,
        serial,
        CoreCommand::Input {
            terminal_id,
            data: bytes.to_vec(),
        },
    );
}

fn text(screen: &Screen) -> String {
    screen
        .cells
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| {
                    if cell.text.is_empty() {
                        " "
                    } else {
                        cell.text.as_str()
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn until(stream: &mut UnixStream, predicate: impl Fn(&Event) -> bool) -> Event {
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut last = None;
    while Instant::now() < deadline {
        match receive::<Event>(stream) {
            Ok(event) => {
                if predicate(&event) {
                    return event;
                }
                last = Some(event);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(error) => panic!("IPC failed: {error}"),
        }
    }
    panic!("expected event did not arrive; last event: {last:?}");
}

fn view(event: Event) -> ViewState {
    let Event::View { state } = event else {
        panic!("expected View event")
    };
    state
}

fn next_view(stream: &mut UnixStream) -> ViewState {
    view(until(stream, |event| matches!(event, Event::View { .. })))
}

fn ack(stream: &mut UnixStream, serial: u64) -> ViewState {
    view(until(
        stream,
        |event| matches!(event, Event::View { state } if state.ack >= serial),
    ))
}

fn contains(event: &Event, needle: &str) -> bool {
    matches!(event, Event::View { state } if state.terminal.as_ref().is_some_and(|terminal| text(&terminal.screen).contains(needle)))
}

#[test]
fn terminals_are_isolated_and_keep_output_while_deselected_and_detached() {
    let service = Service::start();
    let other = service.dir.join("unrelated-directory");
    fs::create_dir(&other).unwrap();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let first_id = initial.active_terminal.unwrap();
    let first_pid = initial.terminal.unwrap().shell_pid;

    input(
        &mut stream,
        1,
        first_id,
        b"export GRIZ_ONLY=first; sleep 0.3; printf 'background-%s\\n' done\r",
    );
    ack(&mut stream, 1);
    command(
        &mut stream,
        2,
        CoreCommand::CreateTerminal {
            workspace_id: initial.active_workspace,
            cwd: Some(other.to_string_lossy().into_owned()),
        },
    );
    let second = ack(&mut stream, 2);
    let second_id = second.active_terminal.unwrap();
    assert_ne!(second_id, first_id);
    assert_ne!(second.terminal.as_ref().unwrap().shell_pid, first_pid);
    assert_eq!(
        second.workspaces[0].terminals[1].initial_cwd,
        other.to_string_lossy()
    );
    input(
        &mut stream,
        3,
        second_id,
        b"printf 'isolated:<%s> cwd:<%s>\\n' \"$GRIZ_ONLY\" \"$PWD\"\r",
    );
    until(&mut stream, |event| contains(event, "isolated:<>"));
    command(
        &mut stream,
        4,
        CoreCommand::SelectTerminal {
            terminal_id: first_id,
        },
    );
    until(&mut stream, |event| {
        matches!(event, Event::View { state } if state.ack >= 4 && state.active_terminal == Some(first_id))
            && contains(event, "background-done")
    });

    input(
        &mut stream,
        5,
        first_id,
        b"sleep 0.2; printf 'detached-%s\\n' output\r",
    );
    ack(&mut stream, 5);
    drop(stream);
    std::thread::sleep(Duration::from_millis(400));
    let mut reattached = service.attach(80, 24);
    until(&mut reattached, |event| contains(event, "detached-output"));
}

#[test]
fn workspace_catalog_and_each_clients_selection_are_independent() {
    let service = Service::start();
    let mut first = service.attach(80, 24);
    let mut second = service.attach(100, 30);
    let first_initial = next_view(&mut first);
    let second_initial = next_view(&mut second);
    let daily = first_initial.active_workspace;
    assert_eq!(daily, second_initial.active_workspace);

    command(
        &mut first,
        1,
        CoreCommand::CreateWorkspace {
            name: "  Project  ".into(),
        },
    );
    let created = ack(&mut first, 1);
    let project = created.active_workspace;
    assert_ne!(project, daily);
    assert_eq!(created.workspaces[1].name, "Project");
    let other_view = until(
        &mut second,
        |event| matches!(event, Event::View { state } if state.workspaces.len() == 2),
    );
    assert_eq!(view(other_view).active_workspace, daily);

    command(
        &mut first,
        2,
        CoreCommand::RenameWorkspace {
            workspace_id: project,
            name: "Renamed".into(),
        },
    );
    assert_eq!(ack(&mut first, 2).workspaces[1].name, "Renamed");
    let second_catalog = until(
        &mut second,
        |event| matches!(event, Event::View { state } if state.workspaces.iter().any(|workspace| workspace.name == "Renamed")),
    );
    assert_eq!(view(second_catalog).active_workspace, daily);

    command(
        &mut second,
        1,
        CoreCommand::SelectWorkspace {
            workspace_id: project,
        },
    );
    assert_eq!(ack(&mut second, 1).active_workspace, project);
    command(
        &mut first,
        3,
        CoreCommand::SelectWorkspace {
            workspace_id: daily,
        },
    );
    assert_eq!(ack(&mut first, 3).active_workspace, daily);
}

#[test]
fn resize_applies_to_selected_terminal_and_observer_does_not_resize() {
    let service = Service::start();
    let mut active = service.attach(91, 31);
    let initial = until(
        &mut active,
        |event| matches!(event, Event::View { state } if state.terminal.as_ref().is_some_and(|terminal| terminal.screen.cols == 91 && terminal.screen.rows == 31)),
    );
    let terminal_id = view(initial).active_terminal.unwrap();
    let mut observer = service.attach(0, 0);
    let observed = next_view(&mut observer);
    assert_eq!(observed.terminal.unwrap().screen.cols, 91);

    command(
        &mut active,
        1,
        CoreCommand::Resize {
            terminal_id,
            cols: 103,
            rows: 37,
        },
    );
    let resized = ack(&mut active, 1);
    assert_eq!(
        (
            resized.terminal.as_ref().unwrap().screen.cols,
            resized.terminal.as_ref().unwrap().screen.rows
        ),
        (103, 37)
    );
    input(&mut active, 2, terminal_id, b"stty size\r");
    until(&mut active, |event| contains(event, "37 103"));
}

#[test]
fn stale_ids_close_fallback_final_workspace_close_and_restart_rules() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let workspace_id = initial.active_workspace;
    let first_id = initial.active_terminal.unwrap();
    let first_pid = initial.terminal.unwrap().shell_pid;

    command(
        &mut stream,
        1,
        CoreCommand::CreateTerminal {
            workspace_id,
            cwd: None,
        },
    );
    let created = ack(&mut stream, 1);
    let second_id = created.active_terminal.unwrap();
    command(
        &mut stream,
        2,
        CoreCommand::StartShell {
            terminal_id: second_id,
        },
    );
    until(
        &mut stream,
        |event| matches!(event, Event::Error { serial: Some(2), message } if message.contains("still running")),
    );
    let live = ack(&mut stream, 2);
    assert_eq!(live.terminal.unwrap().id, second_id);

    command(
        &mut stream,
        3,
        CoreCommand::CloseTerminal {
            terminal_id: second_id,
        },
    );
    let fallback = ack(&mut stream, 3);
    assert_eq!(fallback.active_terminal, Some(first_id));
    assert_eq!(fallback.workspaces.len(), 1);
    assert_eq!(fallback.active_workspace, workspace_id);
    assert_eq!(fallback.workspaces[0].terminals.len(), 1);
    input(&mut stream, 4, second_id, b"printf wrong\r");
    until(
        &mut stream,
        |event| matches!(event, Event::Error { serial: Some(4), message } if message.contains("unknown terminal")),
    );
    assert_eq!(ack(&mut stream, 4).active_terminal, Some(first_id));

    input(&mut stream, 5, first_id, b"exit 7\r");
    let exited = until(
        &mut stream,
        |event| matches!(event, Event::View { state } if state.workspaces[0].terminals[0].exited.is_some()),
    );
    assert_eq!(view(exited).workspaces[0].terminals[0].shell_pid, first_pid);
    command(
        &mut stream,
        6,
        CoreCommand::StartShell {
            terminal_id: first_id,
        },
    );
    let restarted = ack(&mut stream, 6);
    assert_eq!(restarted.active_terminal, Some(first_id));
    assert_ne!(restarted.terminal.unwrap().shell_pid, first_pid);

    command(
        &mut stream,
        7,
        CoreCommand::CloseTerminal {
            terminal_id: first_id,
        },
    );
    let empty = ack(&mut stream, 7);
    assert_eq!(empty.active_terminal, None);
    assert!(empty.terminal.is_none());
    assert!(empty.workspaces.is_empty());
    assert_eq!(empty.active_workspace, 0);
}

#[test]
fn closing_a_terminal_kills_its_foreground_job_but_not_another_terminal() {
    let service = Service::start();
    let pid_file = service.dir.join("foreground.pid");
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let first_id = initial.active_terminal.unwrap();
    command(
        &mut stream,
        1,
        CoreCommand::CreateTerminal {
            workspace_id: initial.active_workspace,
            cwd: None,
        },
    );
    let second_id = ack(&mut stream, 1).active_terminal.unwrap();
    let script = format!("sh -c 'echo $$ > {}; exec sleep 30'\r", pid_file.display());
    input(&mut stream, 2, second_id, script.as_bytes());
    ack(&mut stream, 2);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !pid_file.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let foreground_pid: i32 = fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_ne!(foreground_pid, std::process::id() as i32);

    command(
        &mut stream,
        3,
        CoreCommand::CloseTerminal {
            terminal_id: second_id,
        },
    );
    assert_eq!(ack(&mut stream, 3).active_terminal, Some(first_id));
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        // SAFETY: signal 0 only checks the isolated child PID recorded above.
        if unsafe { libc::kill(foreground_pid, 0) } == -1
            && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // SAFETY: signal 0 does not modify the process.
    assert_eq!(unsafe { libc::kill(foreground_pid, 0) }, -1);
    input(&mut stream, 4, first_id, b"printf 'survivor-%s\\n' alive\r");
    until(&mut stream, |event| contains(event, "survivor-alive"));
}

#[test]
fn rejected_serial_still_gets_feedback_without_decreasing_ack() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    next_view(&mut stream);
    command(
        &mut stream,
        2,
        CoreCommand::CreateWorkspace {
            name: "x".repeat(81),
        },
    );
    until(&mut stream, |event| {
        matches!(
            event,
            Event::Error {
                serial: Some(2),
                ..
            }
        )
    });
    assert_eq!(ack(&mut stream, 2).ack, 2);
    command(
        &mut stream,
        1,
        CoreCommand::SelectWorkspace { workspace_id: 1 },
    );
    until(
        &mut stream,
        |event| matches!(event, Event::Error { serial: Some(1), message } if message.contains("not greater")),
    );
    assert_eq!(next_view(&mut stream).ack, 2);
}

#[test]
fn continuous_output_still_publishes_frames_and_accepts_close() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let terminal_id = next_view(&mut stream).active_terminal.unwrap();
    input(
        &mut stream,
        1,
        terminal_id,
        b"i=0; while :; do printf 'stream-%06d\\n' $i; i=$((i+1)); done\r",
    );
    let first = until(&mut stream, |event| {
        matches!(event, Event::View { state } if state.terminal.as_ref().is_some_and(|t| {
            text(&t.screen).lines().any(|line| line.starts_with("stream-") && line.as_bytes().get(7).is_some_and(u8::is_ascii_digit))
        }))
    });
    let first_screen = view(first).terminal.unwrap().screen;
    until(
        &mut stream,
        |event| matches!(event, Event::View { state } if state.terminal.as_ref().is_some_and(|t| t.screen != first_screen)),
    );
    command(&mut stream, 2, CoreCommand::CloseTerminal { terminal_id });
    assert!(ack(&mut stream, 2).terminal.is_none());
}

#[test]
fn invalid_creation_is_atomic_and_workspace_switch_remembers_the_selected_tab() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let workspace_id = initial.active_workspace;
    command(
        &mut stream,
        1,
        CoreCommand::CreateTerminal {
            workspace_id,
            cwd: Some("relative/path".into()),
        },
    );
    until(&mut stream, |e| {
        matches!(
            e,
            Event::Error {
                serial: Some(1),
                ..
            }
        )
    });
    assert_eq!(ack(&mut stream, 1).workspaces[0].terminals.len(), 1);
    command(
        &mut stream,
        2,
        CoreCommand::CreateWorkspace {
            name: "bad\u{7}name".into(),
        },
    );
    until(&mut stream, |e| {
        matches!(
            e,
            Event::Error {
                serial: Some(2),
                ..
            }
        )
    });
    assert_eq!(ack(&mut stream, 2).workspaces.len(), 1);
    command(
        &mut stream,
        3,
        CoreCommand::CreateTerminal {
            workspace_id,
            cwd: None,
        },
    );
    let selected = ack(&mut stream, 3).active_terminal;
    command(
        &mut stream,
        4,
        CoreCommand::CreateWorkspace {
            name: "Independent".into(),
        },
    );
    assert_ne!(ack(&mut stream, 4).active_workspace, workspace_id);
    command(
        &mut stream,
        5,
        CoreCommand::SelectWorkspace { workspace_id },
    );
    assert_eq!(ack(&mut stream, 5).active_terminal, selected);
}

#[test]
fn commands_before_attach_and_second_instance_are_rejected() {
    let service = Service::start();
    let status = Command::new(env!("CARGO_BIN_EXE_griz-core"))
        .env("GRIZ_SOCKET", &service.path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success());

    let mut stream = UnixStream::connect(&service.path).unwrap();
    send(
        &mut stream,
        &Request::Command {
            serial: 1,
            command: CoreCommand::SelectWorkspace { workspace_id: 1 },
        },
    )
    .unwrap();
    assert!(matches!(
        receive::<Event>(&mut stream).unwrap(),
        Event::Error { serial: None, .. }
    ));
}
