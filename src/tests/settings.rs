use super::*;
use crate::settings::{Action, Font, Settings, Theme};
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
fn settings_open_by_button_or_command_comma_and_never_type_into_shell() {
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
    click(&mut rt, "open-settings");
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
    click(&mut rt, "open-settings");
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
fn shortcut_recording_conflicts_clear_reset_and_search() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    click(&mut rt, "open-settings");
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
    click(&mut rt, "open-settings");
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
                "keybindings": { "sidebar.toggle": "mod+t", "terminal.new": "nonsense key" }
            }),
        )
        .unwrap();
    let (settings, warning) = Settings::from_store(&prefs);
    assert!(warning.is_some());
    assert_eq!(settings.font_size, 14.0);
    assert_eq!(settings.font, Font::Menlo);
    assert_eq!(settings.theme, Theme::Dark);
    prefs
        .set("settings", &serde_json::json!({"font_size": 16}))
        .unwrap();
    let (settings, warning) = Settings::from_store(&prefs);
    assert_eq!(settings.font_size, 16.0);
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
    click(&mut rt, "open-settings");
    click(&mut rt, "settings-size-up");
    assert_eq!(rt.state.settings.font_size, 15.0);
    assert!(rt.state.settings_error.is_some());
    assert!(rt.find("settings-error").is_some());
    std::fs::remove_dir_all(dir).unwrap();
}
