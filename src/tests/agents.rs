use super::*;
use griz_protocol::{AgentInfo, AgentStatus, ProgramStatusRecord};

fn agent(id: &str, status: AgentStatus, unread: bool) -> AgentInfo {
    AgentInfo {
        record: ProgramStatusRecord {
            id: id.into(),
            status,
            app: "pi".into(),
            title: String::new(),
            message: String::new(),
            kind: None,
            progress: None,
            revision: 4,
        },
        generation: 1,
        unread,
    }
}

fn agent_state() -> (State, Receiver<Request>) {
    let (mut state, outgoing) = mock();
    let snapshot = state.snapshot.as_mut().unwrap();
    snapshot.workspaces[0].terminals[1].agents = vec![agent("", AgentStatus::Working, false)];
    snapshot.workspaces[1].terminals[0].agents = vec![agent("", AgentStatus::Done, true)];
    (state, outgoing)
}

#[test]
fn agents_are_listed_across_workspaces_below_workspaces_and_within_the_sidebar() {
    let (state, _) = agent_state();
    let rt = make_runtime(state, (1100.0, 720.0));
    for key in [
        "agents-section",
        "agent-2",
        "agent-3",
        "agent-2-status-border",
        "agent-3-status-border",
        "agent-notifications",
    ] {
        assert!(rt.find(key).is_some(), "missing {key}");
    }
    assert!(rt.find("agent-1").is_none());
    let (rect, _) = rt.presented("agents-section").unwrap();
    let (workspace, _) = rt.presented("workspace-list").unwrap();
    let (sidebar, _) = rt.presented("sidebar").unwrap();
    assert!(rect[1] >= workspace[1] + workspace[3]);
    assert!(rect[1] + rect[3] <= sidebar[1] + sidebar[3]);
    let count = rt
        .elements()
        .iter()
        .find(|e| e.key.as_deref() == Some("agent-count"))
        .unwrap()
        .text
        .clone();
    assert_eq!(count.as_deref(), Some("2"));
}

#[test]
fn clicking_an_agent_acknowledges_only_its_revision_and_gates_input_until_exact_pane_selection() {
    let (state, outgoing) = agent_state();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    rt.frame(0.0);
    commands(&outgoing);
    click(&mut rt, "agent-3");
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 3);
    assert!(
        matches!(&sent[0].1, Command::AcknowledgeAgent { terminal_id: 3, generation: 1, id, revision: 4 } if id.is_empty())
    );
    assert!(matches!(
        sent[1].1,
        Command::SelectWorkspace { workspace_id: 2 }
    ));
    assert!(matches!(
        sent[2].1,
        Command::SelectTerminal { terminal_id: 3 }
    ));
    let final_serial = sent[2].0;
    assert_eq!(rt.state.pending, Some(final_serial));
    let mut intermediate = snapshot();
    intermediate.ack = sent[1].0;
    intermediate.active_workspace = 2;
    intermediate.active_terminal = Some(3);
    rt.update(|s, _| s.apply(intermediate));
    assert_eq!(rt.state.pending, Some(final_serial));
    assert!(!rt.state.ready());
}

#[test]
fn clicking_an_agent_in_a_split_targets_the_leaf_not_the_tab() {
    let (mut state, outgoing) = agent_state();
    let snapshot = state.snapshot.as_mut().unwrap();
    snapshot.workspaces[0].tabs = vec![TabInfo {
        id: 1,
        layout: griz_protocol::PaneLayout::Split {
            direction: SplitDirection::Vertical,
            first: Box::new(griz_protocol::PaneLayout::Leaf { terminal_id: 1 }),
            second: Box::new(griz_protocol::PaneLayout::Leaf { terminal_id: 2 }),
        },
    }];
    let mut rt = make_runtime(state, (1100.0, 720.0));
    rt.frame(0.0);
    commands(&outgoing);
    click(&mut rt, "agent-2");
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 2);
    assert!(matches!(
        sent[1].1,
        Command::SelectTerminal { terminal_id: 2 }
    ));
}

#[test]
fn live_agent_updates_remove_notifications_and_closed_agents_without_changing_selection() {
    let (state, _) = agent_state();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    rt.update(|s, _| {
        let mut next = s.snapshot.clone().unwrap();
        next.workspaces[1].terminals[0].agents.clear();
        next.workspaces[0].terminals[1].agents[0].record.status = AgentStatus::Idle;
        s.apply(next);
    });
    rt.frame(0.0);
    assert!(rt.find("agent-3").is_none());
    assert!(rt.find("agent-notifications").is_none());
    assert!(rt.find("agent-group-idle").is_none());
    assert!(rt.find("agent-2-status-border").is_none());
    assert_eq!(rt.state.active_terminal(), Some(1));
    rt.update(|s, _| {
        s.online = false;
    });
    rt.frame(0.0);
    assert!(rt.find("agent-2").is_some());
}

#[test]
fn stale_clicks_and_disconnected_agents_do_not_send_commands() {
    let (state, outgoing) = agent_state();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    rt.frame(0.0);
    commands(&outgoing);
    // The visible callback belongs to the previous catalog; the agent just closed.
    rt.state.snapshot.as_mut().unwrap().workspaces[1].terminals[0]
        .agents
        .clear();
    click(&mut rt, "agent-3");
    assert!(commands(&outgoing).is_empty());
    rt.state.online = false;
    rt.frame(0.0);
    click(&mut rt, "agent-2");
    assert!(commands(&outgoing).is_empty());
}

#[test]
fn agents_section_scrolls_and_fits_the_sidebar_in_small_windows_at_large_fonts() {
    let (mut state, _) = agent_state();
    state.settings.font_size = 22.0;
    let rt = make_runtime(state, (640.0, 480.0));
    let (section, _) = rt.presented("agents-section").unwrap();
    let (list, _) = rt.presented("agent-list").unwrap();
    let (sidebar, _) = rt.presented("sidebar").unwrap();
    assert!(list[3] > 0.0);
    assert!(list[1] >= section[1]);
    assert!(section[1] + section[3] <= sidebar[1] + sidebar[3] + 0.1);
    assert!(sidebar[1] + sidebar[3] <= 480.0);
}

#[test]
fn child_records_have_unique_keys_show_safe_metadata_and_acknowledge_the_exact_id() {
    let (mut state, outgoing) = agent_state();
    let mut child = agent("worker/test", AgentStatus::Blocked, true);
    child.record.title = "Tests\u{202e}\u{200b}".into();
    child.record.message = "Approve <run>\u{2066}?".into();
    child.record.kind = Some(griz_protocol::ProgramStatusKind::Permission);
    child.record.progress = Some(40);
    state.snapshot.as_mut().unwrap().workspaces[0].terminals[1]
        .agents
        .push(child);
    let mut rt = make_runtime(state, (1100.0, 1000.0));
    rt.frame(0.0);
    assert!(rt.find("agent-2").is_some());
    assert!(rt.find("agent-2:worker/test").is_some());
    let texts: Vec<_> = rt
        .elements()
        .iter()
        .filter_map(|e| e.text.clone())
        .collect();
    assert!(texts.iter().any(|s| s == "Tests"));
    assert!(texts.iter().any(|s| s == "Approve <run>? · 40%"));
    assert!(
        texts
            .iter()
            .all(|s| !s.contains(['\u{202e}', '\u{200b}', '\u{2066}']))
    );
    commands(&outgoing);
    click(&mut rt, "agent-2:worker/test");
    let sent = commands(&outgoing);
    assert!(matches!(&sent[0].1, Command::AcknowledgeAgent {
        terminal_id: 2, generation: 1, id, revision: 4,
    } if id == "worker/test"));
    assert!(matches!(
        sent[1].1,
        Command::SelectTerminal { terminal_id: 2 }
    ));
}

#[test]
fn stale_shell_generation_clicks_do_not_acknowledge_or_focus_a_replacement_record() {
    let (state, outgoing) = agent_state();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    rt.frame(0.0);
    commands(&outgoing);
    rt.state.snapshot.as_mut().unwrap().workspaces[1].terminals[0].agents[0].generation = 2;
    click(&mut rt, "agent-3");
    assert!(commands(&outgoing).is_empty());
}

#[test]
fn agents_empty_state_is_visible_and_hidden_with_the_sidebar() {
    let mut rt = runtime((1100.0, 720.0));
    assert!(rt.find("agents-empty").is_some());
    click(&mut rt, "toggle-sidebar");
    assert!(rt.find("agents-section").is_none());
}

#[test]
fn focusing_an_already_selected_terminal_clears_only_that_panes_notifications() {
    for key in ["terminal", "terminal-header", "tab-1"] {
        let (mut state, outgoing) = agent_state();
        state.snapshot.as_mut().unwrap().workspaces[0].terminals[0].agents =
            vec![agent("", AgentStatus::Done, true)];
        let mut rt = make_runtime(state, (1100.0, 720.0));
        rt.frame(0.0);
        commands(&outgoing);
        click(&mut rt, key);
        let sent = commands(&outgoing);
        let acknowledgements: Vec<_> = sent
            .iter()
            .filter(|(_, c)| matches!(c, Command::AcknowledgeAgent { .. }))
            .collect();
        assert_eq!(acknowledgements.len(), 1, "{key}");
        assert!(
            matches!(&acknowledgements[0].1, Command::AcknowledgeAgent { terminal_id: 1, generation: 1, id, revision: 4 } if id.is_empty())
        );
    }
}

#[test]
fn switching_workspace_acknowledges_only_confirmed_focused_pane() {
    let (mut state, outgoing) = agent_state();
    commands(&outgoing);
    state.change(Command::SelectWorkspace { workspace_id: 2 });
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 1);
    let mut next = state.snapshot.clone().unwrap();
    next.active_workspace = 2;
    next.active_terminal = Some(3);
    // A stale, intermediate snapshot must not acknowledge anything.
    state.apply(next.clone());
    assert!(commands(&outgoing).is_empty());
    next.ack = sent[0].0;
    state.apply(next.clone());
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 1);
    assert!(
        matches!(&sent[0].1, Command::AcknowledgeAgent { terminal_id: 3, generation: 1, id, revision: 4 } if id.is_empty())
    );
    // A background status update is not a focus action.
    state.apply(next);
    assert!(commands(&outgoing).is_empty());
}

#[test]
fn switching_tabs_or_split_panes_acknowledges_the_selected_terminal_only() {
    let (mut state, outgoing) = agent_state();
    state.snapshot.as_mut().unwrap().workspaces[0].terminals[1].agents =
        vec![agent("", AgentStatus::Done, true)];
    commands(&outgoing);
    state.change(Command::SelectTerminal { terminal_id: 2 });
    let selection = commands(&outgoing);
    let mut next = state.snapshot.clone().unwrap();
    next.active_terminal = Some(2);
    next.ack = selection[0].0;
    state.apply(next);
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 1);
    assert!(matches!(
        sent[0].1,
        Command::AcknowledgeAgent { terminal_id: 2, .. }
    ));
}

#[test]
fn focusing_does_not_acknowledge_read_records_or_offline_or_modal_views() {
    let (mut state, outgoing) = agent_state();
    commands(&outgoing);
    state.acknowledge_focused_agents();
    assert!(commands(&outgoing).is_empty());
    state.snapshot.as_mut().unwrap().workspaces[0].terminals[0].agents =
        vec![agent("", AgentStatus::Done, true)];
    state.settings_open = true;
    state.acknowledge_focused_agents();
    state.settings_open = false;
    state.online = false;
    state.acknowledge_focused_agents();
    assert!(commands(&outgoing).is_empty());
}

#[test]
fn agent_cards_replace_group_headings_with_border_only_motion() {
    let (state, _) = agent_state();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    rt.frame(0.0);
    assert!(
        rt.elements()
            .iter()
            .all(|e| !e.key.as_deref().unwrap_or("").starts_with("agent-group-"))
    );
    let inspect = |rt: &Runtime<State>, key: &str| {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap()
    };
    let working = inspect(&rt, "agent-2-status-border");
    let card = inspect(&rt, "agent-2");
    let done = inspect(&rt, "agent-3-status-border");
    assert!(working.loop_phase.is_some());
    assert!(done.loop_phase.is_none());
    assert!(!working.focusable);
    assert_eq!(working.rect, card.rect);
    for _ in 0..84 {
        rt.frame(1.0 / 60.0);
    }
    let later = inspect(&rt, "agent-2-status-border");
    assert!((later.opacity - working.opacity).abs() > 0.1);
    assert_eq!(inspect(&rt, "agent-2").opacity, card.opacity);
    assert_eq!(inspect(&rt, "agent-2").rect, card.rect);
    assert_eq!(inspect(&rt, "agent-3-status-border").opacity, done.opacity);
    rt.reduced_motion = true;
    rt.frame(1.0 / 60.0);
    let still = inspect(&rt, "agent-2-status-border").opacity;
    for _ in 0..84 {
        rt.frame(1.0 / 60.0);
    }
    assert_eq!(inspect(&rt, "agent-2-status-border").opacity, still);
}
