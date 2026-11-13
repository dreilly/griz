use super::*;
use crate::settings::{Action, Font, Settings, TerminalWhites, Theme};
use griz_protocol::WorkspaceAccent;
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
            .any(|(_, command)| key_text(command, 1, "x"))
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
    rt.update(|_, tx| {
        tx.scroll_into_view("settings-appearance-reset");
        tx.instant();
    });
    rt.frame(0.0);
    click(&mut rt, "settings-appearance-reset");
    assert_eq!(rt.state.settings.font_size, 14.0);
    assert_eq!(rt.state.settings.font, Font::Menlo);
    assert_eq!(rt.state.settings.theme, Theme::Dark);
    assert_eq!(rt.state.grid, original_grid);
}

#[test]
fn default_workspace_accent_is_green_live_and_persistent_without_recoloring_existing_workspaces() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    open_settings(&mut rt);
    assert_eq!(
        rt.state.settings.default_workspace_accent,
        WorkspaceAccent::Green
    );
    assert!(rt.find("settings-buttons").is_none());
    commands(&outgoing);
    for accent in WorkspaceAccent::ALL {
        let name = rune::tok::ACCENT_NAMES[workspace_ui::accent_index(accent)];
        click(&mut rt, &format!("settings-accent-{name}"));
        assert_eq!(rt.state.settings.default_workspace_accent, accent);
        assert_eq!(
            Settings::from_store(&rt.state.prefs)
                .0
                .default_workspace_accent,
            accent
        );
        assert!(
            rt.state
                .snapshot
                .as_ref()
                .unwrap()
                .workspaces
                .iter()
                .all(|w| w.accent == WorkspaceAccent::Green)
        );
        assert!(
            commands(&outgoing).is_empty(),
            "a preference change must not recolor existing workspaces"
        );
    }
}

#[test]
fn default_workspace_accent_supports_keyboard_selection() {
    let mut rt = runtime((1100.0, 720.0));
    open_settings(&mut rt);
    click(&mut rt, "settings-accent-green");
    rt.key(Key::Right.into());
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
    assert_eq!(
        rt.state.settings.default_workspace_accent,
        WorkspaceAccent::Graphite
    );
    assert_eq!(
        Settings::from_store(&rt.state.prefs)
            .0
            .default_workspace_accent,
        WorkspaceAccent::Graphite
    );
}

#[test]
fn workspace_creation_uses_the_default_accent_for_named_blank_and_empty_state_actions() {
    for named in [true, false] {
        let (mut state, outgoing) = mock();
        state.settings.default_workspace_accent = WorkspaceAccent::Purple;
        let mut rt = make_runtime(state, (1100.0, 720.0));
        commands(&outgoing);
        click(&mut rt, "new-workspace");
        if named {
            click(&mut rt, "dialog-input");
            rt.key(Key::Text("Project".into()).into());
            rt.frame(0.0);
        }
        click(&mut rt, "dialog-submit");
        assert!(commands(&outgoing).iter().any(|(_, command)|
            matches!(command, Command::CreateWorkspace { name, accent: WorkspaceAccent::Purple }
                if name == if named { "Project" } else { "" })));
    }
    let (mut state, outgoing) = mock();
    state.settings.default_workspace_accent = WorkspaceAccent::Orange;
    let mut empty = snapshot();
    empty.workspaces.clear();
    empty.active_workspace = 0;
    empty.active_terminal = None;
    empty.terminal = None;
    state.apply(empty);
    let mut rt = make_runtime(state, (1100.0, 720.0));
    commands(&outgoing);
    click(&mut rt, "empty-workspaces-create");
    assert!(commands(&outgoing).iter().any(|(_, command)|
        matches!(command, Command::CreateWorkspace { name, accent: WorkspaceAccent::Orange } if name.is_empty())));
}

#[test]
fn retired_button_style_is_ignored_and_removed_on_save() {
    let mut prefs = Prefs::memory();
    prefs
        .set(
            "settings",
            &serde_json::json!({ "workspace_button_style": "solid" }),
        )
        .unwrap();
    let (settings, warning) = Settings::from_store(&prefs);
    assert!(warning.is_none());
    assert_eq!(settings.default_workspace_accent, WorkspaceAccent::Green);
    settings.save(&mut prefs).unwrap();
    assert!(
        prefs
            .get::<serde_json::Value>("settings")
            .unwrap()
            .get("workspace_button_style")
            .is_none()
    );
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
        Action::ToggleSidebar.default_chord()
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
            .any(|(_, command)| terminal_input(command))
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
    settings.terminal_whites = TerminalWhites::Softer;
    settings.default_workspace_accent = WorkspaceAccent::Purple;
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
    assert_eq!(loaded.terminal_whites, TerminalWhites::Softer);
    assert_eq!(loaded.default_workspace_accent, WorkspaceAccent::Purple);
    assert_eq!(
        loaded.chord(Action::ToggleSidebar),
        Some(Chord::parse("mod+j").unwrap())
    );
    assert!(loaded.chord(Action::NewTerminal).is_none());
    assert_eq!(
        loaded.chord(Action::NextTab),
        Action::NextTab.default_chord()
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
                "default_workspace_accent": "unknown",
                "keybindings": { "sidebar.toggle": "mod+t", "terminal.new": "nonsense key" }
            }),
        )
        .unwrap();
    let (settings, warning) = Settings::from_store(&prefs);
    assert!(warning.is_some());
    assert_eq!(settings.font_size, 14.0);
    assert_eq!(settings.font, Font::Menlo);
    assert_eq!(settings.theme, Theme::Dark);
    assert_eq!(settings.default_workspace_accent, WorkspaceAccent::Green);
    prefs
        .set("settings", &serde_json::json!({"font_size": 16}))
        .unwrap();
    let (settings, warning) = Settings::from_store(&prefs);
    assert_eq!(settings.font_size, 16.0);
    assert_eq!(settings.default_workspace_accent, WorkspaceAccent::Green);
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
            Action::ToggleSidebar.default_chord()
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
fn settings_sidebar_and_close_stay_fixed_while_details_scroll_and_pages_start_at_top() {
    let mut rt = runtime((1100.0, 720.0));
    open_settings(&mut rt);
    let navigation = rt.presented("settings-navigation").unwrap().0;
    let close = rt.presented("settings-close").unwrap().0;
    let content = rt.presented("settings-content").unwrap().0;
    assert!(navigation[0] + navigation[2] <= content[0]);
    click(&mut rt, "settings-shortcuts");
    rt.update(|_, tx| {
        tx.scroll_into_view("shortcut-record-pane.previous");
        tx.instant();
    });
    rt.frame(0.0);
    assert_eq!(rt.presented("settings-navigation").unwrap().0, navigation);
    assert_eq!(rt.presented("settings-close").unwrap().0, close);
    click(&mut rt, "settings-appearance");
    let ([_, y, _, height], _) = rt.presented("settings-theme-0").unwrap();
    assert!(y >= content[1] && y + height <= content[1] + content[3]);
    let elements = rt.elements();
    let fill = |key: &str| {
        elements
            .iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap()
            .fill
    };
    assert_ne!(fill("settings-appearance"), rune::Color::TRANSPARENT);
    assert_eq!(fill("settings-shortcuts"), rune::Color::TRANSPARENT);
}

#[test]
fn appearance_controls_fit_small_windows_and_large_fonts_without_horizontal_overflow() {
    for font_size in [14.0, 28.0] {
        let mut state = State::default();
        state.settings.font_size = font_size;
        let mut rt = make_runtime(state, (640.0, 420.0));
        open_settings(&mut rt);
        for key in [
            "settings-theme-0",
            "settings-theme-1",
            "settings-accent-blue",
            "settings-accent-purple",
            "settings-accent-pink",
            "settings-accent-orange",
            "settings-accent-green",
            "settings-accent-graphite",
            "settings-whites-Original",
            "settings-whites-Soft",
            "settings-whites-Softer",
            "settings-font-0",
            "settings-font-1",
            "settings-font-2",
            "settings-size-down",
            "settings-size-up",
            "settings-preview",
            "settings-appearance-reset",
        ] {
            rt.update(|_, tx| {
                tx.scroll_into_view(key);
                tx.instant();
            });
            rt.frame(0.0);
            let ([x, _, width, _], _) = rt.presented(key).unwrap();
            let ([cx, _, cw, _], _) = rt.presented("settings-content").unwrap();
            assert!(
                x >= cx && x + width <= cx + cw,
                "{key} overflows at {font_size}pt"
            );
        }
    }
}

#[test]
fn appearance_reset_restores_the_default_workspace_accent_to_green() {
    let mut state = State::default();
    state.settings.default_workspace_accent = WorkspaceAccent::Purple;
    let mut rt = make_runtime(state, (1100.0, 720.0));
    open_settings(&mut rt);
    rt.update(|_, tx| {
        tx.scroll_into_view("settings-appearance-reset");
        tx.instant();
    });
    rt.frame(0.0);
    click(&mut rt, "settings-appearance-reset");
    assert_eq!(
        rt.state.settings.default_workspace_accent,
        WorkspaceAccent::Green
    );
    assert_eq!(
        Settings::from_store(&rt.state.prefs)
            .0
            .default_workspace_accent,
        WorkspaceAccent::Green
    );
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
