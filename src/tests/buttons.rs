use super::*;
use crate::settings::{Theme, WorkspaceButtonStyle};
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
fn regular_buttons_share_the_soft_sample_in_both_themes_and_at_large_font_sizes() {
    for theme in Theme::ALL {
        for font_size in [14.0, 28.0] {
            let (mut state, _) = mock();
            state.settings.theme = theme;
            state.settings.font_size = font_size;
            assert_eq!(
                state.settings.workspace_button_style,
                WorkspaceButtonStyle::Soft
            );
            let mut rt = make_runtime(state, (1100.0, 720.0));
            click(&mut rt, "open-settings");
            click(&mut rt, "settings-buttons");
            let soft = surface(&rt, "button-style-soft");
            assert_ne!(soft.0, Color::TRANSPARENT);
            assert_eq!(soft.1, 6.0);
            for key in [
                "toggle-sidebar",
                "open-settings",
                "new-workspace",
                "new-terminal",
                "reconnect",
                "settings-close",
                "settings-buttons-reset",
                "settings-appearance",
                "settings-shortcuts",
            ] {
                assert_eq!(surface(&rt, key), soft, "{theme:?}, {font_size}pt: {key}");
            }
        }
    }
}

#[test]
fn dialogs_context_menus_and_settings_actions_use_soft_without_losing_selected_states() {
    for theme in Theme::ALL {
        let (mut state, _) = mock();
        state.settings.theme = theme;
        let mut rt = make_runtime(state, (1100.0, 720.0));
        let soft = surface(&rt, "open-settings");
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
        click(&mut rt, "open-settings");
        assert_eq!(surface(&rt, "settings-font-1"), soft);
        assert_eq!(surface(&rt, "settings-appearance-reset"), soft);
        let inactive_theme = if theme == Theme::Dark {
            "settings-theme-1"
        } else {
            "settings-theme-0"
        };
        assert_eq!(surface(&rt, inactive_theme), soft);
        assert_ne!(
            surface(&rt, "settings-font-0").0,
            soft.0,
            "the selected font stays distinct"
        );
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

#[test]
fn empty_state_style_overrides_do_not_change_the_shared_default_for_regular_actions() {
    let (mut state, _) = mock();
    state.settings.workspace_button_style = WorkspaceButtonStyle::Accent;
    let mut rt = make_runtime(state, (1100.0, 720.0));
    click(&mut rt, "open-settings");
    click(&mut rt, "settings-buttons");
    assert_eq!(
        surface(&rt, "open-settings"),
        surface(&rt, "button-style-soft")
    );
    assert_ne!(
        surface(&rt, "open-settings").0,
        surface(&rt, "button-style-accent").0
    );
}
