use super::agents::agent;
use super::*;
use crate::harness_icons::Harness;
use griz_protocol::{AgentStatus, PaneLayout};

fn assert_text(rt: &Runtime<State>, key: &str, expected: &str) {
    let elements = rt.elements();
    let parent = elements
        .iter()
        .position(|e| e.key.as_deref() == Some(key))
        .unwrap();
    assert!(
        elements
            .iter()
            .any(|e| e.parent == Some(parent) && e.text.as_deref() == Some(expected))
    );
}

#[test]
fn harness_logos_match_in_agent_cards_and_numbered_tabs() {
    for (app, harness) in [
        ("pi", Harness::Pi),
        ("Claude Code", Harness::Claude),
        ("codex", Harness::Codex),
        ("gemini-cli", Harness::Gemini),
        ("opencode", Harness::OpenCode),
        ("terraform", Harness::Other),
        ("", Harness::Other),
    ] {
        let (mut state, _) = mock();
        let mut report = agent("", AgentStatus::Working, false);
        report.record.app = app.into();
        state.snapshot.as_mut().unwrap().workspaces[0].terminals[1].agents = vec![report];
        let rt = make_runtime(state, (1100.0, 720.0));
        for key in [
            "agent-2-harness-icon".to_owned(),
            format!("tab-2-harness-{}", harness.key()),
        ] {
            let elements = rt.elements();
            let mark = elements
                .iter()
                .find(|e| e.key.as_ref() == Some(&key))
                .unwrap();
            assert_eq!(mark.role, "icon");
            assert_eq!(mark.label.as_deref(), Some(harness.name()));
            assert!(mark.rect[2] >= 16.0 && mark.rect[3] >= 16.0);
        }
        assert_text(&rt, "tab-2", "2");
        assert!(rt.find("tab-1-harness-program").is_none());
        assert_eq!(rt.state.active_terminal(), Some(1));
    }
}

#[test]
fn split_tabs_include_background_panes_and_deduplicate_child_harnesses() {
    let (mut state, _) = mock();
    let workspace = &mut state.snapshot.as_mut().unwrap().workspaces[0];
    workspace.tabs = vec![TabInfo {
        id: 1,
        layout: PaneLayout::Split {
            direction: SplitDirection::Vertical,
            first: Box::new(PaneLayout::Leaf { terminal_id: 1 }),
            second: Box::new(PaneLayout::Leaf { terminal_id: 2 }),
        },
    }];
    let root = agent("", AgentStatus::Idle, false);
    let child = agent("worker", AgentStatus::Working, false);
    let mut other = agent("worker/other", AgentStatus::Blocked, false);
    other.record.app = "claude-code".into();
    workspace.terminals[0].agents = vec![root.clone(), child.clone()];
    workspace.terminals[1].agents = vec![root, child, other];
    let mut rt = make_runtime(state, (1100.0, 720.0));
    for _ in 0..2 {
        assert_eq!(
            crate::harness_icons::for_tab(
                rt.state.workspace().unwrap(),
                rt.state.active_tab().unwrap()
            ),
            vec![Harness::Pi, Harness::Claude]
        );
        assert!(rt.find("tab-1-harness-pi").is_some());
        assert!(rt.find("tab-1-harness-claude").is_some());
        assert_eq!(
            rt.elements()
                .iter()
                .filter(|e| e
                    .key
                    .as_deref()
                    .is_some_and(|k| k.starts_with("tab-1-harness-")))
                .count(),
            2
        );
        rt.update(|s, _| s.snapshot.as_mut().unwrap().active_terminal = Some(2));
        rt.frame(0.0);
    }
}

#[test]
fn harness_marks_update_and_clear_without_changing_selection_or_notifications() {
    let (mut state, _) = mock();
    state.snapshot.as_mut().unwrap().workspaces[0].terminals[1].agents =
        vec![agent("", AgentStatus::Done, true)];
    let mut rt = make_runtime(state, (1100.0, 720.0));
    assert!(rt.find("tab-2-harness-pi").is_some());
    rt.update(|s, _| {
        let mut next = s.snapshot.clone().unwrap();
        next.workspaces[0].terminals[1].agents[0].record.app = "codex".into();
        s.apply(next);
    });
    rt.frame(0.0);
    assert!(rt.find("tab-2-harness-pi").is_none());
    assert!(rt.find("tab-2-harness-codex").is_some());
    assert!(rt.find("agent-notifications").is_some());
    assert_eq!(rt.state.active_terminal(), Some(1));
    rt.update(|s, _| {
        let mut next = s.snapshot.clone().unwrap();
        next.workspaces[0].terminals[1].agents.clear();
        s.apply(next);
    });
    rt.frame(0.0);
    assert!(rt.find("tab-2-harness-codex").is_none());
    assert!(rt.find("agent-2-harness-icon").is_none());
    assert_text(&rt, "tab-2", "2");
    assert_eq!(rt.state.active_terminal(), Some(1));
}

#[test]
fn clicking_a_tab_logo_selects_its_terminal_without_closing_it() {
    let (mut state, outgoing) = mock();
    state.snapshot.as_mut().unwrap().workspaces[0].terminals[1].agents =
        vec![agent("", AgentStatus::Working, false)];
    let mut rt = make_runtime(state, (1100.0, 720.0));
    commands(&outgoing);
    click(&mut rt, "tab-2-harness-pi");
    let sent: Vec<_> = commands(&outgoing)
        .into_iter()
        .filter(|(_, c)| !matches!(c, Command::Resize { .. }))
        .collect();
    assert_eq!(sent.len(), 1);
    assert!(matches!(
        sent[0].1,
        Command::SelectTerminal { terminal_id: 2 }
    ));
    assert_eq!(rt.state.pending, Some(sent[0].0));
}

#[test]
fn logos_scale_with_ui_fonts_and_fit_their_controls() {
    for font_size in [13.0, 22.0] {
        let (mut state, _) = mock();
        state.settings.font_size = font_size;
        state.snapshot.as_mut().unwrap().workspaces[0].terminals[1].agents =
            vec![agent("", AgentStatus::Working, false)];
        let rt = make_runtime(state, (640.0, 480.0));
        for (icon, parent) in [
            ("tab-2-harness-pi", "tab-2"),
            ("agent-2-harness-icon", "agent-2"),
        ] {
            let (mark, _) = rt.presented(icon).unwrap();
            let (container, _) = rt.presented(parent).unwrap();
            assert!(mark[2] >= font_size);
            assert!(mark[0] >= container[0] && mark[1] >= container[1]);
            assert!(mark[0] + mark[2] <= container[0] + container[2] + 0.1);
            assert!(mark[1] + mark[3] <= container[1] + container[3] + 0.1);
        }
    }
}
