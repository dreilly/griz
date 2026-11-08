use super::*;
use rune::{Chord, Key, Mods};

// Layout can resize the mock terminal; palette assertions concern addressed actions.
fn commands(outgoing: &Receiver<Request>) -> Vec<(u64, Command)> {
    super::commands(outgoing)
        .into_iter()
        .filter(|(_, c)| !matches!(c, Command::Resize { .. }))
        .collect()
}

fn primary(letter: char, shift: bool) -> KeyEvent {
    KeyEvent {
        key: Key::Command(letter),
        mods: Mods {
            cmd: true,
            shift,
            ..Mods::default()
        },
    }
}

fn ctrl(letter: char) -> KeyEvent {
    KeyEvent {
        key: Key::Text(letter.to_string()),
        mods: Mods {
            ctrl: true,
            ..Mods::default()
        },
    }
}

fn press(rt: &mut Runtime<State>, key: impl Into<KeyEvent>) {
    rt.key(key.into());
    rt.frame(0.0);
}

fn settle(rt: &mut Runtime<State>) {
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
}

fn open(rt: &mut Runtime<State>, commands: bool) {
    press(rt, primary('p', commands));
    settle(rt);
    assert!(rt.find("command-palette").is_some());
    assert!(
        rt.elements()
            .iter()
            .any(|e| e.key.as_deref() == Some("palette-input") && e.focused)
    );
}

#[test]
fn automatic_workspace_names_do_not_replace_each_terminals_live_directory() {
    let (mut state, outgoing) = mock();
    let snapshot = state.snapshot.as_mut().unwrap();
    let workspace = &mut snapshot.workspaces[0];
    workspace.custom_name = None;
    workspace.name = "c_ok".into();
    workspace.directory = "~/code/c_ok".into();
    workspace.terminals[0].directory = "~/Documents".into();
    workspace.terminals[1].directory = "~/code/c_ok".into();
    snapshot.active_terminal = Some(2);
    let mut rt = make_runtime(state, (1100.0, 720.0));
    open(&mut rt, false);
    commands(&outgoing);
    let elements = rt.elements();
    assert!(
        elements
            .iter()
            .any(|e| e.text.as_deref() == Some("Documents › Terminal 1"))
    );
    assert!(
        elements
            .iter()
            .any(|e| e.text.as_deref() == Some("c_ok › Terminal 2"))
    );
    assert!(
        elements
            .iter()
            .any(|e| e.text.as_deref() == Some("Terminal · ~/Documents"))
    );
    press(&mut rt, Key::Text("documents".into()));
    assert!(rt.find("palette-terminal-1").is_some());
    assert!(rt.find("palette-terminal-2").is_none());
    assert!(rt.find("palette-workspace-1").is_none());
    press(&mut rt, Key::Enter);
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 1);
    assert!(matches!(
        sent[0].1,
        Command::SelectTerminal { terminal_id: 1 }
    ));
}

#[test]
fn live_directory_search_updates_for_inactive_tabs_without_selecting_them() {
    let (mut state, outgoing) = mock();
    let workspace = &mut state.snapshot.as_mut().unwrap().workspaces[0];
    workspace.custom_name = None;
    workspace.name = "active-project".into();
    workspace.directory = "~/active-project".into();
    workspace.terminals[0].directory = "~/active-project".into();
    workspace.terminals[1].directory = "~/Documents".into();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    open(&mut rt, false);
    commands(&outgoing);
    press(&mut rt, Key::Text("active-project".into()));
    assert!(rt.find("palette-terminal-1").is_some());
    assert!(rt.find("palette-terminal-2").is_none());
    press(&mut rt, primary('a', false));
    press(&mut rt, Key::Text("new-project".into()));
    assert!(rt.find("palette-empty").is_some());
    let mut updated = rt.state.snapshot.clone().unwrap();
    updated.workspaces[0].terminals[1].directory = "~/code/new-project".into();
    rt.update(|s, _| s.apply(updated));
    rt.frame(0.0);
    assert!(rt.find("palette-terminal-2").is_some());
    assert!(rt.find("palette-terminal-1").is_none());
    assert_eq!(rt.state.active_terminal(), Some(1));
    assert!(commands(&outgoing).is_empty());
    // The original /tmp starting directory must not remain a stale search match.
    press(&mut rt, primary('a', false));
    press(&mut rt, Key::Text("/tmp".into()));
    assert!(rt.find("palette-terminal-1").is_none());
    assert!(rt.find("palette-terminal-2").is_none());
}

#[test]
fn custom_workspace_names_remain_searchable_alongside_each_terminals_directory() {
    let (mut state, _) = mock();
    let workspace = &mut state.snapshot.as_mut().unwrap().workspaces[0];
    workspace.terminals[0].directory = "~/Documents".into();
    workspace.terminals[1].directory = "~/code/griz".into();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    open(&mut rt, false);
    press(&mut rt, Key::Text("daily".into()));
    assert!(rt.find("palette-terminal-1").is_some());
    assert!(rt.find("palette-terminal-2").is_some());
    press(&mut rt, primary('a', false));
    press(&mut rt, Key::Text("daily documents".into()));
    assert!(rt.find("palette-terminal-1").is_some());
    assert!(rt.find("palette-terminal-2").is_none());
}

#[test]
fn quick_open_searches_across_workspaces_and_waits_for_the_final_selection_ack() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    click(&mut rt, "terminal");
    commands(&outgoing);
    open(&mut rt, false);
    press(&mut rt, Key::Text("other terminal".into()));
    assert!(rt.find("palette-terminal-3").is_some());
    assert!(rt.find("palette-terminal-1").is_none());
    assert!(commands(&outgoing).is_empty());
    press(&mut rt, Key::Enter);
    assert!(!rt.state.palette.is_open());
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 2);
    assert!(matches!(
        sent[0].1,
        Command::SelectWorkspace { workspace_id: 2 }
    ));
    assert!(matches!(
        sent[1].1,
        Command::SelectTerminal { terminal_id: 3 }
    ));
    assert_eq!(rt.state.pending, Some(sent[1].0));
    press(&mut rt, Key::Text("not yet".into()));
    assert!(commands(&outgoing).is_empty());
    let mut intermediate = snapshot();
    intermediate.ack = sent[0].0;
    intermediate.active_workspace = 2;
    rt.update(|s, _| s.apply(intermediate));
    assert_eq!(rt.state.active_terminal(), Some(1));
    assert_eq!(rt.state.pending, Some(sent[1].0));
    let mut final_view = snapshot();
    final_view.ack = sent[1].0;
    final_view.active_workspace = 2;
    final_view.active_terminal = Some(3);
    final_view.terminal.as_mut().unwrap().id = 3;
    rt.update(|s, _| s.apply(final_view));
    settle(&mut rt);
    commands(&outgoing);
    press(&mut rt, Key::Text("x".into()));
    assert!(
        commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::Input { terminal_id: 3, data } if data == b"x"))
    );
}

#[test]
fn control_n_p_and_arrows_wrap_and_filtering_resets_the_selection() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    open(&mut rt, false);
    commands(&outgoing);
    press(&mut rt, ctrl('p')); // first -> last
    press(&mut rt, ctrl('n')); // last -> first
    press(&mut rt, Key::Down); // -> Terminal 1
    press(&mut rt, Key::Down); // -> Terminal 2
    press(&mut rt, Key::Up); // -> Terminal 1
    press(&mut rt, ctrl('n')); // -> Terminal 2
    press(&mut rt, Key::Enter);
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(
        matches!(sent[0].1, Command::SelectTerminal { terminal_id: 2 }),
        "{sent:?}"
    );

    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    open(&mut rt, false);
    commands(&outgoing);
    press(&mut rt, ctrl('p')); // last -> other terminal
    press(&mut rt, Key::Text("daily".into()));
    press(&mut rt, Key::Enter);
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 1);
    assert!(matches!(
        sent[0].1,
        Command::SelectWorkspace { workspace_id: 1 }
    ));
}

#[test]
fn search_paste_no_matches_close_shortcut_and_escape_never_reach_the_shell() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    click(&mut rt, "terminal");
    commands(&outgoing);
    open(&mut rt, false);
    rt.paste("no-such-workspace-xyz");
    rt.frame(0.0);
    assert!(rt.find("palette-empty").is_some());
    press(&mut rt, ctrl('n'));
    press(&mut rt, ctrl('p'));
    press(&mut rt, Key::Enter);
    press(&mut rt, primary('w', false));
    assert!(rt.state.palette.is_open());
    assert!(rt.state.dialog.is_none());
    assert!(commands(&outgoing).is_empty());
    press(&mut rt, Key::Escape);
    settle(&mut rt);
    assert!(!rt.state.palette.is_open());
    press(&mut rt, Key::Text("x".into()));
    assert!(
        commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::Input { data, .. } if data == b"x"))
    );
}

#[test]
fn command_mode_can_switch_from_navigation_and_create_a_workspace_here() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    open(&mut rt, false);
    commands(&outgoing);
    press(&mut rt, Key::Text("discard this".into()));
    open(&mut rt, true);
    assert!(rt.find("palette-workspace-1").is_none());
    assert!(rt.find("palette-action-workspace.new").is_some());
    press(&mut rt, Key::Text("nwsp".into()));
    press(&mut rt, Key::Enter);
    settle(&mut rt);
    assert!(!rt.state.palette.is_open());
    assert!(matches!(rt.state.dialog, Some(Dialog::NewWorkspace)));
    assert!(commands(&outgoing).is_empty());
    press(&mut rt, Key::Enter);
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 1);
    assert!(matches!(&sent[0].1, Command::CreateWorkspace { name } if name.is_empty()));
}

#[test]
fn close_terminal_command_only_confirms_when_its_workspace_would_close() {
    for final_terminal in [false, true] {
        let (mut state, outgoing) = mock();
        if final_terminal {
            let mut last = snapshot();
            last.workspaces[0].terminals.truncate(1);
            last.workspaces[0].tabs.truncate(1);
            state.apply(last);
        }
        let mut rt = make_runtime(state, (1100.0, 720.0));
        open(&mut rt, true);
        commands(&outgoing);
        press(&mut rt, Key::Text("close active terminal".into()));
        settle(&mut rt);
        click(&mut rt, "palette-action-terminal.close");
        assert!(!rt.state.palette.is_open());
        let sent = commands(&outgoing);
        if final_terminal {
            assert!(matches!(rt.state.dialog, Some(Dialog::CloseTerminal(1, _))));
            assert!(sent.is_empty());
        } else {
            assert!(rt.state.dialog.is_none());
            assert!(
                sent.iter()
                    .any(|(_, c)| matches!(c, Command::CloseTerminal { terminal_id: 1 }))
            );
        }
    }
}

#[test]
fn clicking_a_command_keeps_existing_destructive_confirmation() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    open(&mut rt, true);
    commands(&outgoing);
    press(&mut rt, Key::Text("close workspace".into()));
    settle(&mut rt);
    click(&mut rt, "palette-close-workspace");
    assert!(matches!(
        rt.state.dialog,
        Some(Dialog::CloseWorkspace(1, _))
    ));
    assert!(commands(&outgoing).is_empty());
    click(&mut rt, "dialog-submit");
    assert!(
        commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::CloseWorkspace { workspace_id: 1 }))
    );
}

#[test]
fn palette_survives_catalog_changes_and_does_not_activate_a_removed_target() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    open(&mut rt, false);
    commands(&outgoing);
    press(&mut rt, Key::Text("other".into()));
    press(&mut rt, ctrl('n'));
    let mut updated = snapshot();
    updated.workspaces.pop();
    rt.update(|s, _| s.apply(updated));
    rt.frame(0.0);
    assert!(rt.find("palette-empty").is_some());
    press(&mut rt, Key::Enter);
    assert!(rt.state.palette.is_open());
    assert!(commands(&outgoing).is_empty());
}

#[test]
fn disconnected_palette_disables_session_actions_but_can_open_settings() {
    let (mut state, outgoing) = mock();
    state.online = false;
    let mut rt = make_runtime(state, (640.0, 420.0));
    open(&mut rt, false);
    commands(&outgoing);
    press(&mut rt, Key::Enter);
    assert!(commands(&outgoing).is_empty());
    assert!(rt.state.palette.is_open());
    open(&mut rt, true);
    press(&mut rt, Key::Text("settings".into()));
    press(&mut rt, Key::Enter);
    assert!(rt.state.settings_open);
    assert!(!rt.state.palette.is_open());
    assert!(commands(&outgoing).is_empty());
}

#[test]
fn palette_bindings_are_configurable_and_respect_other_modals() {
    let (mut state, outgoing) = mock();
    state
        .settings
        .bind(Action::QuickOpen, Some(Chord::parse("mod+o").unwrap()))
        .unwrap();
    state.settings.bind(Action::CommandPalette, None).unwrap();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    press(&mut rt, primary('p', false));
    assert!(!rt.state.palette.is_open());
    press(&mut rt, primary('o', false));
    assert!(rt.state.palette.is_open());
    press(&mut rt, Key::Escape);
    settle(&mut rt);
    commands(&outgoing);
    click(&mut rt, "new-workspace");
    press(&mut rt, primary('o', false));
    assert!(!rt.state.palette.is_open());
    assert!(matches!(rt.state.dialog, Some(Dialog::NewWorkspace)));
    assert!(commands(&outgoing).is_empty());
}

#[test]
fn palette_fits_small_windows_with_large_fonts_and_scrolls_keyboard_selection() {
    let (mut state, _) = mock();
    state.settings.font_size = 28.0;
    let mut rt = make_runtime(state, (640.0, 420.0));
    open(&mut rt, true);
    for _ in 0..14 {
        press(&mut rt, ctrl('n'));
    }
    let ([x, y, w, h], _) = rt.presented("command-palette").unwrap();
    assert!(x >= 0.0 && y >= 0.0 && x + w <= 640.1 && y + h <= 420.1);
    assert!(size(&rt, "palette-results")[1] > 30.0);
    press(&mut rt, Key::Escape);
    assert!(!rt.state.palette.is_open());
}

#[test]
fn outside_click_dismisses_without_activating_underlying_controls() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    open(&mut rt, false);
    commands(&outgoing);
    let (x, y) = rt.find("new-workspace").unwrap();
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    assert!(!rt.state.palette.is_open());
    assert!(rt.state.dialog.is_none());
    assert!(commands(&outgoing).is_empty());
}
