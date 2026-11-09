use super::*;
use griz_protocol::AgentStatus;

fn report(stream: &mut UnixStream, serial: u64, terminal_id: u64, body: &str) {
    input(
        stream,
        serial,
        terminal_id,
        format!("printf '\\033]7501;{body}\\007'\r").as_bytes(),
    );
}

fn agent_view(stream: &mut UnixStream, terminal_id: u64, expected: AgentStatus) -> ViewState {
    view(until(stream, |event| {
        matches!(event, Event::View { state } if state.workspaces.iter().flat_map(|w| &w.terminals)
        .any(|t| t.id == terminal_id && t.agents.iter().any(|a| a.record.status == expected)))
    }))
}

#[test]
fn osc_reports_publish_hidden_catalog_to_all_clients_and_survive_detachment() {
    let service = Service::start();
    let mut first = service.attach(80, 24);
    let initial = next_view(&mut first);
    let terminal_id = initial.active_terminal.unwrap();
    let mut second = service.attach(80, 24);
    next_view(&mut second);
    command(
        &mut first,
        1,
        CoreCommand::CreateWorkspace {
            name: "Elsewhere".into(),
        },
    );
    let elsewhere = ack(&mut first, 1);
    command(
        &mut second,
        1,
        CoreCommand::SelectWorkspace {
            workspace_id: elsewhere.active_workspace,
        },
    );
    ack(&mut second, 1);

    // No client observes this pane; protocol-only changes must still publish.
    report(&mut first, 2, terminal_id, "state=working:app=example");
    let working = agent_view(&mut first, terminal_id, AgentStatus::Working);
    assert_eq!(working.active_workspace, elsewhere.active_workspace);
    assert_eq!(working.active_terminal, elsewhere.active_terminal);
    assert_eq!(
        agent_view(&mut second, terminal_id, AgentStatus::Working).active_terminal,
        elsewhere.active_terminal
    );
    input(
        &mut first,
        3,
        terminal_id,
        b"sleep 0.15; printf '\\033]7501;state=done:app=example\\007'\r",
    );
    ack(&mut first, 3);
    drop(first);
    drop(second);
    std::thread::sleep(Duration::from_millis(300));
    let mut reattached = service.attach(80, 24);
    let done = agent_view(&mut reattached, terminal_id, AgentStatus::Done);
    let agent = &done.workspaces[0].terminals[0].agents[0];
    assert!(agent.unread);
    let revision = agent.record.revision;
    command(
        &mut reattached,
        1,
        CoreCommand::AcknowledgeAgent {
            terminal_id,
            generation: 1,
            id: "".into(),
            revision,
        },
    );
    assert!(!ack(&mut reattached, 1).workspaces[0].terminals[0].agents[0].unread);

    report(&mut reattached, 2, terminal_id, "state=working:app=example");
    agent_view(&mut reattached, terminal_id, AgentStatus::Working);
    report(
        &mut reattached,
        3,
        terminal_id,
        "state=blocked:kind=permission:app=example:msg=QXBwbHk/",
    );
    let blocked = agent_view(&mut reattached, terminal_id, AgentStatus::Blocked);
    assert_eq!(
        blocked.workspaces[0].terminals[0].agents[0].record.message,
        "Apply?"
    );
    command(
        &mut reattached,
        4,
        CoreCommand::AcknowledgeAgent {
            terminal_id,
            generation: 1,
            id: "".into(),
            revision,
        },
    );
    assert!(ack(&mut reattached, 4).workspaces[0].terminals[0].agents[0].unread);
    report(&mut reattached, 5, terminal_id, "state=clear");
    let cleared = view(until(
        &mut reattached,
        |event| matches!(event, Event::View { state } if state.ack >= 5 && state.workspaces[0].terminals[0].agents.is_empty()),
    ));
    assert_eq!(cleared.active_terminal, Some(terminal_id));
}

#[test]
fn finished_results_survive_shell_exit_and_stale_generation_acks_cannot_clear_new_results() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let terminal_id = next_view(&mut stream).active_terminal.unwrap();
    input(
        &mut stream,
        1,
        terminal_id,
        b"printf '\\033]7501;state=done:app=example\\007'; exit\r",
    );
    let exited = view(until(&mut stream, |event| {
        matches!(event, Event::View { state }
        if state.terminal.as_ref().is_some_and(|t| t.exited.is_some())
        && state.workspaces[0].terminals[0].agents.iter().any(|a| a.record.status == AgentStatus::Done))
    }));
    assert!(exited.workspaces[0].terminals[0].agents[0].unread);
    command(&mut stream, 2, CoreCommand::StartShell { terminal_id });
    assert!(
        ack(&mut stream, 2).workspaces[0].terminals[0]
            .agents
            .is_empty()
    );
    report(&mut stream, 3, terminal_id, "state=done:app=example");
    let fresh = agent_view(&mut stream, terminal_id, AgentStatus::Done);
    let revision = fresh.workspaces[0].terminals[0].agents[0].record.revision;
    assert_eq!(fresh.workspaces[0].terminals[0].agents[0].generation, 2);
    command(
        &mut stream,
        4,
        CoreCommand::AcknowledgeAgent {
            terminal_id,
            generation: 1,
            id: "".into(),
            revision,
        },
    );
    assert!(ack(&mut stream, 4).workspaces[0].terminals[0].agents[0].unread);
    command(
        &mut stream,
        5,
        CoreCommand::AcknowledgeAgent {
            terminal_id,
            generation: 2,
            id: "".into(),
            revision,
        },
    );
    assert!(!ack(&mut stream, 5).workspaces[0].terminals[0].agents[0].unread);
    command(&mut stream, 6, CoreCommand::CloseTerminal { terminal_id });
    assert!(ack(&mut stream, 6).workspaces.is_empty());
}

#[test]
fn hierarchical_records_replace_fields_inherit_apps_and_expire_only_transient_states_at_prompt() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let terminal_id = next_view(&mut stream).active_terminal.unwrap();
    input(&mut stream, 1, terminal_id, b"printf '\\033]7501;state=working:app=deploy\\007\\033]7501;state=working:id=build:app=builder\\007\\033]7501;state=done:id=build/test:title=VGVzdHM=:msg=UGFzc2Vk\\007'\r");
    let done = agent_view(&mut stream, terminal_id, AgentStatus::Done);
    let agents = &done.workspaces[0].terminals[0].agents;
    assert_eq!(agents.len(), 3);
    let child = agents.iter().find(|a| a.record.id == "build/test").unwrap();
    assert_eq!(child.record.app, "builder");
    assert_eq!(child.record.title, "Tests");
    assert_eq!(child.record.message, "Passed");
    input(&mut stream, 2, terminal_id, b"printf '\\033]133;A\\007'\r");
    let prompt = view(until(&mut stream, |event| {
        matches!(event, Event::View { state } if state.ack >= 2
        && state.workspaces[0].terminals[0].agents.len() == 1)
    }));
    assert_eq!(
        prompt.workspaces[0].terminals[0].agents[0].record.id,
        "build/test"
    );
    assert!(
        prompt.workspaces[0].terminals[0].agents[0]
            .record
            .app
            .is_empty()
    );
    report(&mut stream, 3, terminal_id, "state=clear:id=build");
    until(
        &mut stream,
        |event| matches!(event, Event::View { state } if state.ack >= 3 && state.workspaces[0].terminals[0].agents.is_empty()),
    );
}

#[test]
fn capability_negotiation_reaches_the_program_without_extensions_or_custom_ipc() {
    let service = Service::start();
    let mut stream = service.attach(300, 24);
    let terminal_id = next_view(&mut stream).active_terminal.unwrap();
    // Read the actual PTY reply, not just the wrapper's response buffer. This
    // follows Pi's query + DA startup negotiation with only standard shell tools.
    input(&mut stream, 1, terminal_id, br#"stty -echo -icanon min 0 time 5; printf '\033]7501;?\033\\\033[c'; reply=$(dd bs=1024 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); stty sane; printf 'PSP-REPLY:%s\n' "$reply"
"#);
    until(&mut stream, |event| {
        contains(event, "PSP-REPLY:1b5d373530313b3f3a7374617465733d")
    });
}
