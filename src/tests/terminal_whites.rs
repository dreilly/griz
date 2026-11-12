use super::*;
use crate::settings::TerminalWhites;
use crate::terminal_view::{color, foreground_for};

#[test]
fn whites_compress_highlights_without_clipping_or_dimming_other_inks() {
    let mut settings = Settings::default();
    assert_eq!(settings.terminal_whites, TerminalWhites::Soft);
    for whites in TerminalWhites::ALL {
        settings.terminal_whites = whites;
        assert_eq!(
            foreground_for(&settings, [255; 3]),
            color([whites.white_point(); 3])
        );
        let mut previous = 0.0;
        for value in 0..=255 {
            let actual = foreground_for(&settings, [value; 3]);
            assert!(
                actual.r >= previous,
                "{whites:?}: gray ramp must be monotonic"
            );
            assert!(actual.r <= color([value; 3]).r);
            previous = actual.r;
            if value <= 160 || whites == TerminalWhites::Original {
                assert_eq!(actual, color([value; 3]));
            }
        }
        for ink in [
            [255, 255, 0],
            [230, 220, 210],
            [100, 150, 200],
            [255, 254, 255],
        ] {
            assert_eq!(foreground_for(&settings, ink), color(ink));
        }
    }
    settings.terminal_whites = TerminalWhites::Soft;
    assert!(foreground_for(&settings, [220; 3]).r < foreground_for(&settings, [255; 3]).r);
}

fn screen() -> griz_protocol::Screen {
    let mut vt = griz_vt::Terminal::new(20, 2).unwrap();
    vt.write(b"\x1b[4;38;2;255;255;255;48;2;255;255;255mA");
    vt.write("█".as_bytes());
    vt.write(b"\x1b[0;38;2;144;144;144mB\x1b[38;2;255;255;0mC");
    vt.screen().unwrap()
}

fn assert_inks(elements: &[rune::runtime::ElementInfo], white: u8) {
    let text_ink = |text: &str| {
        elements
            .iter()
            .find(|e| e.text.as_deref() == Some(text))
            .unwrap()
            .text_color
    };
    assert_eq!(text_ink("A"), Some(color([white; 3])));
    assert_eq!(text_ink("B"), Some(color([144; 3])));
    assert_eq!(text_ink("C"), Some(color([255, 255, 0])));
    let graphic = elements
        .iter()
        .position(|e| e.key.as_deref() == Some("terminal-graphic-1-0"))
        .unwrap();
    assert!(
        elements
            .iter()
            .any(|e| e.parent == Some(graphic) && e.fill == color([white; 3]))
    );
    assert_eq!(
        elements
            .iter()
            .find(|e| e.key.as_deref() == Some("terminal-graphic-underline"))
            .unwrap()
            .fill,
        color([white; 3])
    );
    // Even explicit white backgrounds remain untouched.
    assert!(elements.iter().any(|e| e.fill == color([255; 3])));
}

#[test]
fn live_terminal_and_navigation_preview_share_white_policy_without_mutating_the_screen() {
    for whites in TerminalWhites::ALL {
        let (mut state, _) = mock();
        state.settings.terminal_whites = whites;
        state
            .snapshot
            .as_mut()
            .unwrap()
            .terminal
            .as_mut()
            .unwrap()
            .screen = screen();
        let rt = make_runtime(state, (1100.0, 720.0));
        assert_inks(&rt.elements(), whites.white_point());
        assert_eq!(
            rt.state
                .snapshot
                .as_ref()
                .unwrap()
                .terminal
                .as_ref()
                .unwrap()
                .screen
                .cells[0][0]
                .fg,
            [255; 3]
        );

        let mut settings = Settings::default();
        settings.terminal_whites = whites;
        let mut preview = Runtime::new(
            (screen(), settings),
            |(screen, settings), _| terminal_view::preview(1, screen, settings, 240.0, 120.0),
            (240.0, 120.0),
            Env::new(true, 2.0),
        );
        preview.frame(0.0);
        assert_inks(&preview.elements(), whites.white_point());
    }
}

#[test]
fn whites_control_is_live_persistent_keyboard_accessible_and_resettable() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    open_settings(&mut rt);
    commands(&outgoing);
    let grid = rt.state.grid;
    for whites in TerminalWhites::ALL {
        rt.update(|_, tx| {
            tx.scroll_into_view("settings-terminal-whites");
            tx.instant();
        });
        rt.frame(0.0);
        click(&mut rt, &format!("settings-whites-{}", whites.name()));
        assert_eq!(rt.state.settings.terminal_whites, whites);
        assert_eq!(
            Settings::from_store(&rt.state.prefs).0.terminal_whites,
            whites
        );
        let elements = rt.elements();
        assert_eq!(
            elements
                .iter()
                .find(|e| e.key.as_deref() == Some("settings-preview-white"))
                .unwrap()
                .text_color,
            Some(color([whites.white_point(); 3]))
        );
        assert_eq!(
            elements
                .iter()
                .find(|e| e.key.as_deref() == Some("settings-preview-dim"))
                .unwrap()
                .text_color,
            Some(color([144; 3]))
        );
    }
    click(&mut rt, "settings-whites-Original");
    rt.key(rune::Key::Right.into());
    command_palette_tests::settle(&mut rt);
    assert_eq!(rt.state.settings.terminal_whites, TerminalWhites::Soft);
    click(&mut rt, "settings-whites-Softer");
    rt.update(|_, tx| {
        tx.scroll_into_view("settings-appearance-reset");
        tx.instant();
    });
    rt.frame(0.0);
    click(&mut rt, "settings-appearance-reset");
    assert_eq!(rt.state.settings.terminal_whites, TerminalWhites::Soft);
    assert_eq!(
        Settings::from_store(&rt.state.prefs).0.terminal_whites,
        TerminalWhites::Soft
    );
    assert_eq!(rt.state.grid, grid);
    assert!(commands(&outgoing).is_empty());
}

#[test]
fn whites_preferences_default_and_reject_invalid_values_without_losing_other_settings() {
    let mut prefs = rune::prefs::Prefs::memory();
    let (settings, warning) = Settings::from_store(&prefs);
    assert!(warning.is_none());
    assert_eq!(settings.terminal_whites, TerminalWhites::Soft);
    for value in [
        serde_json::json!("unknown"),
        serde_json::json!(42),
        serde_json::Value::Null,
    ] {
        prefs
            .set(
                "settings",
                &serde_json::json!({"terminal_whites": value, "font_size": 18}),
            )
            .unwrap();
        let (settings, warning) = Settings::from_store(&prefs);
        assert!(warning.is_some());
        assert_eq!(settings.terminal_whites, TerminalWhites::Soft);
        assert_eq!(settings.font_size, 18.0);
    }
}
