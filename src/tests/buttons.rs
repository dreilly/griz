use super::*;
use crate::settings::Theme;
use rune::Color;

fn surface(rt: &Runtime<State>, key: &str) -> (Color, f32) {
    let element = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("missing {key}"));
    (element.fill, element.radius)
}

#[test]
fn regular_buttons_use_soft_in_both_themes_and_at_large_font_sizes() {
    for theme in Theme::ALL {
        for font_size in [14.0, 28.0] {
            let (mut state, _) = mock();
            let mut empty = snapshot();
            empty.workspaces.clear();
            empty.active_workspace = 0;
            empty.active_terminal = None;
            empty.terminal = None;
            state.apply(empty);
            state.settings.theme = theme;
            state.settings.font_size = font_size;
            let mut rt = make_runtime(state, (1100.0, 720.0));
            let soft = surface(&rt, "new-workspace");
            assert_ne!(soft.0, Color::TRANSPARENT);
            assert_eq!(soft.1, 6.0);
            for key in ["toggle-sidebar", "empty-workspaces-create"] {
                assert_eq!(surface(&rt, key), soft, "{theme:?}, {font_size}pt: {key}");
            }
            open_settings(&mut rt);
            assert!(rt.find("settings-buttons").is_none());
        }
    }
}

#[test]
fn dialogs_use_soft_while_settings_choices_have_quiet_distinct_selected_states() {
    for theme in Theme::ALL {
        let (mut state, _) = mock();
        state.settings.theme = theme;
        let mut rt = make_runtime(state, (1100.0, 720.0));
        let soft = surface(&rt, "new-workspace");
        click(&mut rt, "new-workspace");
        assert_eq!(surface(&rt, "dialog-cancel"), soft);
        assert_eq!(surface(&rt, "dialog-submit"), soft);
        click(&mut rt, "dialog-cancel");
        rt.update(|s, tx| s.open_dialog(Dialog::WorkspaceMenu(1, (260.0, 120.0)), tx));
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(surface(&rt, "workspace-menu-rename"), soft);
        assert_eq!(surface(&rt, "workspace-menu-close"), soft);
        rt.key(rune::Key::Escape.into());
        rt.frame(0.0);
        open_settings(&mut rt);
        assert_eq!(surface(&rt, "settings-font-1").0, Color::TRANSPARENT);
        assert_eq!(
            surface(&rt, "settings-appearance-reset").0,
            Color::TRANSPARENT
        );
        assert_eq!(surface(&rt, "settings-close").0, Color::TRANSPARENT);
        assert_eq!(surface(&rt, "settings-shortcuts").0, Color::TRANSPARENT);
        assert_ne!(surface(&rt, "settings-appearance").0, Color::TRANSPARENT);
        assert_ne!(surface(&rt, "settings-font-0").0, Color::TRANSPARENT);
        click(&mut rt, "settings-shortcuts");
        for key in [
            "shortcut-record-terminal.new",
            "shortcut-clear-terminal.new",
            "shortcut-reset-terminal.new",
        ] {
            assert_eq!(surface(&rt, key), soft);
        }
    }
}
