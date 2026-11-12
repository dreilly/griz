use super::*;
use rune::{Chord, Key, KeyEvent, Mods, prefs::Prefs};

fn ctrl(c: char) -> KeyEvent {
    KeyEvent {
        key: Key::Command(c),
        mods: Mods {
            ctrl: true,
            ..Mods::default()
        },
    }
}

fn press(rt: &mut Runtime<State>, event: impl Into<KeyEvent>) {
    rt.key(event.into());
    rt.frame(0.0);
}

fn prefix(rt: &mut Runtime<State>) {
    press(rt, ctrl('b'));
    assert!(rt.state.prefix_waiting);
}

fn no_input(sent: &[(u64, Command)]) {
    assert!(!sent.iter().any(|(_, c)| matches!(c, Command::Input { .. })));
}

#[test]
fn prefix_waits_visibly_cancels_and_never_leaks_unknown_suffixes() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    commands(&outgoing);
    prefix(&mut rt);
    assert!(rt.elements().iter().any(|e| {
        e.text
            .as_deref()
            .is_some_and(|t| t.contains("Prefix: waiting"))
    }));
    no_input(&commands(&outgoing));
    for key in [Key::Escape, Key::Text("?".into())] {
        press(&mut rt, key);
        assert!(!rt.state.prefix_waiting);
        no_input(&commands(&outgoing));
        prefix(&mut rt);
    }
    press(&mut rt, Key::Escape);
    press(&mut rt, Key::Text("x".into()));
    assert!(
        commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::Input { data, .. } if data == b"x"))
    );
}

#[test]
fn pressing_prefix_twice_passes_one_control_byte_to_terminal() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    commands(&outgoing);
    prefix(&mut rt);
    press(&mut rt, ctrl('b'));
    assert!(!rt.state.prefix_waiting);
    let sent = commands(&outgoing);
    assert_eq!(
        sent.iter()
            .filter(|(_, c)| matches!(c, Command::Input { .. }))
            .count(),
        1
    );
    assert!(
        sent.iter()
            .any(|(_, c)| matches!(c, Command::Input { data, .. } if data == &[2]))
    );
}

#[test]
fn shifted_split_symbols_dispatch_both_directions_without_input() {
    for (symbol, direction) in [
        ("\"", SplitDirection::Horizontal),
        ("%", SplitDirection::Vertical),
    ] {
        let (state, outgoing) = mock();
        let mut rt = make_runtime(state, (1100.0, 720.0));
        commands(&outgoing);
        prefix(&mut rt);
        press(
            &mut rt,
            KeyEvent {
                key: Key::Text(symbol.into()),
                mods: Mods {
                    shift: true,
                    ..Mods::default()
                },
            },
        );
        let sent = commands(&outgoing);
        assert!(sent.iter().any(|(_, c)| matches!(c, Command::SplitTerminal { terminal_id: 1, direction: d, .. } if *d == direction)));
        no_input(&sent);
    }
}

#[test]
fn workspace_and_tab_bindings_use_display_order_and_do_not_leak() {
    for (suffix, workspace) in [
        ("p", Some(2)),
        ("n", Some(2)),
        ("1", Some(1)),
        ("2", Some(2)),
        ("9", None),
        ("<", None),
        (">", None),
    ] {
        let (state, outgoing) = mock();
        let mut rt = make_runtime(state, (1100.0, 720.0));
        commands(&outgoing);
        prefix(&mut rt);
        press(
            &mut rt,
            KeyEvent {
                key: Key::Text(suffix.into()),
                mods: Mods {
                    shift: suffix == "<" || suffix == ">",
                    ..Mods::default()
                },
            },
        );
        let sent = commands(&outgoing);
        if let Some(id) = workspace {
            assert!(sent.iter().any(|(_, c)| matches!(c, Command::SelectWorkspace { workspace_id } if *workspace_id == id)), "{suffix}");
        } else if suffix == "<" || suffix == ">" {
            assert!(
                sent.iter()
                    .any(|(_, c)| matches!(c, Command::SelectTerminal { terminal_id: 2 })),
                "{suffix}"
            );
        } else {
            assert!(
                !sent.iter().any(|(_, c)| matches!(
                    c,
                    Command::SelectTerminal { .. } | Command::SelectWorkspace { .. }
                )),
                "{sent:?}"
            );
        }
        no_input(&sent);
    }
}

#[test]
fn vim_focus_is_directional_with_no_wrap_and_works_while_zoomed() {
    for (direction, toward, edge) in [
        (SplitDirection::Vertical, "l", "h"),
        (SplitDirection::Horizontal, "j", "k"),
    ] {
        for zoomed in [false, true] {
            let (mut rt, outgoing) = splits::split_runtime(direction);
            if zoomed {
                rt.update(|s, _| s.toggle_pane_zoom(1));
                rt.frame(0.0);
            }
            commands(&outgoing);
            prefix(&mut rt);
            press(&mut rt, Key::Text(edge.into()));
            let sent = commands(&outgoing);
            no_input(&sent);
            assert!(
                !sent
                    .iter()
                    .any(|(_, c)| matches!(c, Command::SelectTerminal { .. })),
                "{sent:?}"
            );
            prefix(&mut rt);
            press(&mut rt, Key::Text(toward.into()));
            let sent = commands(&outgoing);
            assert!(
                sent.iter()
                    .any(|(_, c)| matches!(c, Command::SelectTerminal { terminal_id: 2 }))
            );
            no_input(&sent);
        }
    }
}

#[test]
fn zoom_prefix_toggles_without_changing_the_existing_shortcut() {
    let (mut rt, outgoing) = splits::split_runtime(SplitDirection::Vertical);
    commands(&outgoing);
    for expected in [true, false] {
        prefix(&mut rt);
        press(&mut rt, Key::Text("z".into()));
        assert_eq!(rt.state.pane_zoomed(), expected);
        no_input(&commands(&outgoing));
    }
    assert_eq!(
        rt.state.settings.chord(Action::TogglePaneZoom),
        Action::TogglePaneZoom.default_chord()
    );
}

#[test]
fn prefix_and_suffix_repeats_never_dispatch_twice_or_type() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    commands(&outgoing);
    prefix(&mut rt);
    rt.repeat_key(ctrl('b'));
    rt.frame(0.0);
    assert!(rt.state.prefix_waiting);
    press(&mut rt, Key::Text("z".into()));
    rt.repeat_key(Key::Text("z".into()).into());
    rt.frame(0.0);
    no_input(&commands(&outgoing));
    press(&mut rt, Key::Text("x".into()));
    assert!(
        commands(&outgoing)
            .iter()
            .any(|(_, c)| matches!(c, Command::Input { data, .. } if data == b"x"))
    );
}

#[test]
fn prefix_recording_is_independent_searchable_and_persistent() {
    let mut rt = runtime((1100.0, 720.0));
    open_settings(&mut rt);
    click(&mut rt, "settings-shortcuts");
    click(&mut rt, "prefix-key-record");
    press(&mut rt, ctrl('a'));
    assert_eq!(
        rt.state.settings.prefix(),
        Some(Chord::parse("ctrl+a").unwrap())
    );
    click(&mut rt, "settings-shortcut-search");
    press(&mut rt, Key::Text("split horizontally".into()));
    click(&mut rt, "prefix-record-pane.split_horizontal");
    press(
        &mut rt,
        KeyEvent {
            key: Key::Text("&".into()),
            mods: Mods {
                shift: true,
                ..Mods::default()
            },
        },
    );
    assert!(rt.state.prefix_recording.is_none());
    assert_eq!(
        rt.state.settings.prefix_chord(Action::SplitHorizontal),
        Some(Chord::parse("&").unwrap())
    );
    assert_eq!(
        rt.state.settings.chord(Action::SplitHorizontal),
        Action::SplitHorizontal.default_chord()
    );
    let (loaded, warning) = Settings::from_store(&rt.state.prefs);
    assert!(warning.is_none());
    assert_eq!(loaded.prefix(), rt.state.settings.prefix());
    assert_eq!(
        loaded.prefix_chord(Action::SplitHorizontal),
        rt.state.settings.prefix_chord(Action::SplitHorizontal)
    );
    click(&mut rt, "prefix-clear-pane.split_horizontal");
    assert!(
        rt.state
            .settings
            .prefix_chord(Action::SplitHorizontal)
            .is_none()
    );
    click(&mut rt, "prefix-reset-pane.split_horizontal");
    assert_eq!(
        rt.state.settings.prefix_chord(Action::SplitHorizontal),
        Action::SplitHorizontal.default_prefix_chord()
    );
    click(&mut rt, "prefix-key-clear");
    assert!(rt.state.settings.prefix().is_none());
    assert!(Settings::from_store(&rt.state.prefs).0.prefix().is_none());
    click(&mut rt, "prefix-key-reset");
    assert_eq!(
        rt.state.settings.prefix(),
        Some(Chord::parse("ctrl+b").unwrap())
    );
    click(&mut rt, "prefix-key-record");
    press(&mut rt, Key::Escape);
    assert!(rt.state.prefix_recording.is_none());
    assert!(rt.state.settings_open);
}

#[test]
fn prefix_conflicts_are_rejected_but_bare_suffixes_are_allowed() {
    let mut settings = Settings::default();
    assert!(
        settings
            .bind(Action::ToggleSidebar, Some(Chord::parse("ctrl+b").unwrap()))
            .is_err()
    );
    assert!(
        settings
            .set_prefix(Some(Chord::parse("mod+t").unwrap()))
            .is_err()
    );
    assert!(
        settings
            .set_prefix(Some(Chord::parse("mod+equal").unwrap()))
            .is_err()
    );
    assert!(
        settings
            .set_prefix(Some(Chord::parse("b").unwrap()))
            .is_err()
    );
    assert!(
        settings
            .bind_prefix(Action::ToggleSidebar, Some(Chord::parse("z").unwrap()))
            .is_err()
    );
    assert!(
        settings
            .bind_prefix(Action::ToggleSidebar, Some(Chord::parse("escape").unwrap()))
            .is_err()
    );
    settings
        .bind_prefix(Action::ToggleSidebar, Some(Chord::parse("s").unwrap()))
        .unwrap();
}

#[test]
fn prefix_settings_load_swaps_disabled_defaults_and_legacy_control_b_safely() {
    let mut settings = Settings::default();
    settings.bind(Action::ToggleSidebar, None).unwrap();
    settings
        .set_prefix(Some(Chord::parse("mod+b").unwrap()))
        .unwrap();
    settings.bind_prefix(Action::NextWorkspace, None).unwrap();
    settings
        .bind_prefix(Action::PreviousWorkspace, Some(Chord::parse("n").unwrap()))
        .unwrap();
    settings
        .bind_prefix(Action::NextWorkspace, Some(Chord::parse("p").unwrap()))
        .unwrap();
    let mut prefs = Prefs::memory();
    settings.save(&mut prefs).unwrap();
    let (loaded, warning) = Settings::from_store(&prefs);
    assert!(warning.is_none());
    assert_eq!(loaded.prefix(), settings.prefix());
    assert_eq!(
        loaded.prefix_chord(Action::NextWorkspace),
        settings.prefix_chord(Action::NextWorkspace)
    );
    prefs
        .set(
            "settings",
            &serde_json::json!({"keybindings": {"sidebar.toggle": "ctrl+b"}}),
        )
        .unwrap();
    let (loaded, warning) = Settings::from_store(&prefs);
    assert!(warning.is_some());
    assert!(loaded.prefix().is_none());
    assert_eq!(
        loaded.chord(Action::ToggleSidebar),
        Some(Chord::parse("ctrl+b").unwrap())
    );
}

#[test]
fn prefix_controls_sit_to_the_right_and_fit_narrow_windows() {
    for (width, font_size) in [(1100.0, 14.0), (640.0, 14.0), (640.0, 28.0)] {
        let mut state = State::default();
        state.settings.font_size = font_size;
        state.settings_open = true;
        state.settings_page = settings_ui::Page::Shortcuts;
        let mut rt = make_runtime(state, (width, 720.0));
        rt.state.shortcut_search = TextEdit::new("split horizontally");
        rt.frame(0.0);
        let direct = rt
            .presented("shortcut-record-pane.split_horizontal")
            .unwrap()
            .0;
        let prefixed = rt
            .presented("prefix-record-pane.split_horizontal")
            .unwrap()
            .0;
        if width == 1100.0 {
            assert!(
                prefixed[0] >= direct[0] + direct[2],
                "{direct:?} {prefixed:?}"
            );
            assert!(
                (prefixed[1] - direct[1]).abs() < 1.0,
                "{direct:?} {prefixed:?}"
            );
        }
        for key in [
            "prefix-key-record",
            "prefix-key-reset",
            "shortcut-record-pane.split_horizontal",
            "prefix-record-pane.split_horizontal",
            "prefix-reset-pane.split_horizontal",
        ] {
            rt.update(|_, tx| {
                tx.scroll_into_view(key);
                tx.instant();
            });
            rt.frame(0.0);
            let [x, _, w, _] = rt.presented(key).unwrap().0;
            assert!(
                x >= 0.0 && x + w <= width,
                "{key} at {x} + {w}, window {width}, font {font_size}"
            );
        }
    }
}

#[test]
fn directional_focus_follows_nested_tree_in_all_four_directions() {
    use griz_protocol::PaneLayout;
    let leaf = |terminal_id| Box::new(PaneLayout::Leaf { terminal_id });
    let mut view = splits::split_snapshot(SplitDirection::Vertical);
    view.workspaces[0].tabs[0].layout = PaneLayout::Split {
        direction: SplitDirection::Vertical,
        first: Box::new(PaneLayout::Split {
            direction: SplitDirection::Horizontal,
            first: leaf(1),
            second: leaf(2),
        }),
        second: Box::new(PaneLayout::Split {
            direction: SplitDirection::Horizontal,
            first: leaf(4),
            second: leaf(5),
        }),
    };
    let (mut state, _) = mock();
    state.apply(view.clone());
    for (current, direction, expected) in [
        (1, Action::FocusPaneRight, Some(4)),
        (1, Action::FocusPaneDown, Some(2)),
        (1, Action::FocusPaneUp, None),
        (1, Action::FocusPaneLeft, None),
        (5, Action::FocusPaneLeft, Some(2)),
        (5, Action::FocusPaneUp, Some(4)),
        (5, Action::FocusPaneRight, None),
        (5, Action::FocusPaneDown, None),
    ] {
        view.active_terminal = Some(current);
        state.apply(view.clone());
        assert_eq!(panes::neighbor(&state, direction), expected);
    }
}

#[test]
fn prefix_does_not_activate_in_modals_and_navigation_cancels_waiting() {
    let mut rt = runtime((1100.0, 720.0));
    prefix(&mut rt);
    click(&mut rt, "workspace-2");
    assert!(!rt.state.prefix_waiting);
    open_settings(&mut rt);
    press(&mut rt, ctrl('b'));
    assert!(!rt.state.prefix_waiting);
    rt.update(|s, tx| s.close_settings(tx));
    rt.frame(0.0);
    click(&mut rt, "new-workspace");
    press(&mut rt, ctrl('b'));
    assert!(!rt.state.prefix_waiting);
}
