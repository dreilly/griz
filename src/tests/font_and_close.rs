use super::*;
use rune::{Key, Mods};

fn command(c: char, shift: bool) -> KeyEvent {
    KeyEvent {
        key: Key::Command(c),
        mods: Mods {
            cmd: true,
            shift,
            ..Mods::default()
        },
    }
}

fn text_height(rt: &Runtime<State>, value: &str) -> f32 {
    rt.elements()
        .iter()
        .find(|e| e.text.as_deref() == Some(value))
        .unwrap_or_else(|| panic!("missing text {value}"))
        .target[3]
}

#[test]
fn font_size_scales_workspace_chrome_settings_and_dialogs_not_just_the_terminal() {
    let mut rt = runtime((1100.0, 720.0));
    let title_height = text_height(&rt, "griz");
    let workspace_height = text_height(&rt, "Daily");
    let button_height = text_height(&rt, "Sidebar");
    click(&mut rt, "open-settings");
    let settings_height = text_height(&rt, "Settings");
    let label_height = text_height(&rt, "App font size");
    click(&mut rt, "settings-size-up");
    assert!(text_height(&rt, "griz") > title_height);
    assert!(text_height(&rt, "Daily") > workspace_height);
    assert!(text_height(&rt, "Sidebar") > button_height);
    assert!(text_height(&rt, "Settings") > settings_height);
    assert!(text_height(&rt, "App font size") > label_height);
    click(&mut rt, "settings-close");
    click(&mut rt, "new-workspace");
    let dialog_height = text_height(&rt, "New workspace");
    rt.key(command('+', true));
    rt.frame(0.0);
    assert!(text_height(&rt, "New workspace") > dialog_height);
}

#[test]
fn command_plus_equal_and_minus_work_in_settings_and_focused_fields_and_persist() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    click(&mut rt, "terminal");
    commands(&outgoing);
    rt.key(command('+', true));
    rt.frame(0.0);
    assert_eq!(rt.state.settings.font_size, 15.0);
    rt.key(command('=', false));
    rt.frame(0.0);
    assert_eq!(rt.state.settings.font_size, 16.0);
    rt.key(command('-', false));
    rt.frame(0.0);
    assert_eq!(rt.state.settings.font_size, 15.0);
    click(&mut rt, "open-settings");
    rt.key(command('+', true));
    rt.frame(0.0);
    assert_eq!(rt.state.settings.font_size, 16.0);
    click(&mut rt, "settings-close");
    click(&mut rt, "new-workspace");
    click(&mut rt, "dialog-input");
    rt.key(Key::Text("keep typing".into()).into());
    rt.frame(0.0);
    let before = size(&rt, "dialog-input")[1];
    rt.key(command('+', true));
    rt.frame(0.0);
    assert_eq!(rt.state.settings.font_size, 17.0);
    // The field stays focused and its text does not gain '+' or '-'.
    rt.key(Key::Text(" here".into()).into());
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
    assert_eq!(rt.state.draft.text, "keep typing here");
    assert!(size(&rt, "dialog-input")[1] > before);
    assert_eq!(Settings::from_store(&rt.state.prefs).0.font_size, 17.0);
    assert!(
        !commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::Input { .. }))
    );
}

#[test]
fn command_w_closes_a_nonfinal_terminal_immediately_by_its_id() {
    let (mut state, outgoing) = mock();
    let mut selected = snapshot();
    selected.workspaces[0].terminals[1].id = 47;
    selected.workspaces[0].tabs[1] = tab(47);
    selected.active_terminal = Some(47);
    selected.terminal.as_mut().unwrap().id = 47;
    state.apply(selected);
    let mut rt = make_runtime(state, (1100.0, 720.0));
    click(&mut rt, "terminal");
    commands(&outgoing);
    rt.key(command('w', false));
    rt.frame(0.0);
    let sent = commands(&outgoing);
    assert!(rt.state.dialog.is_none());
    assert_eq!(sent.len(), 1);
    assert!(matches!(
        sent[0].1,
        Command::CloseTerminal { terminal_id: 47 }
    ));
    assert!(rt.state.pending.is_some());
}

#[test]
fn final_terminal_confirmation_captures_the_terminal_id_and_never_retargets() {
    let (mut state, outgoing) = mock();
    let mut selected = snapshot();
    selected.workspaces[0].terminals[1].id = 47;
    selected.workspaces[0].terminals.remove(0);
    selected.workspaces[0].tabs = vec![tab(47)];
    selected.active_terminal = Some(47);
    selected.terminal.as_mut().unwrap().id = 47;
    state.apply(selected);
    let mut rt = make_runtime(state, (1100.0, 720.0));
    click(&mut rt, "terminal");
    commands(&outgoing);
    rt.key(command('w', false));
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
    assert!(matches!(
        rt.state.dialog,
        Some(Dialog::CloseTerminal(47, _))
    ));
    assert!(
        commands(&outgoing).is_empty(),
        "closing still requires confirmation"
    );
    // A later selection update must not retarget an already-open confirmation.
    rt.update(|s, _| s.apply(snapshot()));
    rt.frame(0.0);
    commands(&outgoing); // Selection reconciliation may resize the newly selected terminal.
    click(&mut rt, "dialog-submit");
    let sent = commands(&outgoing);
    assert_eq!(sent.len(), 1);
    assert!(matches!(
        sent[0].1,
        Command::CloseTerminal { terminal_id: 47 }
    ));
}

#[test]
fn command_w_is_safe_in_settings_and_with_no_active_terminal() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    click(&mut rt, "open-settings");
    commands(&outgoing);
    rt.key(command('w', false));
    rt.frame(0.0);
    assert!(rt.state.settings_open);
    assert!(rt.state.dialog.is_none());
    assert!(commands(&outgoing).is_empty());
    click(&mut rt, "settings-close");
    let mut empty = snapshot();
    empty.workspaces[0].terminals.clear();
    empty.active_terminal = None;
    empty.terminal = None;
    rt.update(|s, _| s.apply(empty));
    rt.frame(0.0);
    rt.key(command('w', false));
    rt.frame(0.0);
    assert!(rt.state.dialog.is_none());
    assert!(commands(&outgoing).is_empty());
}
