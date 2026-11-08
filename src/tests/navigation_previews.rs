use super::command_palette_tests::{open, press, settle};
use super::*;
use griz_protocol::{PaneLayout, PreviewFrame, PreviewTarget, SplitDirection};
use rune::Key;

fn subscriptions(outgoing: &Receiver<Request>) -> Vec<Option<PreviewTarget>> {
    commands(outgoing)
        .into_iter()
        .map(|(_, command)| match command {
            Command::SetPreview { target } => target,
            other => panic!("preview navigation sent a mutating command: {other:?}"),
        })
        .collect()
}

#[test]
fn highlight_subscribes_without_switching_and_escape_unsubscribes() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    rt.frame(0.0);
    commands(&outgoing);
    open(&mut rt, false);
    assert_eq!(
        subscriptions(&outgoing),
        vec![Some(PreviewTarget::Workspace { workspace_id: 1 })]
    );
    assert!(rt.find("palette-preview").is_some());
    press(&mut rt, Key::Down);
    assert_eq!(
        subscriptions(&outgoing),
        vec![Some(PreviewTarget::Terminal {
            workspace_id: 1,
            terminal_id: 1
        })]
    );
    assert_eq!(rt.state.active_terminal(), Some(1));
    assert_eq!(rt.state.snapshot.as_ref().unwrap().active_workspace, 1);
    assert!(rt.state.pending.is_none());
    press(&mut rt, Key::Escape);
    assert_eq!(subscriptions(&outgoing), vec![None]);
    assert!(rt.find("palette-preview").is_none());
}

#[test]
fn filtering_and_catalog_changes_keep_the_preview_on_the_resolved_result() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    rt.frame(0.0);
    commands(&outgoing);
    open(&mut rt, false);
    subscriptions(&outgoing);
    press(&mut rt, Key::Text("other terminal".into()));
    assert_eq!(
        subscriptions(&outgoing),
        vec![Some(PreviewTarget::Terminal {
            workspace_id: 2,
            terminal_id: 3,
        })]
    );
    let mut snapshot = rt.state.snapshot.clone().unwrap();
    snapshot.workspaces.retain(|w| w.id != 2);
    rt.update(|state, _| state.apply(snapshot));
    settle(&mut rt);
    assert_eq!(subscriptions(&outgoing), vec![None]);
    assert!(rt.find("palette-empty").is_some());
    assert_eq!(rt.state.active_terminal(), Some(1));
}

fn show_split_preview(rt: &mut Runtime<State>) {
    let mut snapshot = rt.state.snapshot.clone().unwrap();
    let mut first = snapshot.terminal.clone().unwrap();
    first.screen.cells[0][0].text = "X".into();
    let mut second = first.clone();
    second.id = 2;
    snapshot.preview = Some(Box::new(PreviewFrame {
        target: PreviewTarget::Workspace { workspace_id: 1 },
        selected: 2,
        layout: PaneLayout::Split {
            direction: SplitDirection::Vertical,
            first: Box::new(PaneLayout::Leaf { terminal_id: 1 }),
            second: Box::new(PaneLayout::Leaf { terminal_id: 2 }),
        },
        terminals: vec![first, second],
    }));
    rt.update(|state, _| state.apply(snapshot));
    settle(rt);
}

#[test]
fn preview_renders_fitted_split_screens_without_ime_focus_or_resize() {
    for (width, height, font_size) in [
        (1100.0, 720.0, 14.0),
        (640.0, 720.0, 14.0),
        (940.0, 520.0, 28.0),
    ] {
        let (mut state, outgoing) = mock();
        state.settings.font_size = font_size;
        let mut rt = make_runtime(state, (width, height));
        rt.frame(0.0);
        commands(&outgoing);
        open(&mut rt, false);
        subscriptions(&outgoing);
        show_split_preview(&mut rt);
        assert!(subscriptions(&outgoing).is_empty());
        let (bounds, _) = rt.presented("command-palette").unwrap();
        for key in [
            "palette-preview",
            "palette-preview-pane-1",
            "palette-preview-pane-2",
            "preview-screen-grid-1",
            "preview-screen-grid-2",
        ] {
            let ([x, y, w, h], _) = rt
                .presented(key)
                .unwrap_or_else(|| panic!("missing {key} at {width}x{height}"));
            assert!(w > 0.0 && h > 0.0, "empty {key}");
            assert!(
                x >= bounds[0]
                    && y >= bounds[1]
                    && x + w <= bounds[0] + bounds[2] + 0.1
                    && y + h <= bounds[1] + bounds[3] + 0.1,
                "{key} outside palette at {width}x{height}: {:?} vs {bounds:?}",
                [x, y, w, h]
            );
        }
        assert!(size(&rt, "palette-results")[1] > 30.0);
        // The actual terminal still owns the only IME cursor, and the search keeps focus.
        assert_eq!(
            rt.elements()
                .iter()
                .filter(|e| e.key.as_deref() == Some("terminal-cursor"))
                .count(),
            1
        );
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.key.as_deref() == Some("palette-input") && e.focused)
        );

        // A late frame for the previous highlight must never be shown as the new target.
        press(&mut rt, Key::Down);
        assert!(rt.find("palette-preview-pane-1").is_none());
        assert!(rt.find("preview-screen-grid-1").is_none());
    }
}

#[test]
fn outside_dismissal_and_switching_to_commands_stop_preview_subscription() {
    for outside in [false, true] {
        let (state, outgoing) = mock();
        let mut rt = make_runtime(state, (1100.0, 720.0));
        rt.frame(0.0);
        commands(&outgoing);
        open(&mut rt, false);
        subscriptions(&outgoing);
        if outside {
            let (x, y) = rt.find("new-workspace").unwrap();
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
        } else {
            open(&mut rt, true);
        }
        assert_eq!(subscriptions(&outgoing), vec![None]);
        assert!(rt.find("palette-preview").is_none());
    }
}
