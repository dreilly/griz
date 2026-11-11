use super::splits::{split_runtime, split_snapshot};
use super::*;
use griz_protocol::{PaneLayout, PanePlacement, SplitDirection};

fn settle(rt: &mut Runtime<State>) {
    for _ in 0..4 {
        rt.frame(0.0);
    }
}

fn drag(rt: &mut Runtime<State>, key: &str, dx: f32, dy: f32) {
    let (x, y) = rt.find(key).unwrap();
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_move(x + dx, y + dy);
    settle(rt);
    rt.pointer_up(x + dx, y + dy);
    settle(rt);
}

#[test]
fn dividers_resize_both_axes_and_reflow_each_shell_immediately() {
    for direction in [SplitDirection::Vertical, SplitDirection::Horizontal] {
        let (mut rt, outgoing) = split_runtime(direction);
        commands(&outgoing);
        let (before, _) = rt.presented("terminal-card-1").unwrap();
        let vertical = direction == SplitDirection::Vertical;
        drag(
            &mut rt,
            "pane-divider-root",
            if vertical { 70.0 } else { 0.0 },
            if vertical { 0.0 } else { 70.0 },
        );
        let (a, _) = rt.presented("terminal-card-1").unwrap();
        let (b, _) = rt.presented("terminal-card-2").unwrap();
        let axis = if vertical { 2 } else { 3 };
        assert!(
            (a[axis] - before[axis] - 70.0).abs() < 1.0,
            "{direction:?}: {before:?} -> {a:?}"
        );
        assert!((b[axis] - before[axis] + 70.0).abs() < 1.0);
        let sent = commands(&outgoing);
        for id in [1, 2] {
            let [w, h] = size(&rt, &format!("terminal-viewport-{id}"));
            let grid = terminal_view::grid_for(w, h, &rt.state.settings);
            assert!(sent.iter().any(|(_, c)| matches!(c, Command::Resize { terminal_id, cols, rows } if *terminal_id == id && (*cols, *rows) == grid)));
        }
        assert!(
            !sent
                .iter()
                .any(|(_, c)| matches!(c, Command::Input { .. } | Command::SelectTerminal { .. }))
        );
        // Output snapshots and zoom must not discard the window's proportions.
        rt.update(|s, _| s.apply(split_snapshot(direction)));
        settle(&mut rt);
        click(&mut rt, "zoom-pane-1");
        click(&mut rt, "zoom-pane-1");
        assert!((size(&rt, "terminal-card-1")[axis - 2] - a[axis]).abs() < 1.0);
    }
}

#[test]
fn resize_clamps_before_panes_collapse_and_escape_restores_original_sizes() {
    let (mut rt, outgoing) = split_runtime(SplitDirection::Vertical);
    drag(&mut rt, "pane-divider-root", -10000.0, 0.0);
    assert!(size(&rt, "terminal-card-1")[0] >= 307.0);
    assert!(size(&rt, "terminal-card-2")[0] >= 307.0);
    let ([gx, _, gw, _], _) = rt.presented("move-pane-1").unwrap();
    let ([cx, _, _, _], _) = rt.presented("split-vertical-1").unwrap();
    assert!(
        gx + gw <= cx,
        "the centered grip must not cover the pane controls"
    );
    let before = size(&rt, "terminal-card-1");
    commands(&outgoing);
    let (x, y) = rt.find("pane-divider-root").unwrap();
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_move(x + 80.0, y);
    settle(&mut rt);
    assert!(size(&rt, "terminal-card-1")[0] > before[0] + 70.0);
    rt.key(rune::Key::Text("must not type".into()).into());
    rt.key(rune::Key::Escape.into());
    settle(&mut rt);
    rt.pointer_up(x + 80.0, y);
    settle(&mut rt);
    assert!((size(&rt, "terminal-card-1")[0] - before[0]).abs() < 1.0);
    assert!(
        !commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::Input { .. }))
    );
}

#[test]
fn proportions_survive_window_resize_and_tab_switch_and_double_click_resets() {
    let (mut rt, _) = split_runtime(SplitDirection::Vertical);
    drag(&mut rt, "pane-divider-root", 50.0, 0.0);
    let original = size(&rt, "terminal-card-1")[0] / size(&rt, "terminal-card-2")[0];
    rt.set_viewport((1500.0, 900.0), 2.0);
    settle(&mut rt);
    let enlarged = size(&rt, "terminal-card-1")[0] / size(&rt, "terminal-card-2")[0];
    assert!((original - enlarged).abs() < 0.01);
    let mut other = split_snapshot(SplitDirection::Vertical);
    other.active_workspace = 2;
    other.active_terminal = Some(3);
    other.terminal.as_mut().unwrap().id = 3;
    other.terminals = vec![other.terminal.clone().unwrap()];
    rt.update(|s, _| s.apply(other));
    settle(&mut rt);
    rt.update(|s, _| s.apply(split_snapshot(SplitDirection::Vertical)));
    settle(&mut rt);
    assert!(
        (original - size(&rt, "terminal-card-1")[0] / size(&rt, "terminal-card-2")[0]).abs() < 0.01
    );
    let (x, y) = rt.find("pane-divider-root").unwrap();
    for _ in 0..2 {
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
    }
    settle(&mut rt);
    assert!((size(&rt, "terminal-card-1")[0] - size(&rt, "terminal-card-2")[0]).abs() < 1.0);
}

#[test]
fn each_grip_is_centered_and_drop_preview_matches_swap_or_edge_command() {
    for (placement, fx, fy) in [
        (PanePlacement::Swap, 0.5, 0.5),
        (PanePlacement::Left, 0.05, 0.5),
        (PanePlacement::Right, 0.95, 0.5),
        (PanePlacement::Top, 0.5, 0.05),
        (PanePlacement::Bottom, 0.5, 0.95),
    ] {
        let (mut rt, outgoing) = split_runtime(SplitDirection::Vertical);
        for id in [1, 2] {
            let ([hx, _, hw, _], _) = rt.presented(&format!("terminal-header-{id}")).unwrap();
            let ([gx, _, gw, _], _) = rt.presented(&format!("move-pane-{id}")).unwrap();
            assert!((gx + gw / 2.0 - hx - hw / 2.0).abs() < 1.0);
        }
        commands(&outgoing);
        // Drag an inactive pane without a separate selection/ack round trip.
        let (x, y) = rt.find("move-pane-2").unwrap();
        let ([tx, ty, tw, th], _) = rt.presented("terminal-card-1").unwrap();
        let end = (tx + tw * fx, ty + th * fy);
        rt.pointer_down(x, y);
        rt.frame(0.0);
        rt.pointer_move(end.0, end.1);
        settle(&mut rt);
        assert!(
            rt.find("pane-drop-preview-1").is_some(),
            "missing {placement:?} preview"
        );
        assert!(commands(&outgoing).is_empty(), "don't move until release");
        rt.pointer_up(end.0, end.1);
        settle(&mut rt);
        let sent = commands(&outgoing);
        assert_eq!(sent.len(), 1);
        assert!(
            matches!(sent[0].1, Command::MovePane { terminal_id: 2, target_terminal_id: 1, placement: p } if p == placement)
        );
        assert!(rt.state.pending.is_some());
        assert!(rt.find("pane-drop-preview-1").is_none());
        rt.key(rune::Key::Text("wait for ack".into()).into());
        rt.frame(0.0);
        assert!(commands(&outgoing).is_empty());
    }
}

#[test]
fn cancelled_outside_and_self_drops_do_not_change_layout_or_type() {
    for destination in ["escape", "outside", "self", "layout-change"] {
        let (mut rt, outgoing) = split_runtime(SplitDirection::Vertical);
        commands(&outgoing);
        let (x, y) = rt.find("move-pane-1").unwrap();
        let (tx, ty) = rt
            .find(if destination == "self" {
                "terminal-viewport-1"
            } else {
                "terminal-viewport-2"
            })
            .unwrap();
        let end = if destination == "outside" {
            (-50.0, -50.0)
        } else {
            (tx, ty)
        };
        rt.pointer_down(x, y);
        rt.frame(0.0);
        rt.pointer_move(end.0, end.1);
        settle(&mut rt);
        if destination == "escape" {
            rt.key(rune::Key::Text("blocked".into()).into());
            rt.key(rune::Key::Escape.into());
        } else if destination == "layout-change" {
            rt.update(|s, _| s.apply(split_snapshot(SplitDirection::Horizontal)));
        }
        settle(&mut rt);
        rt.pointer_up(end.0, end.1);
        settle(&mut rt);
        assert!(
            !commands(&outgoing)
                .iter()
                .any(|(_, c)| matches!(c, Command::MovePane { .. } | Command::Input { .. })),
            "{destination}"
        );
        assert!(rt.find("pane-drop-preview-2").is_none());
    }
}

#[test]
fn moving_a_nested_pane_keeps_unchanged_panes_and_dividers_as_targets() {
    let (mut state, outgoing) = mock();
    let mut view = split_snapshot(SplitDirection::Vertical);
    if let PaneLayout::Split { first, .. } = &mut view.workspaces[0].tabs[0].layout {
        **first = PaneLayout::Split {
            direction: SplitDirection::Horizontal,
            first: Box::new(PaneLayout::Leaf { terminal_id: 1 }),
            second: Box::new(PaneLayout::Leaf { terminal_id: 4 }),
        };
    }
    let mut frame = view.terminals[0].clone();
    frame.id = 4;
    view.terminals.push(frame);
    state.apply(view.clone());
    let mut rt = make_runtime(state, (1400.0, 900.0));
    settle(&mut rt);
    commands(&outgoing);
    let (x, y) = rt.find("move-pane-1").unwrap();
    let (tx, ty) = rt.find("terminal-viewport-4").unwrap();
    drag(&mut rt, "move-pane-1", tx - x, ty - y);
    let sent = commands(&outgoing);
    let serial = sent
        .iter()
        .find(|(_, c)| matches!(c, Command::MovePane { .. }))
        .unwrap()
        .0;
    if let PaneLayout::Split { first, .. } = &mut view.workspaces[0].tabs[0].layout {
        **first = PaneLayout::Split {
            direction: SplitDirection::Horizontal,
            first: Box::new(PaneLayout::Leaf { terminal_id: 4 }),
            second: Box::new(PaneLayout::Leaf { terminal_id: 1 }),
        };
    }
    view.ack = serial;
    rt.update(|s, _| s.apply(view));
    settle(&mut rt);
    commands(&outgoing);
    let (x, y) = rt.find("move-pane-1").unwrap();
    let (tx, ty) = rt.find("terminal-viewport-2").unwrap();
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_move(tx, ty);
    settle(&mut rt);
    assert!(rt.find("pane-drop-preview-2").is_some());
    rt.key(rune::Key::Escape.into());
    rt.pointer_up(tx, ty);
    settle(&mut rt);
    let before = size(&rt, "terminal-card-2");
    drag(&mut rt, "pane-divider-root", 40.0, 0.0);
    assert!((size(&rt, "terminal-card-2")[0] - before[0] + 40.0).abs() < 1.0);
}

#[test]
fn nested_dividers_resize_only_their_branch_and_fresh_layouts_start_equal() {
    let (mut state, outgoing) = mock();
    let mut view = split_snapshot(SplitDirection::Vertical);
    if let PaneLayout::Split { first, .. } = &mut view.workspaces[0].tabs[0].layout {
        **first = PaneLayout::Split {
            direction: SplitDirection::Horizontal,
            first: Box::new(PaneLayout::Leaf { terminal_id: 1 }),
            second: Box::new(PaneLayout::Leaf { terminal_id: 4 }),
        };
    }
    let mut frame = view.terminals[0].clone();
    frame.id = 4;
    view.terminals.push(frame);
    state.apply(view);
    let mut rt = make_runtime(state, (1400.0, 900.0));
    settle(&mut rt);
    commands(&outgoing);
    let right = size(&rt, "terminal-card-2");
    let first = size(&rt, "terminal-card-1");
    drag(&mut rt, "pane-divider-root-first", 0.0, 60.0);
    assert!((size(&rt, "terminal-card-1")[1] - first[1] - 60.0).abs() < 1.0);
    assert_eq!(size(&rt, "terminal-card-2"), right);
    drag(&mut rt, "pane-divider-root", 70.0, 0.0);
    rt.update(|s, _| s.apply(split_snapshot(SplitDirection::Vertical)));
    settle(&mut rt);
    assert!((size(&rt, "terminal-card-1")[0] - size(&rt, "terminal-card-2")[0]).abs() < 1.0);
}
