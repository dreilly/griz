#[path = "tests/agents.rs"]
mod agents;
#[path = "tests/buttons.rs"]
mod buttons;
#[path = "tests/command_palette.rs"]
mod command_palette_tests;
#[path = "tests/font_and_close.rs"]
mod font_and_close;
#[path = "tests/input_latency.rs"]
mod input_latency;
#[path = "tests/navigation_previews.rs"]
mod navigation_previews;
#[path = "tests/settings.rs"]
mod settings_tests;
#[path = "tests/splits.rs"]
mod splits;
#[path = "tests/workspace_accents.rs"]
mod workspace_accents;
#[path = "tests/workspace_labels.rs"]
mod workspace_labels;

use super::*;
use griz_protocol::{Request, TerminalFrame, TerminalInfo, WorkspaceInfo};
use rune::{Env, Runtime};
use std::sync::mpsc::Receiver;

fn tab(id: TerminalId) -> TabInfo {
    TabInfo {
        id,
        layout: griz_protocol::PaneLayout::Leaf { terminal_id: id },
    }
}

fn snapshot() -> ViewState {
    let screen = griz_vt::Terminal::new(80, 24).unwrap().screen().unwrap();
    let info = |id| TerminalInfo {
        id,
        label: format!("Terminal {id}"),
        initial_cwd: "/tmp".into(),
        directory: "/tmp".into(),
        shell_pid: id as u32 + 100,
        exited: None,
        agents: vec![],
    };
    ViewState {
        core_pid: 42,
        default_cwd: "/tmp".into(),
        ack: 0,
        workspaces: vec![
            WorkspaceInfo {
                id: 1,
                name: "Daily".into(),
                custom_name: Some("Daily".into()),
                directory: "/tmp".into(),
                accent: griz_protocol::WorkspaceAccent::default(),
                terminals: vec![info(1), info(2)],
                tabs: vec![tab(1), tab(2)],
            },
            WorkspaceInfo {
                id: 2,
                name: "Other".into(),
                custom_name: Some("Other".into()),
                directory: "/tmp".into(),
                accent: griz_protocol::WorkspaceAccent::default(),
                terminals: vec![info(3)],
                tabs: vec![tab(3)],
            },
        ],
        active_workspace: 1,
        active_terminal: Some(1),
        preview: None,
        terminals: vec![],
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
    assert!(rt.state.dialog.is_none());
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
    updated.workspaces[0].tabs.remove(0);
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
fn workspace_name_can_be_left_blank_on_create_and_rename() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1000.0, 680.0));
    click(&mut rt, "new-workspace");
    assert!(rt.state.draft.text.is_empty());
    commands(&outgoing);
    click(&mut rt, "dialog-submit");
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 1);
    assert!(matches!(&sent[0].1, Command::CreateWorkspace { name } if name.is_empty()));
    // Acknowledge creation before opening a second dialog.
    let mut updated = snapshot();
    updated.ack = sent[0].0;
    rt.update(|s, _| s.apply(updated));
    rt.frame(0.0);
    workspace_menu(&mut rt, 2);
    pick_workspace_menu(&mut rt, "rename");
    rt.update(|s, _| s.draft = TextEdit::new("   "));
    rt.frame(0.0);
    commands(&outgoing);
    click(&mut rt, "dialog-submit");
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 1);
    assert!(
        matches!(&sent[0].1, Command::RenameWorkspace { workspace_id: 2, name } if name.is_empty())
    );
}

#[test]
fn workspace_context_menu_renames_the_clicked_workspace_without_selecting_it() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1000.0, 680.0));
    rt.frame(0.0); // Deliver the initial viewport resize before testing menu commands.
    assert!(rt.find("rename-workspace").is_none());
    assert!(
        !rt.elements()
            .iter()
            .any(|e| e.text.as_deref().is_some_and(|t| t.contains("tabs ·")))
    );
    commands(&outgoing);
    workspace_menu(&mut rt, 2);
    assert!(rt.find("workspace-context-menu").is_some());
    assert!(rt.find("workspace-menu-rename").is_some());
    assert!(rt.find("workspace-menu-close").is_some());
    for name in rune::tok::ACCENT_NAMES {
        assert!(rt.find(&format!("workspace-accent-{name}")).is_some());
    }
    assert!(commands(&outgoing).is_empty());
    assert_eq!(rt.state.snapshot.as_ref().unwrap().active_workspace, 1);
    pick_workspace_menu(&mut rt, "rename");
    assert_eq!(rt.state.draft.text, "Other");
    rt.update(|s, _| s.draft = TextEdit::new("Renamed"));
    rt.frame(0.0);
    click(&mut rt, "dialog-submit");
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 1);
    assert!(matches!(
        &sent[0].1,
        Command::RenameWorkspace { workspace_id: 2, name } if name == "Renamed"
    ));
}

fn workspace_menu(rt: &mut Runtime<State>, id: WorkspaceId) {
    let (x, y) = rt.find(&format!("workspace-{id}")).unwrap();
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
}

fn pick_workspace_menu(rt: &mut Runtime<State>, item: &str) {
    click(rt, &format!("workspace-menu-{item}"));
}

#[test]
fn workspace_close_confirms_the_clicked_id_and_supports_cancellation() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1000.0, 680.0));
    rt.frame(0.0);
    commands(&outgoing);
    workspace_menu(&mut rt, 2);
    rt.key(rune::Key::Escape.into());
    rt.frame(0.0);
    assert!(rt.state.dialog.is_none());
    workspace_menu(&mut rt, 2);
    pick_workspace_menu(&mut rt, "close");
    assert!(matches!(&rt.state.dialog, Some(Dialog::CloseWorkspace(2, name)) if name == "Other"));
    assert!(rt.find("dialog-input").is_none());
    assert!(!rt.state.draft.focused);
    assert!(commands(&outgoing).is_empty());
    click(&mut rt, "dialog-cancel");
    assert!(commands(&outgoing).is_empty());
    workspace_menu(&mut rt, 2);
    pick_workspace_menu(&mut rt, "close");
    // Catalog/selection updates must not retarget an open confirmation.
    rt.update(|s, _| s.apply(snapshot()));
    rt.frame(0.0);
    commands(&outgoing);
    click(&mut rt, "dialog-submit");
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 1);
    assert!(matches!(
        sent[0].1,
        Command::CloseWorkspace { workspace_id: 2 }
    ));
}

#[test]
fn workspace_menus_are_safe_when_disconnected_pending_or_target_removed() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1000.0, 680.0));
    rt.frame(0.0);
    commands(&outgoing);
    workspace_menu(&mut rt, 2);
    rt.update(|s, _| {
        let mut updated = s.snapshot.clone().unwrap();
        updated.workspaces.remove(1);
        s.apply(updated);
    });
    rt.frame(0.0);
    assert!(rt.find("workspace-context-menu").is_none());
    assert!(rt.state.dialog.is_none());
    rt.update(|s, _| s.pending = Some(999));
    rt.frame(0.0);
    workspace_menu(&mut rt, 1);
    assert!(rt.find("workspace-context-menu").is_none());
    rt.update(|s, _| {
        s.pending = None;
        s.online = false;
    });
    rt.frame(0.0);
    workspace_menu(&mut rt, 1);
    assert!(rt.find("workspace-context-menu").is_none());
    assert!(commands(&outgoing).is_empty());
}

#[test]
fn closing_the_last_workspace_leaves_creation_available() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1000.0, 680.0));
    let mut empty = snapshot();
    empty.workspaces.clear();
    empty.active_workspace = 0;
    empty.active_terminal = None;
    empty.terminal = None;
    rt.update(|s, _| s.apply(empty));
    rt.frame(0.0);
    assert!(rt.state.status.starts_with("No workspaces."));
    assert!(rt.find("workspace-1").is_none());
    assert!(rt.state.workspace().is_none());
    assert!(rt.find("empty-workspaces").is_some());
    for key in [
        "terminal",
        "terminal-pane",
        "terminal-card",
        "terminal-header",
        "terminal-tabs",
        "terminal-status",
        "new-terminal",
    ] {
        assert!(
            rt.find(key).is_none(),
            "unexpected {key} in the empty state"
        );
    }
    commands(&outgoing);
    click(&mut rt, "new-workspace");
    assert!(matches!(rt.state.dialog, Some(Dialog::NewWorkspace)));
    rt.update(|s, _| s.draft = TextEdit::new("Fresh"));
    rt.frame(0.0);
    click(&mut rt, "dialog-submit");
    assert!(
        commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::CreateWorkspace { name } if name == "Fresh"))
    );
}

#[test]
fn empty_workspace_button_is_centered_scales_and_disappears_when_a_workspace_opens() {
    for sidebar_open in [true, false] {
        let (mut state, outgoing) = mock();
        state.sidebar_open = sidebar_open;
        let mut empty = snapshot();
        empty.workspaces.clear();
        empty.active_workspace = 0;
        empty.active_terminal = None;
        empty.terminal = None;
        state.apply(empty);
        let mut rt = make_runtime(state, (640.0, 420.0));
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        let ([x, y, width, height], _) = rt.presented("empty-workspaces").unwrap();
        let ([mx, my, mw, mh], _) = rt.presented("empty-workspaces-message").unwrap();
        assert!((mx + mw / 2.0 - (x + width / 2.0)).abs() < 1.0);
        let ([bx, by, bw, bh], _) = rt.presented("empty-workspaces-create").unwrap();
        assert!((bx + bw / 2.0 - (x + width / 2.0)).abs() < 1.0);
        assert!((by + bh / 2.0 - (y + height / 2.0)).abs() < 1.0);
        assert!(my >= by && my + mh <= by + bh);
        for key in [
            "empty-workspaces-icon",
            "empty-workspaces-symbol",
            "empty-workspaces-cursor",
        ] {
            assert!(
                rt.find(key).is_none(),
                "unexpected {key} in the empty state"
            );
        }
        assert!(mx >= x && mx + mw <= x + width);
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.text.as_deref() == Some("Create a workspace to get started"))
        );
        commands(&outgoing);
        rt.key(rune::Key::Text("no shell input".into()).into());
        rt.frame(0.0);
        assert!(commands(&outgoing).is_empty());
        rt.update(|s, _| s.settings.font_size = 18.0);
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        assert!(size(&rt, "empty-workspaces-message")[1] > mh);
        rt.update(|s, _| s.apply(snapshot()));
        rt.frame(0.0);
        assert!(rt.find("empty-workspaces").is_none());
        for key in [
            "terminal",
            "terminal-card",
            "terminal-header",
            "terminal-status",
            "new-terminal",
        ] {
            assert!(
                rt.find(key).is_some(),
                "missing {key} after opening a workspace"
            );
        }
    }
}

#[test]
fn empty_workspace_button_stays_centered_and_fits_after_resizing() {
    let (mut state, _) = mock();
    let mut empty = snapshot();
    empty.workspaces.clear();
    empty.active_workspace = 0;
    empty.active_terminal = None;
    empty.terminal = None;
    state.apply(empty);
    let mut rt = make_runtime(state, (1100.0, 720.0));
    for (viewport, sidebar, font_size) in [
        ((640.0, 420.0), true, 14.0),
        ((640.0, 420.0), true, 28.0),
        ((640.0, 420.0), false, 28.0),
        ((1400.0, 900.0), false, 14.0),
    ] {
        rt.update(|s, _| {
            s.sidebar_open = sidebar;
            s.settings.font_size = font_size;
        });
        rt.set_viewport(viewport, 2.0);
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        let ([x, y, width, height], _) = rt.presented("empty-workspaces").unwrap();
        for key in ["empty-workspaces-message", "empty-workspaces-create"] {
            let ([cx, cy, cw, ch], _) = rt.presented(key).unwrap();
            assert!(
                cx >= x && cx + cw <= x + width,
                "{key} overflows horizontally"
            );
            assert!(
                cy >= y && cy + ch <= y + height,
                "{key} overflows vertically"
            );
            assert!((cx + cw / 2.0 - (x + width / 2.0)).abs() < 1.0);
            assert!((cy + ch / 2.0 - (y + height / 2.0)).abs() < 1.0);
        }
    }
}

#[test]
fn empty_workspace_button_label_scales_with_font_preferences() {
    let (mut state, _) = mock();
    state.sidebar_open = false;
    let mut empty = snapshot();
    empty.workspaces.clear();
    empty.active_workspace = 0;
    empty.active_terminal = None;
    empty.terminal = None;
    state.apply(empty);
    let mut rt = make_runtime(state, (1100.0, 720.0));
    let caption_height = size(&rt, "empty-workspaces-message")[1];
    for (font_size, font) in [
        (10.0, settings::Font::Menlo),
        (18.0, settings::Font::Monaco),
        (28.0, settings::Font::System),
    ] {
        rt.update(|s, _| {
            s.settings.font_size = font_size;
            s.settings.font = font;
        });
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        let height = size(&rt, "empty-workspaces-message")[1];
        if font_size < 14.0 {
            assert!(height < caption_height);
        } else {
            assert!(height > caption_height);
        }
    }
}

#[test]
fn empty_workspace_button_creates_a_default_named_workspace_without_a_dialog() {
    let (mut state, outgoing) = mock();
    state.sidebar_open = false;
    let mut empty = snapshot();
    empty.workspaces.clear();
    empty.active_workspace = 0;
    empty.active_terminal = None;
    empty.terminal = None;
    state.apply(empty);
    let mut rt = make_runtime(state, (1100.0, 720.0));
    commands(&outgoing);
    click(&mut rt, "empty-workspaces-create");
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 1);
    assert!(matches!(&sent[0].1, Command::CreateWorkspace { name } if name.is_empty()));
    assert!(rt.state.dialog.is_none());
    assert!(rt.state.pending.is_some());
    click(&mut rt, "empty-workspaces-create");
    assert!(
        commands(&outgoing).is_empty(),
        "pending creation must not be submitted twice"
    );
    let mut created = snapshot();
    created.ack = sent[0].0;
    rt.update(|s, _| s.apply(created));
    rt.frame(0.0);
    assert!(rt.find("empty-workspaces-create").is_none());
    assert!(rt.find("terminal").is_some());
}

#[test]
fn empty_workspace_button_does_not_create_when_offline_or_disconnected() {
    for offline in [true, false] {
        let (mut state, outgoing) = mock();
        let mut empty = snapshot();
        empty.workspaces.clear();
        empty.active_workspace = 0;
        empty.active_terminal = None;
        empty.terminal = None;
        state.apply(empty);
        state.offline = offline;
        state.online = false;
        let mut rt = make_runtime(state, (1100.0, 720.0));
        commands(&outgoing);
        click(&mut rt, "empty-workspaces-create");
        assert!(commands(&outgoing).is_empty());
        assert!(rt.state.dialog.is_none());
    }
}

#[test]
fn terminal_dialog_sends_its_own_directory_and_final_terminal_close_requires_confirmation() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1000.0, 680.0));
    commands(&outgoing);
    click(&mut rt, "new-terminal");
    click(&mut rt, "dialog-input");
    rt.key(rune::Key::Text("/tmp/something-else".into()).into());
    rt.frame(0.0);
    click(&mut rt, "dialog-submit");
    assert!(commands(&outgoing).iter().any(|(_,c)| matches!(c, Command::CreateTerminal { workspace_id: 1, cwd: Some(cwd) } if cwd == "/tmp/something-else")));
    let mut final_terminal = snapshot();
    final_terminal.ack = rt.state.pending.unwrap();
    final_terminal.workspaces[0].terminals.truncate(1);
    final_terminal.workspaces[0].tabs.truncate(1);
    rt.update(|s, _| s.apply(final_terminal));
    rt.frame(0.0);
    commands(&outgoing);
    click(&mut rt, "close-terminal-1");
    assert!(matches!(rt.state.dialog, Some(Dialog::CloseTerminal(1, _))));
    let elements = rt.elements();
    let dialog = elements
        .iter()
        .position(|e| e.key.as_deref() == Some("dialog"))
        .unwrap();
    let dialog_text: Vec<_> = elements
        .iter()
        .filter(|e| e.parent == Some(dialog))
        .filter_map(|e| e.text.as_deref())
        .collect();
    assert_eq!(dialog_text, vec!["Close workspace?", "Daily"]);
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
    empty.workspaces[0].tabs.clear();
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
        inbox.error = Some("Core is stopping".into());
        inbox.disconnected = true;
    }
    let mut rt = make_runtime(State::default(), (1000.0, 680.0));
    rt.update(|_, tx| {
        assert!(poll(&mut state, 0.0, tx));
    });
    assert!(!state.online);
    assert_eq!(state.notice.as_deref(), Some("Core is stopping"));
    state.input(b"x".to_vec());
    assert!(commands(&outgoing).is_empty());
}
