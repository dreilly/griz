use super::*;
use griz_protocol::{PaneLayout, SplitDirection};

fn split_snapshot(direction: SplitDirection) -> ViewState {
    let mut view = snapshot();
    view.workspaces[0].tabs = vec![TabInfo {
        id: 1,
        layout: PaneLayout::Split {
            direction,
            first: Box::new(PaneLayout::Leaf { terminal_id: 1 }),
            second: Box::new(PaneLayout::Leaf { terminal_id: 2 }),
        },
    }];
    let first = view.terminal.clone().unwrap();
    let mut second = first.clone();
    second.id = 2;
    second.shell_pid = 102;
    view.terminals = vec![first, second];
    view
}

fn split_runtime(direction: SplitDirection) -> (Runtime<State>, Receiver<Request>) {
    let (mut state, outgoing) = mock();
    state.apply(split_snapshot(direction));
    let mut rt = make_runtime(state, (1100.0, 720.0));
    rt.frame(0.0);
    rt.frame(0.0);
    (rt, outgoing)
}

#[test]
fn both_split_directions_have_equal_rounded_cards_with_a_ten_point_gap() {
    for direction in [SplitDirection::Horizontal, SplitDirection::Vertical] {
        let (rt, _) = split_runtime(direction);
        let (a, _) = rt.presented("terminal-card-1").unwrap();
        let (b, _) = rt.presented("terminal-card-2").unwrap();
        if direction == SplitDirection::Vertical {
            assert!((a[2] - b[2]).abs() < 0.1);
            assert!((a[3] - b[3]).abs() < 0.1);
            assert!((b[0] - a[0] - a[2] - 10.0).abs() < 0.1);
        } else {
            assert!((a[2] - b[2]).abs() < 0.1);
            assert!((a[3] - b[3]).abs() < 0.1);
            assert!((b[1] - a[1] - a[3] - 10.0).abs() < 0.1);
        }
        let elements = rt.elements();
        for id in [1, 2] {
            let card = elements
                .iter()
                .find(|e| e.key.as_deref() == Some(&format!("terminal-card-{id}")))
                .unwrap();
            assert_eq!(card.radius, 11.0);
            let content = elements
                .iter()
                .find(|e| e.key.as_deref() == Some(&format!("terminal-content-{id}")))
                .unwrap();
            assert_eq!(content.radius, 10.0);
        }
        assert!(rt.find("tab-1").is_some());
        assert!(rt.find("tab-2").is_none(), "splits stay in one tab");
    }
}

#[test]
fn every_visible_pane_gets_its_own_pty_dimensions_and_font_updates() {
    for direction in [SplitDirection::Horizontal, SplitDirection::Vertical] {
        let (mut rt, outgoing) = split_runtime(direction);
        let sent = commands(&outgoing);
        for id in [1, 2] {
            let [w, h] = size(&rt, &format!("terminal-viewport-{id}"));
            let grid = terminal_view::grid_for(w, h, &rt.state.settings);
            assert!(sent.iter().any(|(_, c)| matches!(c, Command::Resize { terminal_id, cols, rows } if *terminal_id == id && (*cols, *rows) == grid)));
        }
        rt.update(|s, _| s.change_font_size(3.0));
        rt.frame(0.0);
        let sent = commands(&outgoing);
        for id in [1, 2] {
            assert!(sent.iter().any(
                |(_, c)| matches!(c, Command::Resize { terminal_id, .. } if *terminal_id == id)
            ));
        }
    }
}

#[test]
fn clicking_a_pane_gates_typing_until_ack_then_routes_only_to_that_shell() {
    let (mut rt, outgoing) = split_runtime(SplitDirection::Vertical);
    commands(&outgoing);
    click(&mut rt, "terminal-viewport-2");
    let sent = commands(&outgoing);
    assert!(matches!(
        sent[0].1,
        Command::SelectTerminal { terminal_id: 2 }
    ));
    rt.key(rune::Key::Text("blocked".into()).into());
    rt.frame(0.0);
    assert!(commands(&outgoing).is_empty());
    let mut acknowledged = split_snapshot(SplitDirection::Vertical);
    acknowledged.ack = sent[0].0;
    acknowledged.active_terminal = Some(2);
    acknowledged.terminal = Some(acknowledged.terminals[1].clone());
    rt.update(|s, _| s.apply(acknowledged));
    rt.frame(0.0);
    commands(&outgoing);
    rt.key(rune::Key::Text("hello".into()).into());
    rt.frame(0.0);
    let sent = commands(&outgoing);
    assert!(
        sent.iter()
            .any(|(_, c)| matches!(c, Command::Input { terminal_id: 2, data } if data == b"hello"))
    );
    assert!(
        !sent
            .iter()
            .any(|(_, c)| matches!(c, Command::Input { terminal_id: 1, .. }))
    );
    click(&mut rt, "close-pane-1");
    assert!(rt.state.dialog.is_none());
    assert!(
        commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::CloseTerminal { terminal_id: 1 }))
    );
}

#[test]
fn pane_split_controls_and_shortcuts_are_addressed_and_do_not_type_into_shell() {
    for (key, direction) in [
        ("split-horizontal", SplitDirection::Horizontal),
        ("split-vertical", SplitDirection::Vertical),
    ] {
        let (state, outgoing) = mock();
        let mut rt = make_runtime(state, (1100.0, 720.0));
        commands(&outgoing);
        click(&mut rt, key);
        assert!(commands(&outgoing).iter().any(|(_, c)| matches!(c, Command::SplitTerminal { terminal_id: 1, direction: d } if *d == direction)));
    }
    let (mut rt, outgoing) = split_runtime(SplitDirection::Vertical);
    commands(&outgoing);
    rt.key(KeyEvent {
        key: rune::Key::Command('d'),
        mods: rune::Mods {
            cmd: true,
            ..rune::Mods::default()
        },
    });
    rt.frame(0.0);
    let sent = commands(&outgoing);
    assert!(sent.iter().any(|(_, c)| matches!(
        c,
        Command::SplitTerminal {
            terminal_id: 1,
            direction: SplitDirection::Vertical
        }
    )));
    assert!(!sent.iter().any(|(_, c)| matches!(c, Command::Input { .. })));
}

#[test]
fn pane_controls_are_always_visible_inside_each_header() {
    let single = runtime((1100.0, 720.0));
    let (split, _) = split_runtime(SplitDirection::Vertical);
    for (rt, suffixes) in [(&single, vec![""]), (&split, vec!["-1", "-2"])] {
        for suffix in suffixes {
            let ([hx, hy, hw, hh], _) = rt.presented(&format!("terminal-header{suffix}")).unwrap();
            for control in ["split-vertical", "split-horizontal", "close-pane"] {
                let key = format!("{control}{suffix}");
                let ([x, y, w, h], _) = rt.presented(&key).unwrap();
                assert!(
                    x >= hx && x + w <= hx + hw,
                    "{key} outside header horizontally"
                );
                assert!(
                    y >= hy && y + h <= hy + hh,
                    "{key} outside header vertically"
                );
                let elements = rt.elements();
                let element = elements
                    .iter()
                    .find(|e| e.key.as_deref() == Some(&key))
                    .unwrap();
                assert_eq!(element.opacity, 1.0, "{key} should not require hover");
                assert!(element.visible[2] > 0.0 && element.visible[3] > 0.0);
            }
        }
    }
    assert!(split.find("split-vertical").is_none());
    assert!(split.find("split-horizontal").is_none());
}

#[test]
fn splitting_an_inactive_pane_queues_selection_and_waits_for_the_split_ack() {
    for (key, direction) in [
        ("split-horizontal-2", SplitDirection::Horizontal),
        ("split-vertical-2", SplitDirection::Vertical),
    ] {
        let (mut rt, outgoing) = split_runtime(SplitDirection::Vertical);
        commands(&outgoing);
        click(&mut rt, key);
        let sent = commands(&outgoing);
        assert_eq!(sent.len(), 2);
        assert!(matches!(
            sent[0].1,
            Command::SelectTerminal { terminal_id: 2 }
        ));
        assert!(
            matches!(sent[1].1, Command::SplitTerminal { terminal_id: 2, direction: d } if d == direction)
        );
        assert_eq!(rt.state.pending, Some(sent[1].0));
        let mut intermediate = split_snapshot(SplitDirection::Vertical);
        intermediate.ack = sent[0].0;
        intermediate.active_terminal = Some(2);
        intermediate.terminal = Some(intermediate.terminals[1].clone());
        rt.update(|s, _| s.apply(intermediate));
        rt.frame(0.0);
        assert_eq!(rt.state.active_terminal(), Some(1));
        assert_eq!(rt.state.pending, Some(sent[1].0));
        rt.key(rune::Key::Text("blocked".into()).into());
        rt.frame(0.0);
        assert!(commands(&outgoing).is_empty());
    }
}

#[test]
fn original_pane_can_be_split_repeatedly_after_each_new_pane_takes_focus() {
    fn add_split(node: &mut PaneLayout, new_id: TerminalId, direction: SplitDirection) {
        match node {
            PaneLayout::Leaf { terminal_id: 1 } => {
                *node = PaneLayout::Split {
                    direction,
                    first: Box::new(PaneLayout::Leaf { terminal_id: 1 }),
                    second: Box::new(PaneLayout::Leaf {
                        terminal_id: new_id,
                    }),
                };
            }
            PaneLayout::Split { first, second, .. } => {
                add_split(first, new_id, direction);
                add_split(second, new_id, direction);
            }
            _ => {}
        }
    }
    for (control, direction) in [
        ("split-horizontal", SplitDirection::Horizontal),
        ("split-vertical", SplitDirection::Vertical),
    ] {
        let (mut state, outgoing) = mock();
        let mut view = snapshot();
        view.workspaces[0].terminals.truncate(1);
        view.workspaces[0].tabs.truncate(1);
        view.terminals = vec![view.terminal.clone().unwrap()];
        state.apply(view.clone());
        let mut rt = make_runtime(state, (1400.0, 900.0));
        rt.frame(0.0);
        for new_id in 2..=4 {
            commands(&outgoing);
            let key = if new_id == 2 {
                control.to_owned()
            } else {
                format!("{control}-1")
            };
            click(&mut rt, &key);
            let sent = commands(&outgoing);
            let (serial, split) = sent
                .last()
                .unwrap_or_else(|| panic!("no command for {key}, new pane {new_id}"));
            assert!(
                matches!(split, Command::SplitTerminal { terminal_id: 1, direction: d } if *d == direction)
            );
            if new_id > 2 {
                assert!(matches!(
                    sent[0].1,
                    Command::SelectTerminal { terminal_id: 1 }
                ));
            }
            add_split(&mut view.workspaces[0].tabs[0].layout, new_id, direction);
            let mut info = view.workspaces[0].terminals[0].clone();
            info.id = new_id;
            view.workspaces[0].terminals.push(info);
            let mut frame = view.terminals[0].clone();
            frame.id = new_id;
            view.terminals.push(frame.clone());
            view.active_terminal = Some(new_id);
            view.terminal = Some(frame);
            view.ack = *serial;
            rt.update(|s, _| s.apply(view.clone()));
            for _ in 0..60 {
                rt.frame(1.0 / 60.0);
            }
            assert!(rt.state.ready());
            assert_eq!(rt.state.active_terminal(), Some(new_id));
            assert_eq!(
                rt.state.active_tab().unwrap().layout.terminal_ids().len(),
                new_id as usize
            );
        }
    }
}

#[test]
fn single_pane_header_close_preserves_last_terminal_confirmation() {
    for last_terminal in [false, true] {
        let (mut state, outgoing) = mock();
        if last_terminal {
            let workspace = &mut state.snapshot.as_mut().unwrap().workspaces[0];
            workspace.terminals.truncate(1);
            workspace.tabs.truncate(1);
        }
        let mut rt = make_runtime(state, (1100.0, 720.0));
        rt.frame(0.0);
        commands(&outgoing);
        click(&mut rt, "close-pane");
        let sent = commands(&outgoing);
        if last_terminal {
            assert!(matches!(rt.state.dialog, Some(Dialog::CloseTerminal(1, _))));
            assert!(sent.is_empty());
        } else {
            assert!(rt.state.dialog.is_none());
            assert_eq!(sent.len(), 1);
            assert!(matches!(
                sent[0].1,
                Command::CloseTerminal { terminal_id: 1 }
            ));
        }
    }
}

#[test]
fn nested_first_child_splits_have_unique_keys_and_collapse_back_to_a_full_card() {
    let (mut state, outgoing) = mock();
    let mut view = split_snapshot(SplitDirection::Vertical);
    let first = &mut view.workspaces[0].tabs[0].layout;
    if let PaneLayout::Split { first, .. } = first {
        **first = PaneLayout::Split {
            direction: SplitDirection::Horizontal,
            first: Box::new(PaneLayout::Leaf { terminal_id: 1 }),
            second: Box::new(PaneLayout::Leaf { terminal_id: 4 }),
        };
    }
    let mut frame = view.terminals[0].clone();
    frame.id = 4;
    view.terminals.push(frame);
    let mut info = view.workspaces[0].terminals[0].clone();
    info.id = 4;
    view.workspaces[0].terminals.push(info);
    state.apply(view);
    let mut rt = make_runtime(state, (1100.0, 720.0));
    rt.frame(0.0);
    rt.frame(0.0);
    let mut keys = std::collections::HashSet::new();
    for element in rt.elements() {
        if let Some(key) = element.key {
            assert!(keys.insert(key.clone()), "duplicate key: {key}");
        }
    }
    let (a, _) = rt.presented("terminal-card-1").unwrap();
    let (b, _) = rt.presented("terminal-card-4").unwrap();
    let (c, _) = rt.presented("terminal-card-2").unwrap();
    assert!((a[3] - b[3]).abs() < 0.1);
    assert!((c[3] - a[3] - b[3] - 10.0).abs() < 0.1);
    commands(&outgoing);
    let mut collapsed = snapshot();
    collapsed.workspaces[0].terminals.truncate(1);
    collapsed.workspaces[0].tabs.truncate(1);
    collapsed.terminals = vec![collapsed.terminal.clone().unwrap()];
    rt.update(|s, _| s.apply(collapsed));
    rt.frame(0.0);
    rt.frame(0.0);
    assert!(rt.find("terminal-card").is_some());
    assert!(rt.find("terminal-card-4").is_none());
    assert_eq!(rt.state.pane_sizes.len(), 1);
    assert!(size(&rt, "terminal")[0] > a[2] * 1.8);
    assert!(
        commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::Resize { terminal_id: 1, .. }))
    );
}

#[test]
fn tab_navigation_skips_sibling_panes_and_pane_navigation_stays_in_tab() {
    let (mut rt, outgoing) = split_runtime(SplitDirection::Vertical);
    rt.update(|s, _| {
        let w = &mut s.snapshot.as_mut().unwrap().workspaces[0];
        let mut info = w.terminals[0].clone();
        info.id = 4;
        w.terminals.push(info);
        w.tabs.push(tab(4));
    });
    rt.frame(0.0);
    commands(&outgoing);
    rt.update(|s, tx| s.act(Action::NextPane, tx));
    let sent = commands(&outgoing);
    assert!(matches!(
        sent[0].1,
        Command::SelectTerminal { terminal_id: 2 }
    ));
    rt.state.pending = None;
    rt.update(|s, tx| s.act(Action::NextTab, tx));
    let sent = commands(&outgoing);
    assert!(matches!(
        sent[0].1,
        Command::SelectTerminal { terminal_id: 4 }
    ));
}
