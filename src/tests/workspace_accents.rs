use super::*;
use griz_protocol::WorkspaceAccent;

#[test]
fn swatches_change_the_clicked_workspace_without_selecting_it_or_typing_into_the_shell() {
    for accent in WorkspaceAccent::ALL {
        let (state, outgoing) = mock();
        let mut rt = make_runtime(state, (1000.0, 680.0));
        rt.frame(0.0);
        commands(&outgoing);
        workspace_menu(&mut rt, 2);
        rt.key(rune::Key::Text("not shell input".into()).into());
        rt.paste("not shell paste");
        rt.frame(0.0);
        assert!(commands(&outgoing).is_empty());
        let name = rune::tok::ACCENT_NAMES[workspace_ui::accent_index(accent)];
        click(&mut rt, &format!("workspace-accent-{name}"));
        let sent = commands(&outgoing);
        assert_eq!(sent.len(), 1);
        assert!(matches!(
            sent[0].1,
            Command::SetWorkspaceAccent { workspace_id: 2, accent: chosen } if chosen == accent
        ));
        assert_eq!(rt.state.snapshot.as_ref().unwrap().active_workspace, 1);
        assert_eq!(rt.state.active_terminal(), Some(1));
        assert!(rt.state.dialog.is_none());
        assert!(rt.find("workspace-context-menu").is_none());
        let mut acknowledged = snapshot();
        acknowledged.ack = sent[0].0;
        acknowledged.workspaces[1].accent = accent;
        rt.update(|s, _| s.apply(acknowledged));
        rt.frame(0.0);
        assert_eq!(
            rt.state.snapshot.as_ref().unwrap().workspaces[1].accent,
            accent
        );
        assert_eq!(rt.state.workspace().unwrap().accent, WorkspaceAccent::Green);
    }
}

#[test]
fn workspace_accent_swatches_support_keyboard_selection() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1000.0, 680.0));
    rt.frame(0.0);
    commands(&outgoing);
    workspace_menu(&mut rt, 1);
    // Rune radio groups select on arrow navigation, starting from the checked green swatch.
    rt.key(rune::Key::Right.into());
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 1);
    assert!(matches!(
        sent[0].1,
        Command::SetWorkspaceAccent {
            workspace_id: 1,
            accent: WorkspaceAccent::Graphite
        }
    ));
    assert!(rt.state.dialog.is_none());
}

#[test]
fn workspace_accent_colors_the_selected_tab_sidebar_and_active_pane_in_both_themes() {
    let mut rt = runtime((1100.0, 720.0));
    for theme in [settings::Theme::Dark, settings::Theme::Midnight] {
        for accent in WorkspaceAccent::ALL {
            let mut updated = snapshot();
            updated.workspaces[0].accent = accent;
            rt.update(|s, _| {
                s.settings.theme = theme;
                s.apply(updated);
            });
            for _ in 0..60 {
                rt.frame(1.0 / 60.0);
            }
            let color = workspace_ui::accent_color(accent);
            let elements = rt.elements();
            let element = |key: &str| {
                elements
                    .iter()
                    .find(|e| e.key.as_deref() == Some(key))
                    .unwrap()
            };
            assert_eq!(element("workspace-1").fill, color.alpha(0.12));
            assert_eq!(element("tab-container-1").fill, color.alpha(0.15));
            assert_eq!(element("tab-container-2").fill, rune::Color::TRANSPARENT);
            assert_eq!(element("terminal-header-symbol").text_color, Some(color));
            assert_eq!(
                element("terminal").fill,
                terminal_view::background_for(theme)
            );
            let border = color.alpha(0.55).premul(1.0);
            assert!(
                rt.instances.iter().any(|instance| {
                    instance.params[0] > 0.0
                        && instance
                            .border
                            .into_iter()
                            .zip(border)
                            .all(|(actual, expected)| (actual - expected).abs() < 0.00001)
                }),
                "active pane border should use {accent:?}"
            );
        }
    }
}

#[test]
fn workspace_labels_keep_their_accent_when_unselected_without_a_background_highlight() {
    let mut rt = runtime((1100.0, 720.0));
    for theme in [settings::Theme::Dark, settings::Theme::Midnight] {
        for accent in WorkspaceAccent::ALL {
            for active_workspace in [1, 2] {
                let mut updated = snapshot();
                updated.workspaces[0].accent = WorkspaceAccent::Pink;
                updated.workspaces[1].accent = accent;
                updated.active_workspace = active_workspace;
                updated.active_terminal = Some(if active_workspace == 1 { 1 } else { 3 });
                updated.terminal.as_mut().unwrap().id = updated.active_terminal.unwrap();
                let workspaces = updated.workspaces.clone();
                rt.update(|s, _| {
                    s.settings.theme = theme;
                    s.apply(updated);
                });
                for _ in 0..60 {
                    rt.frame(1.0 / 60.0);
                }
                let elements = rt.elements();
                for workspace in workspaces {
                    let color = workspace_ui::accent_color(workspace.accent);
                    let label = elements
                        .iter()
                        .find(|e| e.text.as_deref() == Some(workspace.name.as_str()))
                        .unwrap();
                    assert_eq!(label.text_color, Some(color));
                    let key = format!("workspace-{}", workspace.id);
                    let row = elements
                        .iter()
                        .find(|e| e.key.as_deref() == Some(key.as_str()))
                        .unwrap();
                    assert_eq!(
                        row.fill,
                        if workspace.id == active_workspace {
                            color.alpha(0.12)
                        } else {
                            rune::Color::TRANSPARENT
                        }
                    );
                }
            }
        }
    }
}

#[test]
fn switching_tabs_keeps_the_accent_and_switching_workspaces_uses_their_own_color() {
    let mut rt = runtime((1100.0, 720.0));
    let mut updated = snapshot();
    updated.workspaces[0].accent = WorkspaceAccent::Pink;
    updated.workspaces[1].accent = WorkspaceAccent::Blue;
    updated.active_terminal = Some(2);
    updated.terminal.as_mut().unwrap().id = 2;
    rt.update(|s, _| s.apply(updated.clone()));
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
    let elements = rt.elements();
    let tab = elements
        .iter()
        .find(|e| e.key.as_deref() == Some("tab-container-2"))
        .unwrap();
    assert_eq!(
        tab.fill,
        workspace_ui::accent_color(WorkspaceAccent::Pink).alpha(0.15)
    );
    updated.active_workspace = 2;
    updated.active_terminal = Some(3);
    updated.terminal.as_mut().unwrap().id = 3;
    rt.update(|s, _| s.apply(updated));
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
    let elements = rt.elements();
    let symbol = elements
        .iter()
        .find(|e| e.key.as_deref() == Some("terminal-header-symbol"))
        .unwrap();
    assert_eq!(
        symbol.text_color,
        Some(workspace_ui::accent_color(WorkspaceAccent::Blue))
    );
}

#[test]
fn workspace_menu_fits_near_window_edges_at_large_font_sizes_and_dismisses_outside() {
    let (mut state, outgoing) = mock();
    state.settings.font_size = 28.0;
    let mut rt = make_runtime(state, (640.0, 420.0));
    rt.frame(0.0);
    commands(&outgoing);
    rt.update(|s, tx| s.open_dialog(Dialog::WorkspaceMenu(2, (638.0, 418.0)), tx));
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
    let ([x, y, w, h], _) = rt.presented("workspace-context-menu").unwrap();
    assert!(x >= 0.0 && x + w <= 640.0);
    assert!(y >= 0.0 && y + h <= 420.0);
    for name in rune::tok::ACCENT_NAMES {
        let ([sx, sy, sw, sh], _) = rt.presented(&format!("workspace-accent-{name}")).unwrap();
        assert!(sx >= x && sx + sw <= x + w);
        assert!(sy >= y && sy + sh <= y + h);
    }
    rt.pointer_down(2.0, 400.0);
    rt.pointer_up(2.0, 400.0);
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
    assert!(rt.state.dialog.is_none());
    assert!(commands(&outgoing).is_empty());
}

#[test]
fn a_removed_workspace_dismisses_its_accent_menu_without_sending_a_command() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1000.0, 680.0));
    rt.frame(0.0);
    commands(&outgoing);
    workspace_menu(&mut rt, 2);
    let mut updated = snapshot();
    updated.workspaces.pop();
    rt.update(|s, _| s.apply(updated));
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
    assert!(rt.state.dialog.is_none());
    assert!(rt.find("workspace-context-menu").is_none());
    assert!(commands(&outgoing).is_empty());
}
