use super::splits::{split_runtime, split_snapshot};
use super::*;
use griz_protocol::SplitDirection;

fn settle(rt: &mut Runtime<State>) {
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
}

#[test]
fn zoom_fills_the_content_area_and_icon_restores_both_split_directions() {
    for direction in [SplitDirection::Horizontal, SplitDirection::Vertical] {
        let (mut rt, outgoing) = split_runtime(direction);
        let (a, _) = rt.presented("terminal-card-1").unwrap();
        let (b, _) = rt.presented("terminal-card-2").unwrap();
        let original_layout = rt.state.active_tab().unwrap().layout.clone();
        commands(&outgoing);
        click(&mut rt, "zoom-pane-1");
        assert!(rt.state.pane_zoomed());
        assert!(rt.find("terminal-card-2").is_none());
        let (zoom, _) = rt.presented("terminal-card-1").unwrap();
        let width = if direction == SplitDirection::Vertical {
            a[2] + b[2] + 10.0
        } else {
            a[2]
        };
        let height = if direction == SplitDirection::Horizontal {
            a[3] + b[3] + 10.0
        } else {
            a[3]
        };
        assert!((zoom[2] - width).abs() < 0.1);
        assert!((zoom[3] - height).abs() < 0.1);
        assert!(rt.find("sidebar").is_some());
        assert!(rt.find("tab-1").is_some());
        assert!(rt.find("pane-zoom-indicator-1").is_none());
        assert!(rt.find("zoom-pane-1").is_some());
        assert_eq!(rt.state.active_tab().unwrap().layout, original_layout);
        assert_eq!(rt.state.pane_sizes.len(), 1);
        let sent = commands(&outgoing);
        assert!(
            sent.iter()
                .any(|(_, c)| matches!(c, Command::Resize { terminal_id: 1, .. }))
        );
        assert!(
            !sent
                .iter()
                .any(|(_, c)| matches!(c, Command::Resize { terminal_id: 2, .. }))
        );
        click(&mut rt, "zoom-pane-1");
        assert!(!rt.state.pane_zoomed());
        assert!(rt.find("terminal-card-2").is_some());
        assert!(rt.find("pane-zoom-indicator-1").is_none());
        let (restored, _) = rt.presented("terminal-card-1").unwrap();
        assert!((restored[2] - a[2]).abs() < 0.1);
        assert!((restored[3] - a[3]).abs() < 0.1);
        assert_eq!(rt.state.pane_sizes.len(), 2);
    }
}

#[test]
fn zoom_shortcut_is_rebindable_and_never_reaches_the_shell() {
    let (mut rt, outgoing) = split_runtime(SplitDirection::Vertical);
    commands(&outgoing);
    rt.key(KeyEvent {
        key: rune::Key::Enter,
        mods: rune::Mods {
            cmd: true,
            ..Default::default()
        },
    });
    settle(&mut rt);
    assert!(rt.state.pane_zoomed());
    assert!(
        !commands(&outgoing)
            .iter()
            .any(|(_, command)| terminal_input(command))
    );
    rt.update(|s, _| {
        s.bind_shortcut(
            Action::TogglePaneZoom,
            Some(rune::Chord::parse("mod+shift+z").unwrap()),
        )
    });
    // Reserved shortcuts are still rejected for this action.
    assert!(rt.state.settings_error.is_some());
    rt.update(|s, _| {
        s.bind_shortcut(
            Action::TogglePaneZoom,
            Some(rune::Chord::parse("mod+j").unwrap()),
        )
    });
    assert!(rt.state.settings_error.is_none());
    let (loaded, error) = Settings::from_store(&rt.state.prefs);
    assert!(error.is_none());
    assert_eq!(
        loaded.chord(Action::TogglePaneZoom),
        rt.state.settings.chord(Action::TogglePaneZoom)
    );
    rt.key(KeyEvent {
        key: rune::Key::Command('j'),
        mods: rune::Mods {
            cmd: true,
            ..Default::default()
        },
    });
    settle(&mut rt);
    assert!(!rt.state.pane_zoomed());
    assert!(
        !commands(&outgoing)
            .iter()
            .any(|(_, command)| terminal_input(command))
    );
}

#[test]
fn zoom_follows_acknowledged_pane_focus_and_ignores_hidden_layout_callbacks() {
    let (mut rt, outgoing) = split_runtime(SplitDirection::Vertical);
    click(&mut rt, "zoom-pane-1");
    commands(&outgoing);
    rt.update(|s, _| {
        s.layout_pane(2, 20.0, 20.0);
        s.change_font_size(1.0);
    });
    assert!(!rt.state.pane_sizes.contains_key(&2));
    assert!(
        !commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::Resize { terminal_id: 2, .. }))
    );
    rt.update(|s, tx| s.act(Action::NextPane, tx));
    let sent = commands(&outgoing);
    let serial = sent
        .iter()
        .find(|(_, c)| matches!(c, Command::SelectTerminal { terminal_id: 2 }))
        .unwrap()
        .0;
    assert_eq!(rt.state.active_terminal(), Some(1));
    let mut view = split_snapshot(SplitDirection::Vertical);
    view.ack = serial;
    view.active_terminal = Some(2);
    view.terminal = Some(view.terminals[1].clone());
    rt.update(|s, _| s.apply(view));
    settle(&mut rt);
    assert!(rt.state.pane_zoomed());
    assert!(rt.find("terminal-card-1").is_none());
    assert!(rt.find("terminal-card-2").is_some());
    assert!(rt.find("pane-zoom-indicator-2").is_none());
    assert!(rt.find("zoom-pane-2").is_some());
    assert_eq!(rt.state.pane_sizes.len(), 1);
    assert!(rt.state.pane_sizes.contains_key(&2));
}

#[test]
fn clicking_zoom_on_an_inactive_pane_selects_it_before_input_can_resume() {
    let (mut rt, outgoing) = split_runtime(SplitDirection::Vertical);
    commands(&outgoing);
    click(&mut rt, "zoom-pane-2");
    let sent = commands(&outgoing);
    assert!(
        sent.iter()
            .any(|(_, c)| matches!(c, Command::SelectTerminal { terminal_id: 2 }))
    );
    assert!(rt.state.pending.is_some());
    assert!(rt.state.pane_zoomed());
    rt.key(rune::Key::Text("blocked".into()).into());
    settle(&mut rt);
    assert!(
        !commands(&outgoing)
            .iter()
            .any(|(_, command)| terminal_input(command))
    );
}

#[test]
fn zoom_is_per_tab_and_cleared_when_a_tab_collapses_or_the_service_restarts() {
    let (mut rt, _) = split_runtime(SplitDirection::Vertical);
    click(&mut rt, "zoom-pane-1");
    let mut other = split_snapshot(SplitDirection::Vertical);
    other.active_workspace = 2;
    other.active_terminal = Some(3);
    other.terminal.as_mut().unwrap().id = 3;
    other.terminals = vec![other.terminal.clone().unwrap()];
    rt.update(|s, _| s.apply(other));
    settle(&mut rt);
    assert!(!rt.state.pane_zoomed());
    rt.update(|s, _| s.apply(split_snapshot(SplitDirection::Vertical)));
    settle(&mut rt);
    assert!(rt.state.pane_zoomed());
    let mut single = snapshot();
    single.terminals = vec![single.terminal.clone().unwrap()];
    rt.update(|s, _| s.apply(single));
    settle(&mut rt);
    assert!(rt.state.zoomed_tabs.is_empty());
    assert!(rt.find("terminal-card").is_some());
    rt.update(|s, _| s.apply(split_snapshot(SplitDirection::Vertical)));
    settle(&mut rt);
    click(&mut rt, "zoom-pane-1");
    let mut restarted = split_snapshot(SplitDirection::Vertical);
    restarted.core_pid += 1;
    rt.update(|s, _| s.apply(restarted));
    settle(&mut rt);
    assert!(rt.state.zoomed_tabs.is_empty());
    assert!(rt.find("terminal-card-2").is_some());
}

#[test]
fn nested_zoom_hides_other_panes_and_restores_the_exact_tree() {
    let (mut state, _) = mock();
    let mut view = split_snapshot(SplitDirection::Vertical);
    if let griz_protocol::PaneLayout::Split { first, .. } = &mut view.workspaces[0].tabs[0].layout {
        **first = griz_protocol::PaneLayout::Split {
            direction: SplitDirection::Horizontal,
            first: Box::new(griz_protocol::PaneLayout::Leaf { terminal_id: 1 }),
            second: Box::new(griz_protocol::PaneLayout::Leaf { terminal_id: 4 }),
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
    settle(&mut rt);
    let original_layout = rt.state.active_tab().unwrap().layout.clone();
    click(&mut rt, "zoom-pane-1");
    assert!(rt.find("pane-zoom-indicator-1").is_none());
    assert!(rt.find("terminal-card-4").is_none());
    assert!(rt.find("terminal-card-2").is_none());
    click(&mut rt, "zoom-pane-1");
    assert!(rt.find("terminal-card-4").is_some());
    assert!(rt.find("terminal-card-2").is_some());
    assert_eq!(rt.state.active_tab().unwrap().layout, original_layout);
}

#[test]
fn zoom_icon_scales_and_uses_the_pane_accent_border_only_while_zoomed() {
    for font_size in [10.0, 14.0, 28.0] {
        for accent in griz_protocol::WorkspaceAccent::ALL {
            let (mut rt, _) = split_runtime(SplitDirection::Vertical);
            rt.update(|s, _| {
                s.settings.font_size = font_size;
                s.snapshot.as_mut().unwrap().workspaces[0].accent = accent;
            });
            settle(&mut rt);
            let icon_size = 18.0 * font_size / 14.0;
            let [width, height] = size(&rt, "zoom-pane-icon-1");
            assert!((width - icon_size).abs() < 0.1);
            assert!((height - icon_size).abs() < 0.1);
            for zoomed in [false, true, false] {
                if rt.state.pane_zoomed() != zoomed {
                    click(&mut rt, "zoom-pane-1");
                }
                let (rect, _) = rt.presented("zoom-pane-1").unwrap();
                let border = workspace_ui::accent_color(accent).premul(1.0);
                let has_accent_border = rt.instances.iter().any(|instance| {
                    instance.params[0] > 0.0
                        && instance
                            .rect
                            .into_iter()
                            .zip(rect)
                            .all(|(actual, expected)| (actual - expected * 2.0).abs() < 0.1)
                        && instance
                            .border
                            .into_iter()
                            .zip(border)
                            .all(|(actual, expected)| (actual - expected).abs() < 0.00001)
                });
                assert_eq!(has_accent_border, zoomed, "{accent:?}, font {font_size}");
                assert!(rt.find("pane-zoom-indicator-1").is_none());
                assert!(
                    !rt.elements()
                        .iter()
                        .any(|e| e.text.as_deref().is_some_and(|t| t.starts_with("Zoomed ·")))
                );
            }
        }
    }
}

#[test]
fn command_palette_can_toggle_pane_zoom() {
    let (mut rt, outgoing) = split_runtime(SplitDirection::Vertical);
    commands(&outgoing);
    command_palette_tests::open(&mut rt, true);
    command_palette_tests::press(&mut rt, rune::Key::Text("zoom".into()));
    settle(&mut rt);
    click(&mut rt, "palette-action-pane.toggle_zoom");
    assert!(!rt.state.palette.is_open());
    assert!(rt.state.pane_zoomed());
    assert!(rt.find("terminal-card-2").is_none());
    assert!(
        !commands(&outgoing)
            .iter()
            .any(|(_, command)| terminal_input(command))
    );
}

#[test]
fn splitting_reveals_the_layout_and_zoom_is_a_noop_for_a_single_pane() {
    let (mut rt, outgoing) = split_runtime(SplitDirection::Vertical);
    click(&mut rt, "zoom-pane-1");
    commands(&outgoing);
    click(&mut rt, "split-vertical-1");
    assert!(!rt.state.pane_zoomed());
    assert!(rt.find("terminal-card-2").is_some());
    assert!(
        commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::SplitTerminal { terminal_id: 1, .. }))
    );
    let (state, outgoing) = mock();
    let mut single = make_runtime(state, (1100.0, 720.0));
    commands(&outgoing);
    single.update(|s, tx| s.act(Action::TogglePaneZoom, tx));
    assert!(!single.state.pane_zoomed());
    assert!(single.find("zoom-pane").is_none());
    assert!(commands(&outgoing).is_empty());
}
