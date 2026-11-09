use super::*;
use crate::settings::{Action, Font, Settings, Theme, WorkspaceButtonStyle};
use rune::{Chord, Key, Mods, prefs::Prefs};

fn primary(key: Key) -> KeyEvent {
    KeyEvent {
        key,
        mods: Mods {
            cmd: true,
            ..Mods::default()
        },
    }
}

#[test]
fn settings_open_by_palette_or_command_comma_and_never_type_into_shell() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    click(&mut rt, "terminal");
    commands(&outgoing);
    rt.key(primary(Key::Command(',')));
    rt.frame(0.0);
    assert!(rt.state.settings_open);
    assert!(rt.find("settings-panel").is_some());
    rt.key(Key::Text("must not reach shell".into()).into());
    rt.paste("nor this paste");
    rt.frame(0.0);
    assert!(commands(&outgoing).is_empty());
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
    click(&mut rt, "settings-close");
    assert!(!rt.state.settings_open);
    open_settings(&mut rt);
    assert!(rt.state.settings_open);
    rt.key(Key::Escape.into());
    rt.frame(0.0);
    assert!(!rt.state.settings_open);
    rt.key(Key::Text("x".into()).into());
    rt.frame(0.0);
    assert!(
        commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::Input { data, .. } if data == b"x"))
    );
}

#[test]
fn appearance_is_live_reflows_the_pty_and_can_be_reset() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    rt.frame(0.0);
    let original_grid = rt.state.grid;
    open_settings(&mut rt);
    commands(&outgoing);
    click(&mut rt, "settings-size-up");
    assert_eq!(rt.state.settings.font_size, 15.0);
    assert!(rt.state.grid.0 < original_grid.0);
    assert!(rt.state.grid.1 < original_grid.1);
    assert!(
        commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::Resize { terminal_id: 1, .. }))
    );
    assert_eq!(Settings::from_store(&rt.state.prefs).0.font_size, 15.0);
    click(&mut rt, "settings-font-1");
    assert_eq!(rt.state.settings.font, Font::Monaco);
    click(&mut rt, "settings-theme-1");
    assert_eq!(rt.state.settings.theme, Theme::Midnight);
    let elements = rt.elements();
    let terminal = elements
        .iter()
        .find(|e| e.key.as_deref() == Some("terminal"))
        .unwrap();
    assert_eq!(
        terminal.fill,
        terminal_view::background_for(Theme::Midnight)
    );
    click(&mut rt, "settings-appearance-reset");
    assert_eq!(rt.state.settings.font_size, 14.0);
    assert_eq!(rt.state.settings.font, Font::Menlo);
    assert_eq!(rt.state.settings.theme, Theme::Dark);
    assert_eq!(rt.state.grid, original_grid);
}

#[test]
fn workspace_button_gallery_selects_saves_and_applies_styles_without_creating_workspaces() {
    let (mut state, outgoing) = mock();
    let mut empty = snapshot();
    empty.workspaces.clear();
    empty.active_workspace = 0;
    empty.active_terminal = None;
    empty.terminal = None;
    state.apply(empty);
    let mut rt = make_runtime(state, (1100.0, 720.0));
    open_settings(&mut rt);
    click(&mut rt, "settings-buttons");
    commands(&outgoing);
    for style in WorkspaceButtonStyle::ALL {
        let key = format!("button-style-{}", style.id());
        assert!(rt.find(&key).is_some());
        rt.update(|_, tx| {
            tx.scroll_into_view(&key);
            tx.instant();
        });
        rt.frame(0.0);
        click(&mut rt, &key);
        assert_eq!(rt.state.settings.workspace_button_style, style);
        assert_eq!(
            Settings::from_store(&rt.state.prefs)
                .0
                .workspace_button_style,
            style
        );
        assert!(rt.state.dialog.is_none());
        assert!(
            commands(&outgoing).is_empty(),
            "samples must not create terminals or workspaces"
        );
        let elements = rt.elements();
        let live = elements
            .iter()
            .find(|e| e.key.as_deref() == Some("empty-workspaces-create"))
            .unwrap();
        let sample = elements
            .iter()
            .find(|e| e.key.as_deref() == Some(key.as_str()))
            .unwrap();
        assert_eq!(
            live.fill, sample.fill,
            "the sample must match the actual button"
        );
    }
    rt.update(|_, tx| {
        tx.scroll_into_view("settings-buttons-reset");
        tx.instant();
    });
    rt.frame(0.0);
    click(&mut rt, "settings-buttons-reset");
    assert_eq!(
        rt.state.settings.workspace_button_style,
        WorkspaceButtonStyle::Soft
    );
    assert_eq!(
        Settings::from_store(&rt.state.prefs)
            .0
            .workspace_button_style,
        WorkspaceButtonStyle::Soft
    );
    assert!(commands(&outgoing).is_empty());
}

#[test]
fn workspace_button_gallery_is_usable_disconnected_and_in_a_small_window() {
    let mut rt = make_runtime(State::default(), (640.0, 420.0));
    open_settings(&mut rt);
    click(&mut rt, "settings-buttons");
    for font_size in [14.0, 28.0] {
        rt.update(|s, _| s.settings.font_size = font_size);
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        for style in WorkspaceButtonStyle::ALL {
            let key = format!("button-style-{}", style.id());
            rt.update(|_, tx| {
                tx.scroll_into_view(&key);
                tx.instant();
            });
            rt.frame(0.0);
            let ([x, _, width, _], _) = rt.presented(&key).unwrap();
            let ([cx, _, cw, _], _) = rt.presented("settings-content").unwrap();
            assert!(
                x >= cx && x + width <= cx + cw,
                "sample must not overflow horizontally"
            );
            click(&mut rt, &key);
            assert_eq!(rt.state.settings.workspace_button_style, style);
        }
    }
    assert!(!rt.state.online);
    assert!(rt.state.dialog.is_none());
}

#[test]
fn every_workspace_button_style_round_trips_without_affecting_other_preferences() {
    let mut prefs = Prefs::memory();
    for style in WorkspaceButtonStyle::ALL {
        let mut settings = Settings::default();
        settings.workspace_button_style = style;
        settings.font_size = 18.0;
        settings.save(&mut prefs).unwrap();
        let (loaded, warning) = Settings::from_store(&prefs);
        assert!(warning.is_none());
        assert_eq!(loaded.workspace_button_style, style);
        assert_eq!(loaded.font_size, 18.0);
    }
}

#[test]
fn shortcut_recording_conflicts_clear_reset_and_search() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    open_settings(&mut rt);
    click(&mut rt, "settings-shortcuts");
    commands(&outgoing);
    click(&mut rt, "shortcut-record-sidebar.toggle");
    rt.key(primary(Key::Command('t')));
    rt.frame(0.0);
    assert_eq!(rt.state.recording, Some(Action::ToggleSidebar));
    assert!(
        rt.state
            .settings_error
            .as_deref()
            .unwrap()
            .contains("already used")
    );
    rt.key(primary(Key::Command('j')));
    rt.frame(0.0);
    assert!(rt.state.recording.is_none());
    assert!(rt.state.settings_error.is_none());
    assert_eq!(
        rt.state.settings.chord(Action::ToggleSidebar),
        Some(Chord::parse("mod+j").unwrap())
    );
    assert!(commands(&outgoing).is_empty());
    click(&mut rt, "shortcut-clear-sidebar.toggle");
    assert!(rt.state.settings.chord(Action::ToggleSidebar).is_none());
    click(&mut rt, "shortcut-reset-sidebar.toggle");
    assert_eq!(
        rt.state.settings.chord(Action::ToggleSidebar),
        Some(Action::ToggleSidebar.default_chord())
    );
    click(&mut rt, "settings-shortcut-search");
    rt.key(Key::Text("sidebar".into()).into());
    rt.frame(0.0);
    assert!(rt.find("shortcut-record-sidebar.toggle").is_some());
    assert!(rt.find("shortcut-record-terminal.new").is_none());
    click(&mut rt, "shortcut-record-sidebar.toggle");
    rt.key(Key::Escape.into());
    rt.frame(0.0);
    assert!(rt.state.recording.is_none());
    assert!(
        rt.state.settings_open,
        "Escape cancels recording before dismissing Settings"
    );
}

#[test]
fn custom_shortcuts_dispatch_actions_without_leaking_bytes() {
    let (mut state, outgoing) = mock();
    state
        .settings
        .bind(Action::ToggleSidebar, Some(Chord::parse("mod+j").unwrap()))
        .unwrap();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    click(&mut rt, "terminal");
    commands(&outgoing);
    rt.key(primary(Key::Command('j')));
    rt.frame(0.0);
    assert!(!rt.state.sidebar_open);
    assert!(
        !commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::Input { .. }))
    );
    // Let the expanded sidebar finish leaving before clicking its collapsed toggle.
    command_palette_tests::settle(&mut rt);
    // App shortcuts must not steal keys from a workspace-name field.
    click(&mut rt, "toggle-sidebar");
    click(&mut rt, "new-workspace");
    click(&mut rt, "dialog-input");
    rt.key(primary(Key::Command('j')));
    rt.frame(0.0);
    assert!(rt.state.sidebar_open);
}

#[test]
fn settings_are_available_disconnected_and_fit_a_small_window() {
    let mut rt = make_runtime(State::default(), (640.0, 420.0));
    open_settings(&mut rt);
    assert!(rt.state.settings_open);
    let ([x, y, width, height], _) = rt.presented("settings-panel").unwrap();
    assert!(x >= 0.0 && y >= 0.0 && x + width <= 640.1 && y + height <= 420.1);
    assert!(rt.find("settings-close").is_some());
    click(&mut rt, "settings-shortcuts");
    assert!(rt.find("settings-shortcut-search").is_some());
}

#[test]
fn preferences_round_trip_with_disabled_bindings_and_missing_defaults() {
    let dir = std::env::temp_dir().join(format!("griz-settings-roundtrip-{}", std::process::id()));
    let path = dir.join("prefs.json");
    let _ = std::fs::remove_dir_all(&dir);
    let mut prefs = Prefs::at(&path);
    let mut settings = Settings::default();
    settings.font = Font::Monaco;
    settings.font_size = 19.0;
    settings.theme = Theme::Midnight;
    settings.workspace_button_style = WorkspaceButtonStyle::Outline;
    settings
        .bind(Action::ToggleSidebar, Some(Chord::parse("mod+j").unwrap()))
        .unwrap();
    settings.bind(Action::NewTerminal, None).unwrap();
    settings.save(&mut prefs).unwrap();
    let (loaded, warning) = Settings::from_store(&Prefs::at(&path));
    assert!(warning.is_none());
    assert_eq!(loaded.font, Font::Monaco);
    assert_eq!(loaded.font_size, 19.0);
    assert_eq!(loaded.theme, Theme::Midnight);
    assert_eq!(loaded.workspace_button_style, WorkspaceButtonStyle::Outline);
    assert_eq!(
        loaded.chord(Action::ToggleSidebar),
        Some(Chord::parse("mod+j").unwrap())
    );
    assert!(loaded.chord(Action::NewTerminal).is_none());
    assert_eq!(
        loaded.chord(Action::NextTab),
        Some(Action::NextTab.default_chord())
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn malformed_or_invalid_preferences_fall_back_without_panicking() {
    let mut prefs = Prefs::memory();
    prefs
        .set(
            "settings",
            &serde_json::json!({
                "font_size": 999,
                "font": "not installed",
                "theme": 42,
                "workspace_button_style": "unknown",
                "keybindings": { "sidebar.toggle": "mod+t", "terminal.new": "nonsense key" }
            }),
        )
        .unwrap();
    let (settings, warning) = Settings::from_store(&prefs);
    assert!(warning.is_some());
    assert_eq!(settings.font_size, 14.0);
    assert_eq!(settings.font, Font::Menlo);
    assert_eq!(settings.theme, Theme::Dark);
    assert_eq!(settings.workspace_button_style, WorkspaceButtonStyle::Soft);
    prefs
        .set("settings", &serde_json::json!({"font_size": 16}))
        .unwrap();
    let (settings, warning) = Settings::from_store(&prefs);
    assert_eq!(settings.font_size, 16.0);
    assert_eq!(settings.workspace_button_style, WorkspaceButtonStyle::Soft);
    assert!(warning.is_none());
}

#[test]
fn conflicts_and_reserved_or_unmodified_shortcuts_do_not_mutate_the_model() {
    let mut settings = Settings::default();
    for spec in [
        "mod+t",
        "mod+w",
        "mod+equal",
        "mod+shift+equal",
        "mod+plus",
        "mod+minus",
        "mod+comma",
        "mod+q",
        "mod+v",
        "j",
        "shift+j",
    ] {
        assert!(
            settings
                .bind(Action::ToggleSidebar, Some(Chord::parse(spec).unwrap()))
                .is_err(),
            "accepted {spec}"
        );
        assert_eq!(
            settings.chord(Action::ToggleSidebar),
            Some(Action::ToggleSidebar.default_chord())
        );
    }
    settings
        .bind(Action::ToggleSidebar, Some(Chord::parse("mod+j").unwrap()))
        .unwrap();
    settings
        .bind(Action::NewTerminal, Some(Chord::parse("mod+b").unwrap()))
        .unwrap();
    assert!(settings.reset_binding(Action::ToggleSidebar).is_err());
}

#[test]
fn save_failure_is_visible_but_the_live_preference_still_applies() {
    let dir = std::env::temp_dir().join(format!("griz-settings-error-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let blocked_parent = dir.join("file-not-directory");
    std::fs::write(&blocked_parent, "occupied").unwrap();
    let (mut state, _) = mock();
    state.prefs = Prefs::at(blocked_parent.join("prefs.json"));
    let mut rt = make_runtime(state, (1100.0, 720.0));
    open_settings(&mut rt);
    click(&mut rt, "settings-size-up");
    assert_eq!(rt.state.settings.font_size, 15.0);
    assert!(rt.state.settings_error.is_some());
    assert!(rt.find("settings-error").is_some());
    std::fs::remove_dir_all(dir).unwrap();
}
