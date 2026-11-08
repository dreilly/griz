#[path = "tests/font_and_close.rs"]
mod font_and_close;
#[path = "tests/settings.rs"]
mod settings_tests;

use super::*;
use griz_protocol::{Request, TerminalFrame, TerminalInfo, WorkspaceInfo};
use rune::{Env, Runtime};
use std::sync::mpsc::Receiver;

fn snapshot() -> ViewState {
    let screen = griz_vt::Terminal::new(80, 24).unwrap().screen().unwrap();
    let info = |id| TerminalInfo {
        id,
        label: format!("Terminal {id}"),
        initial_cwd: "/tmp".into(),
        shell_pid: id as u32 + 100,
        exited: None,
    };
    ViewState {
        core_pid: 42,
        default_cwd: "/tmp".into(),
        ack: 0,
        workspaces: vec![
            WorkspaceInfo {
                id: 1,
                name: "Daily".into(),
                terminals: vec![info(1), info(2)],
            },
            WorkspaceInfo {
                id: 2,
                name: "Other".into(),
                terminals: vec![info(3)],
            },
        ],
        active_workspace: 1,
        active_terminal: Some(1),
        terminal: Some(TerminalFrame {
            id: 1,
            shell_pid: 101,
            screen,
            exited: None,
        }),
    }
}

fn mock() -> (State, Receiver<Request>) {
    let (client, outgoing) = client::Client::for_test();
    let mut state = State {
        client: Some(client),
        ..State::default()
    };
    state.apply(snapshot());
    (state, outgoing)
}

fn runtime(size: (f32, f32)) -> Runtime<State> {
    let (state, _) = mock();
    make_runtime(state, size)
}

fn make_runtime(state: State, size: (f32, f32)) -> Runtime<State> {
    let mut rt = Runtime::new(state, view, size, Env::new(true, 2.0));
    rt.on_key = Some(on_key);
    rt.frame(0.0);
    rt
}

fn size(rt: &Runtime<State>, key: &str) -> [f32; 2] {
    let ([_, _, width, height], _) = rt.presented(key).unwrap();
    [width, height]
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
}

fn commands(outgoing: &Receiver<Request>) -> Vec<(u64, Command)> {
    outgoing
        .try_iter()
        .filter_map(|r| match r {
            Request::Command { serial, command } => Some((serial, command)),
            _ => None,
        })
        .collect()
}

#[test]
fn first_frame_has_workspaces_tabs_and_a_terminal_viewport() {
    let rt = runtime((1100.0, 720.0));
    for key in [
        "sidebar",
        "workspace-1",
        "workspace-2",
        "tab-1",
        "tab-2",
        "terminal",
        "terminal-status",
    ] {
        assert!(rt.find(key).is_some(), "missing {key}");
    }
    assert!(rt.find("workspace-header").is_none());
    assert_eq!(size(&rt, "root"), [1100.0, 720.0]);
    let [width, height] = size(&rt, "terminal");
    assert!(width > 800.0 && height > 500.0);
}

#[test]
fn terminal_card_is_inset_with_a_header_and_shared_charcoal_surface() {
    let mut rt = runtime((1100.0, 720.0));
    // Layout callbacks are delivered after the first frame.
    rt.frame(0.0);
    let elements = rt.elements();
    let element = |key: &str| {
        elements
            .iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("missing {key}"))
    };
    let stage = element("terminal-stage");
    let card = element("terminal-card");
    let content = element("terminal-content");
    let header = element("terminal-header");
    let viewport = element("terminal");
    assert_eq!(card.radius, 11.0);
    assert_eq!(content.radius, 10.0);
    // The rounded content is inset by the frame width on every side.
    for (actual, expected) in content.rect.into_iter().zip([
        card.rect[0] + 1.0,
        card.rect[1] + 1.0,
        card.rect[2] - 2.0,
        card.rect[3] - 2.0,
    ]) {
        assert!((actual - expected).abs() < 0.1);
    }
    assert_eq!(
        card.fill,
        terminal_view::background_for(rt.state.settings.theme)
    );
    assert_eq!(viewport.fill, card.fill);
    assert_ne!(element("root").fill, card.fill);
    for (actual, expected) in card.rect.into_iter().zip([
        stage.rect[0] + 10.0,
        stage.rect[1] + 10.0,
        stage.rect[2] - 20.0,
        stage.rect[3] - 20.0,
    ]) {
        assert!((actual - expected).abs() < 0.1);
    }
    assert_eq!(header.rect[3], 34.0);
    assert!((viewport.rect[1] - header.rect[1] - header.rect[3]).abs() < 0.1);
    assert!(element("terminal-status").rect[1] > card.rect[1] + card.rect[3]);
    assert_eq!(
        rt.state.grid,
        terminal_view::grid_for(viewport.rect[2], viewport.rect[3], &rt.state.settings)
    );
}

#[test]
fn single_terminal_tab_stays_content_sized() {
    let (mut state, _) = mock();
    state.snapshot.as_mut().unwrap().workspaces[0]
        .terminals
        .truncate(1);
    let mut rt = make_runtime(state, (1100.0, 720.0));
    let [tab_width, _] = size(&rt, "tab-container-1");
    let [strip_width, _] = size(&rt, "terminal-tabs");
    assert!(tab_width < 150.0, "single tab stretched to {tab_width}");
    assert!(tab_width < strip_width / 2.0);
    // Adding another tab shouldn't change the first tab's natural width.
    rt.update(|s, _| s.apply(snapshot()));
    rt.frame(0.0);
    assert!((size(&rt, "tab-container-1")[0] - tab_width).abs() < 0.1);
}

#[test]
fn numbered_tabs_have_independent_close_buttons() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    rt.frame(0.0);
    let elements = rt.elements();
    for id in [1, 2] {
        let tab = elements
            .iter()
            .position(|e| e.key.as_deref() == Some(&format!("tab-{id}")))
            .unwrap();
        assert!(
            elements
                .iter()
                .any(|e| { e.parent == Some(tab) && e.text.as_deref() == Some(&id.to_string()) })
        );
        assert!(rt.find(&format!("close-terminal-{id}")).is_some());
    }
    commands(&outgoing);
    // Closing an inactive tab must not select it or affect the active shell.
    click(&mut rt, "close-terminal-2");
    assert_eq!(rt.state.active_terminal(), Some(1));
    assert!(commands(&outgoing).is_empty());
    assert!(matches!(rt.state.dialog, Some(Dialog::CloseTerminal(2, _))));
    click(&mut rt, "dialog-cancel");
    assert!(commands(&outgoing).is_empty());
    click(&mut rt, "close-terminal-2");
    click(&mut rt, "dialog-submit");
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 1);
    assert!(matches!(
        sent[0].1,
        Command::CloseTerminal { terminal_id: 2 }
    ));
}

#[test]
fn tab_numbers_are_positions_in_the_current_workspace_not_stored_labels_or_ids() {
    let mut rt = runtime((1100.0, 720.0));
    let assert_text = |rt: &Runtime<State>, key: &str, expected: &str| {
        let elements = rt.elements();
        let parent = elements
            .iter()
            .position(|e| e.key.as_deref() == Some(key))
            .unwrap();
        assert!(
            elements
                .iter()
                .any(|e| { e.parent == Some(parent) && e.text.as_deref() == Some(expected) }),
            "{key} should show {expected}"
        );
    };
    let mut updated = snapshot();
    updated.workspaces[0].terminals[0].label = "Terminal 47".into();
    updated.workspaces[0].terminals[1].label = "Terminal 103".into();
    rt.update(|s, _| s.apply(updated.clone()));
    rt.frame(0.0);
    assert_text(&rt, "tab-1", "1");
    assert_text(&rt, "tab-2", "2");
    // Another workspace starts at 1 regardless of global terminal IDs.
    updated.active_workspace = 2;
    updated.active_terminal = Some(3);
    updated.terminal.as_mut().unwrap().id = 3;
    rt.update(|s, _| s.apply(updated.clone()));
    rt.frame(0.0);
    assert_text(&rt, "tab-3", "1");
    assert_text(&rt, "terminal-header", "1");
    // Removing a tab leaves the remaining positions consecutive.
    updated.active_workspace = 1;
    updated.workspaces[0].terminals.remove(0);
    updated.active_terminal = Some(2);
    updated.terminal.as_mut().unwrap().id = 2;
    rt.update(|s, _| s.apply(updated));
    rt.frame(0.0);
    assert_text(&rt, "tab-2", "1");
    assert_text(&rt, "terminal-header", "1");
}

#[test]
fn tabs_are_wider_and_close_glyph_is_muted_without_hover_background() {
    let mut rt = runtime((1100.0, 720.0));
    assert!(size(&rt, "tab-container-1")[0] >= 80.0);
    let close_look = |rt: &Runtime<State>| {
        let elements = rt.elements();
        let close = elements
            .iter()
            .position(|e| e.key.as_deref() == Some("close-terminal-1"))
            .unwrap();
        let glyph = elements.iter().find(|e| e.parent == Some(close)).unwrap();
        (elements[close].fill, glyph.text_color)
    };
    let before = close_look(&rt);
    assert_eq!(before.0, rune::Color::TRANSPARENT);
    assert_eq!(before.1, Some(rune::oklch(0.64, 0.01, 260.0)));
    let (x, y) = rt.find("close-terminal-1").unwrap();
    rt.pointer_move(x, y);
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
    assert_eq!(close_look(&rt), before);
}

#[test]
fn terminal_header_tracks_the_selected_tab() {
    let mut rt = runtime((1100.0, 720.0));
    let mut updated = snapshot();
    updated.active_terminal = Some(2);
    updated.terminal.as_mut().unwrap().id = 2;
    rt.update(|s, _| s.apply(updated));
    rt.frame(0.0);
    let elements = rt.elements();
    let header = elements
        .iter()
        .position(|e| e.key.as_deref() == Some("terminal-header"))
        .unwrap();
    assert!(
        elements
            .iter()
            .any(|e| { e.parent == Some(header) && e.text.as_deref() == Some("2") })
    );
}

#[test]
fn clicking_sidebar_control_hides_and_restores_it() {
    let mut rt = runtime((1000.0, 680.0));
    let expanded_width = size(&rt, "terminal")[0];
    for expected in [false, true] {
        click(&mut rt, "toggle-sidebar");
        assert_eq!(rt.find("sidebar").is_some(), expected);
        if !expected {
            assert!(size(&rt, "terminal")[0] > expanded_width);
        }
    }
}

#[test]
fn rune_renderer_places_styled_and_wide_cells_on_the_grid() {
    let mut terminal = griz_vt::Terminal::new(80, 24).unwrap();
    terminal.write("A界B\r\n\x1b[38;2;1;2;3mC".as_bytes());
    let (mut state, _) = mock();
    state
        .snapshot
        .as_mut()
        .unwrap()
        .terminal
        .as_mut()
        .unwrap()
        .screen = terminal.screen().unwrap();
    let rt = make_runtime(state, (1000.0, 680.0));
    let elements = rt.elements();
    let glyph = |text: &str| {
        elements
            .iter()
            .find(|e| e.text.as_deref() == Some(text))
            .unwrap()
    };
    let a = glyph("A");
    let wide = glyph("界");
    let b = glyph("B");
    let c = glyph("C");
    let (cell_width, cell_height) = terminal_view::cell_size(&rt.state.settings);
    assert!((wide.rect[0] - a.rect[0] - cell_width).abs() < 0.1);
    assert!((b.rect[0] - a.rect[0] - 3.0 * cell_width).abs() < 0.1);
    assert!((c.rect[1] - a.rect[1] - cell_height).abs() < 0.1);
    assert_eq!(c.text_color, Some(terminal_view::color([1, 2, 3])));
    assert!(rt.find("terminal-cursor").is_some());
}

#[test]
fn typing_works_after_terminal_or_non_selection_toolbar_clicks() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1000.0, 680.0));
    for target in ["terminal", "reconnect", "toggle-sidebar"] {
        click(&mut rt, target);
        commands(&outgoing);
        rt.key(rune::Key::Text("x".into()).into());
        rt.frame(0.0);
        assert!(
            commands(&outgoing)
                .iter()
                .any(|(_, c)| matches!(c, Command::Input { terminal_id: 1, data } if data == b"x")),
            "typing after {target}"
        );
    }
}

#[test]
fn switching_tabs_waits_for_ack_and_routes_input_to_the_new_id() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1000.0, 680.0));
    commands(&outgoing);
    click(&mut rt, "tab-2");
    let sent = commands(&outgoing);
    let serial = sent
        .iter()
        .find_map(|(s, c)| matches!(c, Command::SelectTerminal { terminal_id: 2 }).then_some(*s))
        .unwrap();
    rt.key(rune::Key::Text("wrong".into()).into());
    rt.frame(0.0);
    assert!(
        commands(&outgoing).is_empty(),
        "never type into the old tab while switching"
    );
    let mut old = snapshot();
    old.ack = serial - 1;
    rt.state.apply(old);
    assert_eq!(rt.state.pending, Some(serial));
    rt.key(rune::Key::Text("still wrong".into()).into());
    rt.frame(0.0);
    assert!(commands(&outgoing).is_empty());
    let mut updated = snapshot();
    updated.ack = serial;
    updated.active_terminal = Some(2);
    updated.terminal.as_mut().unwrap().id = 2;
    rt.update(|s, _| s.apply(updated));
    rt.frame(0.0);
    assert!(rt.state.pending.is_none());
    commands(&outgoing);
    rt.key(rune::Key::Text("right".into()).into());
    rt.frame(0.0);
    let sent = commands(&outgoing);
    assert!(
        sent.iter()
            .any(|(_, c)| matches!(c, Command::Input { terminal_id: 2, data } if data == b"right")),
        "sent={sent:?}, online={}, pending={:?}, active={:?}",
        rt.state.online,
        rt.state.pending,
        rt.state.active_terminal()
    );
}

#[test]
fn workspace_click_sends_selection_not_shell_input() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1000.0, 680.0));
    commands(&outgoing);
    click(&mut rt, "workspace-2");
    assert!(
        commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::SelectWorkspace { workspace_id: 2 }))
    );
}

#[test]
fn workspace_dialog_keeps_typing_out_of_the_shell_and_creates_named_collection() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1000.0, 680.0));
    commands(&outgoing);
    click(&mut rt, "new-workspace");
    assert!(rt.find("dialog").is_some());
    click(&mut rt, "dialog-input");
    rt.key(rune::Key::Text("Unrelated things".into()).into());
    rt.frame(0.0);
    assert_eq!(rt.state.draft.text, "Unrelated things");
    assert!(
        !commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::Input { .. }))
    );
    click(&mut rt, "dialog-submit");
    assert!(commands(&outgoing).iter().any(
        |(_, c)| matches!(c, Command::CreateWorkspace { name } if name == "Unrelated things")
    ));
    assert!(rt.find("dialog").is_none());
}

#[test]
fn sidebar_rename_updates_the_selected_workspace() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1000.0, 680.0));
    let elements = rt.elements();
    let rename = elements
        .iter()
        .find(|e| e.key.as_deref() == Some("rename-workspace"))
        .unwrap();
    assert_eq!(
        elements[rename.parent.unwrap()].key.as_deref(),
        Some("sidebar")
    );
    commands(&outgoing);
    click(&mut rt, "rename-workspace");
    rt.update(|s, _| s.draft = TextEdit::new("Renamed"));
    rt.frame(0.0);
    click(&mut rt, "dialog-submit");
    assert!(commands(&outgoing).iter().any(|(_, c)| matches!(
        c,
        Command::RenameWorkspace { workspace_id: 1, name } if name == "Renamed"
    )));
}

#[test]
fn terminal_dialog_sends_its_own_directory_and_closing_requires_confirmation() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1000.0, 680.0));
    commands(&outgoing);
    click(&mut rt, "new-terminal");
    click(&mut rt, "dialog-input");
    rt.key(rune::Key::Text("/tmp/something-else".into()).into());
    rt.frame(0.0);
    click(&mut rt, "dialog-submit");
    assert!(commands(&outgoing).iter().any(|(_,c)| matches!(c, Command::CreateTerminal { workspace_id: 1, cwd: Some(cwd) } if cwd == "/tmp/something-else")));
    rt.state.pending = None;
    click(&mut rt, "close-terminal-1");
    assert!(
        !commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::CloseTerminal { .. }))
    );
    click(&mut rt, "dialog-cancel");
    assert!(
        !commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::CloseTerminal { .. }))
    );
    click(&mut rt, "close-terminal-1");
    click(&mut rt, "dialog-submit");
    assert!(
        commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::CloseTerminal { terminal_id: 1 }))
    );
}

#[test]
fn exited_shell_exposes_explicit_start_action_without_accepting_input() {
    let (mut state, outgoing) = mock();
    let mut ended = snapshot();
    ended.terminal.as_mut().unwrap().exited = Some("status 7".into());
    state.apply(ended);
    let mut rt = make_runtime(state, (1000.0, 680.0));
    assert!(rt.find("start-shell").is_some());
    commands(&outgoing);
    rt.key(rune::Key::Text("x".into()).into());
    rt.frame(0.0);
    assert!(
        !commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::Input { .. }))
    );
    click(&mut rt, "start-shell");
    assert!(
        commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::StartShell { terminal_id: 1 }))
    );
}

#[test]
fn empty_workspace_and_small_window_remain_usable() {
    let mut rt = runtime((640.0, 420.0));
    let [width, height] = size(&rt, "terminal");
    // The inset card and its header still leave room for a useful 40 × 9 grid.
    let (cols, rows) = terminal_view::grid_for(width, height, &rt.state.settings);
    assert!(cols >= 40 && rows >= 9, "grid is {cols} × {rows}");
    let mut empty = snapshot();
    empty.workspaces[0].terminals.clear();
    empty.active_terminal = None;
    empty.terminal = None;
    rt.update(|s, _| s.apply(empty));
    rt.frame(0.0);
    assert!(rt.find("new-terminal").is_some());
    assert!(rt.find("close-terminal-1").is_none());
    assert!(rt.state.status.starts_with("No terminals"));
}

#[test]
fn offline_capture_does_not_send_input_or_resize() {
    let (mut state, outgoing) = mock();
    state.offline = true;
    let mut rt = make_runtime(state, (1000.0, 680.0));
    rt.key(rune::Key::Text("x".into()).into());
    rt.frame(0.0);
    assert!(commands(&outgoing).is_empty());
}

#[test]
fn ordinary_pictures_do_not_reassert_dimensions_against_another_client() {
    let (mut state, outgoing) = mock();
    let mut other_size = snapshot();
    other_size.terminal.as_mut().unwrap().screen.cols = 100;
    other_size.terminal.as_mut().unwrap().screen.rows = 30;
    for _ in 0..5 {
        state.apply(other_size.clone());
    }
    assert!(
        commands(&outgoing).is_empty(),
        "last resize wins, no resize ping-pong"
    );
    state.change(Command::SelectTerminal { terminal_id: 2 });
    let serial = commands(&outgoing)[0].0;
    other_size.ack = serial;
    other_size.active_terminal = Some(2);
    other_size.terminal.as_mut().unwrap().id = 2;
    state.apply(other_size);
    assert!(commands(&outgoing).iter().any(|(_, c)| matches!(
        c,
        Command::Resize {
            terminal_id: 2,
            cols: 80,
            rows: 24
        }
    )));
}

#[test]
fn reconnect_restore_is_scoped_to_the_same_core_process() {
    let (mut state, outgoing) = mock();
    state.restore = Some((42, 2, Some(3)));
    state.online = false;
    state.apply(snapshot());
    let sent = commands(&outgoing);
    assert!(matches!(
        sent[0].1,
        Command::SelectWorkspace { workspace_id: 2 }
    ));
    assert!(matches!(
        sent[1].1,
        Command::SelectTerminal { terminal_id: 3 }
    ));
    assert_eq!(state.pending, Some(sent[1].0));
    state.pending = None;
    state.restore = Some((999, 2, Some(3)));
    state.apply(snapshot());
    assert!(
        commands(&outgoing).is_empty(),
        "IDs from a dead core must not be restored in a new core"
    );
}

#[test]
fn paste_targets_the_selected_terminal_but_dialog_paste_edits_the_field() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1000.0, 680.0));
    commands(&outgoing);
    rt.paste("echo hello");
    rt.frame(0.0);
    assert!(commands(&outgoing).iter().any(
        |(_, c)| matches!(c, Command::Input { terminal_id: 1, data } if data == b"echo hello")
    ));
    click(&mut rt, "new-workspace");
    click(&mut rt, "dialog-input");
    rt.paste("Pasted name");
    rt.frame(0.0);
    assert_eq!(rt.state.draft.text, "Pasted name");
    assert!(commands(&outgoing).is_empty());
}

#[test]
fn disconnected_client_stops_accepting_input_and_keeps_useful_error() {
    let (mut state, outgoing) = mock();
    let client = state.client.as_ref().unwrap();
    {
        let mut inbox = client.inbox.lock().unwrap();
        inbox.error = Some("Protocol version mismatch".into());
        inbox.disconnected = true;
    }
    let mut rt = make_runtime(State::default(), (1000.0, 680.0));
    rt.update(|_, tx| {
        assert!(poll(&mut state, 0.0, tx));
    });
    assert!(!state.online);
    assert_eq!(state.notice.as_deref(), Some("Protocol version mismatch"));
    state.input(b"x".to_vec());
    assert!(commands(&outgoing).is_empty());
}
