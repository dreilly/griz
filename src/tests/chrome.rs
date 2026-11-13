use super::*;
use rune::{Chord, Key};

fn shortcut(rt: &Runtime<State>) -> Option<String> {
    rt.elements()
        .iter()
        .find(|e| e.key.as_deref() == Some("quick-open-shortcut"))
        .and_then(|e| e.text.clone())
}

#[test]
fn search_and_workspace_controls_replace_the_old_buttons_and_leave_a_collapsed_rail() {
    let mut rt = runtime((1100.0, 720.0));
    for key in [
        "quick-open",
        "search-icon",
        "toggle-sidebar",
        "sidebar-icon",
    ] {
        assert!(rt.find(key).is_some(), "missing {key}");
    }
    for key in ["open-settings", "new-terminal", "reconnect"] {
        assert!(rt.find(key).is_none(), "unexpected {key}");
    }
    assert_eq!(
        shortcut(&rt),
        Some(Action::QuickOpen.default_chord().unwrap().label())
    );
    assert!(size(&rt, "quick-open")[0] > 400.0);
    let [width, height] = size(&rt, "toggle-sidebar");
    assert_eq!(width, height);
    let (new_workspace, _) = rt.presented("new-workspace").unwrap();
    let (toggle, _) = rt.presented("toggle-sidebar").unwrap();
    assert!((toggle[0] - new_workspace[0] - new_workspace[2] - 4.0).abs() < 0.1);
    assert!((toggle[1] - new_workspace[1]).abs() < 0.1);
    assert!(toggle[1] >= TITLEBAR_HEIGHT);
    assert!(
        !rt.elements()
            .iter()
            .any(|e| e.text.as_deref() == Some("Sidebar"))
    );
    let expanded_terminal_width = size(&rt, "terminal")[0];
    click(&mut rt, "toggle-sidebar");
    assert!(rt.find("sidebar").is_none());
    assert!(rt.find("new-workspace").is_none());
    assert!(rt.find("workspace-list").is_none());
    assert!(rt.find("agents-section").is_none());
    assert!(rt.find("quick-open").is_some());
    let (rail, _) = rt.presented("sidebar-rail").unwrap();
    let (toggle, _) = rt.presented("toggle-sidebar").unwrap();
    assert_eq!(rail[0], 0.0);
    assert_eq!(rail[1], TITLEBAR_HEIGHT);
    assert!((rail[2] - width - 24.0).abs() < 0.1);
    assert!((toggle[0] - rail[0] - 12.0).abs() < 0.1);
    assert!((toggle[1] - rail[1] - 12.0).abs() < 0.1);
    assert!(size(&rt, "terminal")[0] > expanded_terminal_width);
    click(&mut rt, "toggle-sidebar");
    assert!(rt.find("sidebar").is_some());
    assert!(rt.find("sidebar-rail").is_none());
    assert!(rt.find("new-workspace").is_some());
}

#[test]
fn clicking_search_opens_focused_quick_open_without_typing_into_the_shell() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    click(&mut rt, "terminal");
    commands(&outgoing);
    click(&mut rt, "quick-open");
    assert!(rt.state.palette.is_open());
    assert!(rt.find("palette-workspace-1").is_some());
    assert!(
        rt.elements()
            .iter()
            .any(|e| { e.key.as_deref() == Some("palette-input") && e.focused })
    );
    command_palette_tests::press(&mut rt, Key::Text("other".into()));
    assert!(rt.find("palette-workspace-2").is_some());
    assert!(rt.find("palette-workspace-1").is_none());
    command_palette_tests::press(&mut rt, Key::Escape);
    command_palette_tests::settle(&mut rt);
    assert!(!rt.state.palette.is_open());
    assert!(
        !commands(&outgoing)
            .iter()
            .any(|(_, command)| terminal_input(command))
    );
    command_palette_tests::press(&mut rt, Key::Text("x".into()));
    assert!(
        commands(&outgoing)
            .iter()
            .any(|(_, command)| key_text(command, 1, "x"))
    );
}

#[test]
fn search_displays_its_rebound_shortcut_and_still_opens_when_unbound() {
    let (mut state, _) = mock();
    let chord = Chord::parse("mod+o").unwrap();
    state.settings.bind(Action::QuickOpen, Some(chord)).unwrap();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    assert_eq!(shortcut(&rt), Some(chord.label()));
    click(&mut rt, "quick-open");
    assert!(rt.find("palette-workspace-1").is_some());
    command_palette_tests::press(&mut rt, Key::Escape);
    command_palette_tests::settle(&mut rt);
    rt.update(|s, _| s.settings.bind(Action::QuickOpen, None).unwrap());
    command_palette_tests::settle(&mut rt);
    assert_eq!(shortcut(&rt), None);
    click(&mut rt, "quick-open");
    assert!(rt.find("palette-workspace-1").is_some());
}

#[test]
fn search_and_sidebar_control_fit_small_windows_at_all_font_sizes() {
    for font_size in [14.0, 28.0] {
        let (mut state, _) = mock();
        state.settings.font_size = font_size;
        let mut rt = make_runtime(state, (640.0, 420.0));
        let (search, _) = rt.presented("quick-open").unwrap();
        let (toggle, _) = rt.presented("toggle-sidebar").unwrap();
        let (sidebar, _) = rt.presented("sidebar").unwrap();
        let (new_workspace, _) = rt.presented("new-workspace").unwrap();
        let (shortcut, _) = rt.presented("quick-open-shortcut").unwrap();
        assert!(search[2] > 200.0);
        assert!(search[0] >= 0.0 && search[1] >= 0.0);
        assert!(search[0] + search[2] <= 640.0);
        assert!(toggle[0] >= new_workspace[0] + new_workspace[2]);
        assert!(toggle[0] + toggle[2] <= sidebar[0] + sidebar[2]);
        assert!(toggle[1] >= TITLEBAR_HEIGHT);
        assert!(search[1] + search[3] <= TITLEBAR_HEIGHT);
        assert!(shortcut[0] >= search[0]);
        assert!(shortcut[0] + shortcut[2] <= search[0] + search[2]);
        click(&mut rt, "quick-open");
        assert!(rt.state.palette.is_open());
    }
}

#[test]
fn reconnect_remains_discoverable_in_the_command_palette_when_disconnected() {
    let (mut state, _) = mock();
    state.online = false;
    let mut rt = make_runtime(state, (640.0, 420.0));
    assert!(rt.find("reconnect").is_none());
    command_palette_tests::open(&mut rt, true);
    command_palette_tests::press(&mut rt, Key::Text("reconnect".into()));
    command_palette_tests::settle(&mut rt);
    assert!(rt.find("palette-reconnect").is_some());
    assert!(
        rt.elements()
            .iter()
            .any(|e| e.text.as_deref() == Some("Reconnect to service"))
    );
}
