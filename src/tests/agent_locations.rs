use super::agents::agent;
use super::*;
use griz_protocol::{AgentStatus, PaneLayout};

fn assert_location(rt: &Runtime<State>, key: &str, expected: &str, path: &str) {
    let elements = rt.elements();
    let index = elements
        .iter()
        .position(|e| e.key.as_deref() == Some(key))
        .unwrap();
    let subtitle = elements
        .iter()
        .find(|e| e.parent == Some(index) && e.text.is_some())
        .unwrap();
    assert_eq!(subtitle.text.as_deref(), Some(expected), "{key}");
    assert!(elements[index].label.as_deref().unwrap().contains(expected));
    assert!(elements[index].help.as_deref().unwrap().contains(path));
}

fn location_state(split: bool) -> State {
    let (mut state, _) = mock();
    let snapshot = state.snapshot.as_mut().unwrap();
    let workspace = &mut snapshot.workspaces[0];
    workspace.custom_name = None;
    workspace.name = "frontend".into();
    workspace.directory = "~/code/frontend".into();
    workspace.terminals[0].directory = "~/code/frontend".into();
    workspace.terminals[1].directory = "~/code/backend".into();
    workspace.terminals[0].agents = vec![
        agent("", AgentStatus::Working, false),
        agent("worker", AgentStatus::Idle, false),
    ];
    workspace.terminals[1].agents = vec![agent("", AgentStatus::Working, false)];
    if split {
        workspace.tabs = vec![TabInfo {
            id: 1,
            layout: PaneLayout::Split {
                direction: SplitDirection::Vertical,
                first: Box::new(PaneLayout::Leaf { terminal_id: 1 }),
                second: Box::new(PaneLayout::Leaf { terminal_id: 2 }),
            },
        }];
    }
    let other = &mut snapshot.workspaces[1];
    other.custom_name = None;
    other.name = "logging".into();
    other.directory = "/opt/logging".into();
    other.terminals[0].directory = "/opt/logging".into();
    other.terminals[0].agents = vec![agent("", AgentStatus::Idle, false)];
    state
}

#[test]
fn agent_card_directories_belong_to_their_own_panes_not_the_workspace_selection() {
    for split in [false, true] {
        let mut rt = make_runtime(location_state(split), (1100.0, 900.0));
        for (workspace_id, terminal_id, automatic_name, directory) in [
            (1, 1, "frontend", "~/code/frontend"),
            (1, 2, "backend", "~/code/backend"),
            (2, 3, "logging", "/opt/logging"),
            (1, 1, "frontend", "~/code/frontend"),
        ] {
            rt.update(|s, _| {
                let mut next = s.snapshot.clone().unwrap();
                next.active_workspace = workspace_id;
                next.active_terminal = Some(terminal_id);
                next.terminal.as_mut().unwrap().id = terminal_id;
                next.workspaces[workspace_id as usize - 1].name = automatic_name.into();
                next.workspaces[workspace_id as usize - 1].directory = directory.into();
                s.apply(next);
            });
            rt.frame(0.0);
            assert_location(&rt, "agent-1", "frontend · Terminal 1", "~/code/frontend");
            assert_location(
                &rt,
                "agent-1:worker",
                "frontend · Terminal 1",
                "~/code/frontend",
            );
            assert_location(&rt, "agent-2", "backend · Terminal 2", "~/code/backend");
            assert_location(&rt, "agent-3", "logging · Terminal 3", "/opt/logging");
        }
    }
}

#[test]
fn only_the_agent_whose_pane_changes_directory_updates_its_location() {
    let mut rt = make_runtime(location_state(true), (1100.0, 900.0));
    rt.update(|s, _| {
        let mut next = s.snapshot.clone().unwrap();
        // An unfocused pane can change cwd; its initial_cwd stays /tmp.
        next.workspaces[0].terminals[1].directory = "~/code/backend/tests".into();
        s.apply(next);
    });
    rt.frame(0.0);
    assert_location(&rt, "agent-2", "tests · Terminal 2", "~/code/backend/tests");
    assert_location(&rt, "agent-1", "frontend · Terminal 1", "~/code/frontend");
    assert_location(&rt, "agent-3", "logging · Terminal 3", "/opt/logging");
}

#[test]
fn custom_workspace_names_are_context_not_a_replacement_for_each_panes_directory() {
    let mut state = location_state(false);
    let workspace = &mut state.snapshot.as_mut().unwrap().workspaces[0];
    workspace.custom_name = Some("Daily".into());
    workspace.name = "Daily".into();
    let rt = make_runtime(state, (1100.0, 900.0));
    assert_location(
        &rt,
        "agent-1",
        "Daily · frontend · Terminal 1",
        "~/code/frontend",
    );
    assert_location(
        &rt,
        "agent-2",
        "Daily · backend · Terminal 2",
        "~/code/backend",
    );
}

#[test]
fn agent_directory_labels_handle_home_root_unicode_and_formatting_controls() {
    for (path, expected) in [
        ("~", "~"),
        ("/", "/"),
        ("~/code/my project/日本語/", "日本語"),
        ("~/code/backend\u{202e}\u{200b}", "backend"),
    ] {
        let mut state = location_state(false);
        state.snapshot.as_mut().unwrap().workspaces[0].terminals[1].directory = path.into();
        let rt = make_runtime(state, (1100.0, 900.0));
        let safe_path = path.replace(['\u{202e}', '\u{200b}'], "");
        assert_location(
            &rt,
            "agent-2",
            &format!("{expected} · Terminal 2"),
            &safe_path,
        );
    }
}
