use super::*;
use griz_protocol::{MouseAction, MouseButton, MouseInput};
use rune::{Mods, PhysicalInput};

struct Clipboard(rune::clipboard::MemoryClipboard);
impl Clipboard {
    fn text(&self) -> String {
        self.0
            .text()
            .map(|text| text.to_string())
            .unwrap_or_default()
    }
    fn set(&self, text: &str) {
        self.0.set_text(text);
    }
}

fn setup(text: &str, tracking: bool) -> (Runtime<State>, Receiver<Request>, Clipboard) {
    let (mut state, outgoing) = mock();
    let mut vt = griz_vt::Terminal::new(80, 24).unwrap();
    vt.write(text.as_bytes());
    if tracking {
        vt.write(b"\x1b[?1003h\x1b[?1006h");
    }
    let mut view = snapshot();
    view.terminal.as_mut().unwrap().screen = vt.screen().unwrap();
    state.apply(view);
    let mut rt = make_runtime(state, (1100.0, 720.0));
    let clipboard = rune::clipboard::memory();
    rt.use_clipboard(&clipboard);
    rt.frame(0.0);
    commands(&outgoing);
    (rt, outgoing, Clipboard(clipboard))
}

fn point(rt: &Runtime<State>, key: &str, col: f32, row: f32) -> (f32, f32) {
    let ([x, y, _, _], _) = rt.presented(key).unwrap();
    let (cw, ch) = terminal_view::cell_size(&rt.state.settings);
    (
        x + terminal_view::PAD + col * cw,
        y + terminal_view::PAD + row * ch,
    )
}

fn select(rt: &mut Runtime<State>, from: (f32, f32), to: (f32, f32)) {
    rt.pointer_down(from.0, from.1);
    rt.pointer_move(to.0, to.1);
    rt.frame(0.0);
    rt.pointer_up(to.0, to.1);
    rt.frame(0.0);
}

fn mouse_commands(outgoing: &Receiver<Request>) -> Vec<(TerminalId, MouseInput)> {
    commands(outgoing)
        .into_iter()
        .filter_map(|(_, c)| match c {
            Command::Mouse { terminal_id, event } => Some((terminal_id, event)),
            _ => None,
        })
        .collect()
}

#[test]
fn dragging_highlights_and_copies_only_on_release_with_a_bottom_toast() {
    let (mut rt, outgoing, clipboard) = setup("hello world", false);
    clipboard.set("untouched");
    let start = point(&rt, "terminal", 0.0, 0.5);
    let end = point(&rt, "terminal", 5.0, 0.5);
    rt.pointer_down(start.0, start.1);
    rt.pointer_move(end.0, end.1);
    rt.frame(0.0);
    assert_eq!(clipboard.text(), "untouched");
    assert!(rt.find("terminal-selection-0").is_some());
    rt.pointer_up(end.0, end.1);
    rt.frame(0.0);
    assert_eq!(clipboard.text(), "hello");
    assert!(
        commands(&outgoing).is_empty(),
        "selection is local, never shell input"
    );
    let ([_, y, _, h], _) = rt.presented("clipboard-toast").unwrap();
    assert!(y > 500.0 && y + h < 720.0);
    rt.update(|s, _| s.terminal_mouse.copied_until = Some(std::time::Instant::now()));
    rt.update(|s, tx| {
        assert!(terminal_mouse::poll(s, tx));
    });
    rt.frame(0.0);
    assert!(rt.find("clipboard-toast").is_none());
}

#[test]
fn single_click_does_not_copy_and_reverse_multiline_selection_preserves_unicode() {
    let (mut rt, outgoing, clipboard) = setup("A界e\u{301}  \r\nsecond", false);
    clipboard.set("old");
    let start = point(&rt, "terminal", 0.0, 0.5);
    rt.pointer_down(start.0, start.1);
    rt.pointer_up(start.0, start.1);
    rt.frame(0.0);
    assert_eq!(clipboard.text(), "old");
    assert!(rt.find("clipboard-toast").is_none());
    let end = point(&rt, "terminal", 6.0, 1.5);
    select(&mut rt, end, start);
    assert_eq!(clipboard.text(), "A界e\u{301}\nsecond");
    assert!(rt.find("terminal-selection-1").is_some());
    assert!(commands(&outgoing).is_empty());
}

#[test]
fn selection_in_a_wide_continuation_copies_the_whole_grapheme_once() {
    let (mut rt, _, clipboard) = setup("A界B", false);
    let start = point(&rt, "terminal", 2.0, 0.5);
    let end = point(&rt, "terminal", 3.0, 0.5);
    select(&mut rt, start, end);
    assert_eq!(clipboard.text(), "界");
    let ([_, _, width, _], _) = rt.presented("terminal-selection-0").unwrap();
    assert_eq!(width, 2.0 * terminal_view::cell_size(&rt.state.settings).0);
}

#[test]
fn double_click_copies_a_word_and_triple_click_copies_a_line() {
    let (mut rt, _, clipboard) = setup("hello world", false);
    let p = point(&rt, "terminal", 7.0, 0.5);
    for expected in ["", "world", "hello world"] {
        rt.pointer_down(p.0, p.1);
        rt.pointer_up(p.0, p.1);
        rt.frame(0.0);
        assert_eq!(clipboard.text(), expected);
    }
}

#[test]
fn applications_receive_press_motion_release_and_shift_overrides_tracking_for_selection() {
    let (mut rt, outgoing, clipboard) = setup("hello world", true);
    let start = point(&rt, "terminal", 0.0, 0.5);
    let end = point(&rt, "terminal", 5.0, 0.5);
    select(&mut rt, start, end);
    let sent = mouse_commands(&outgoing);
    assert_eq!(sent.first().unwrap().1.action, MouseAction::Press);
    assert_eq!(sent.last().unwrap().1.action, MouseAction::Release);
    assert!(
        sent.iter()
            .any(|(_, e)| e.action == MouseAction::Motion && e.button == Some(MouseButton::Left))
    );
    assert!(sent.iter().all(|(id, _)| *id == 1));
    assert_eq!(sent[0].1.x, 0.0);
    assert_eq!(
        sent[0].1.y,
        terminal_view::cell_size(&rt.state.settings).1 * 0.5
    );
    assert_eq!(clipboard.text(), "");
    assert!(rt.find("clipboard-toast").is_none());
    rt.physical_input(PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    select(&mut rt, start, end);
    assert_eq!(clipboard.text(), "hello");
    assert!(mouse_commands(&outgoing).is_empty());
}

#[test]
fn hover_and_scroll_reach_mouse_apps_without_changing_focus_or_copying() {
    let (mut rt, outgoing, clipboard) = setup("hello", true);
    let p = point(&rt, "terminal", 2.0, 1.5);
    rt.pointer_move(p.0, p.1);
    rt.frame(0.0);
    let sent = mouse_commands(&outgoing);
    assert!(
        sent.iter()
            .any(|(_, e)| e.action == MouseAction::Motion && e.button.is_none())
    );
    rt.wheel_event(rune::WheelEvent::new(0.0, 40.0).at(p.0, p.1));
    rt.frame(0.0);
    let sent = mouse_commands(&outgoing);
    assert!(
        sent.iter()
            .any(|(_, e)| e.button == Some(MouseButton::WheelUp))
    );
    rt.wheel_event(rune::WheelEvent::new(0.0, -40.0).at(p.0, p.1));
    rt.frame(0.0);
    assert!(
        mouse_commands(&outgoing)
            .iter()
            .any(|(_, e)| e.button == Some(MouseButton::WheelDown))
    );
    assert_eq!(clipboard.text(), "");
    assert_eq!(rt.state.active_terminal(), Some(1));
}

#[test]
fn selection_is_cleared_by_output_resize_typing_escape_and_navigation() {
    for operation in 0..5 {
        let (mut rt, outgoing, clipboard) = setup("hello world", false);
        let start = point(&rt, "terminal", 0.0, 0.5);
        let end = point(&rt, "terminal", 5.0, 0.5);
        select(&mut rt, start, end);
        match operation {
            0 | 1 => rt.update(|s, _| {
                let mut view = s.snapshot.clone().unwrap();
                let screen = &mut view.terminal.as_mut().unwrap().screen;
                if operation == 0 {
                    screen.cells[0][0].text = "X".into();
                } else {
                    screen.cols -= 1;
                }
                s.apply(view);
            }),
            2 => rt.key(rune::Key::Text("x".into()).into()),
            3 => rt.key(rune::Key::Escape.into()),
            _ => rt.update(|s, _| s.change(Command::SelectTerminal { terminal_id: 2 })),
        }
        rt.frame(0.0);
        assert!(
            rt.find("terminal-selection-0").is_none(),
            "operation {operation}"
        );
        assert_eq!(clipboard.text(), "hello");
        if operation == 3 {
            assert!(
                commands(&outgoing).is_empty(),
                "selection Escape isn't shell input"
            );
        }
    }
}

#[test]
fn stale_frames_cannot_turn_an_in_progress_selection_into_different_clipboard_text() {
    let (mut rt, _, clipboard) = setup("hello", false);
    clipboard.set("old");
    let start = point(&rt, "terminal", 0.0, 0.5);
    let end = point(&rt, "terminal", 5.0, 0.5);
    rt.pointer_down(start.0, start.1);
    rt.pointer_move(end.0, end.1);
    rt.frame(0.0);
    rt.update(|s, _| {
        let mut view = s.snapshot.clone().unwrap();
        view.terminal.as_mut().unwrap().screen.cells[0][0].text = "X".into();
        s.apply(view);
    });
    rt.pointer_up(end.0, end.1);
    rt.frame(0.0);
    assert_eq!(clipboard.text(), "old");
}

#[test]
fn cancellation_never_copies_but_releases_the_applications_button() {
    for tracking in [false, true] {
        let (mut rt, outgoing, clipboard) = setup("hello", tracking);
        clipboard.set("old");
        let start = point(&rt, "terminal", 0.0, 0.5);
        let end = point(&rt, "terminal", 5.0, 0.5);
        rt.pointer_down(start.0, start.1);
        rt.pointer_move(end.0, end.1);
        rt.frame(0.0);
        mouse_commands(&outgoing);
        rt.physical_input(PhysicalInput::Key {
            key: rune::PhysicalKeyCode::Escape,
            mods: Mods::default(),
            pressed: true,
            repeat: false,
        });
        rt.frame(0.0);
        assert_eq!(clipboard.text(), "old");
        assert!(rt.find("terminal-selection-0").is_none());
        let sent = mouse_commands(&outgoing);
        if tracking {
            assert_eq!(sent.last().unwrap().1.action, MouseAction::Release);
        } else {
            assert!(sent.is_empty());
        }
    }
}

#[test]
fn clicks_in_an_inactive_split_reach_only_that_pane_even_before_focus_acknowledgement() {
    let (mut rt, outgoing, _) = setup("hello", true);
    rt.update(|s, _| {
        let mut view = s.snapshot.clone().unwrap();
        let first = view.terminal.clone().unwrap();
        let mut second = first.clone();
        second.id = 2;
        second.shell_pid = 102;
        view.terminals = vec![first, second];
        view.workspaces[0].tabs = vec![TabInfo {
            id: 1,
            layout: griz_protocol::PaneLayout::Split {
                direction: SplitDirection::Vertical,
                first: Box::new(griz_protocol::PaneLayout::Leaf { terminal_id: 1 }),
                second: Box::new(griz_protocol::PaneLayout::Leaf { terminal_id: 2 }),
            },
        }];
        s.apply(view);
    });
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
    commands(&outgoing);
    let p = point(&rt, "terminal-viewport-2", 3.0, 0.5);
    rt.pointer_down(p.0, p.1);
    rt.frame(0.0);
    assert!(rt.state.pending.is_some());
    rt.pointer_up(p.0, p.1);
    rt.frame(0.0);
    let sent = commands(&outgoing);
    assert!(matches!(
        sent[0].1,
        Command::SelectTerminal { terminal_id: 2 }
    ));
    let mouse: Vec<_> = sent
        .iter()
        .filter_map(|(_, c)| match c {
            Command::Mouse { terminal_id, event } => Some((*terminal_id, event.action)),
            _ => None,
        })
        .collect();
    assert_eq!(
        mouse,
        vec![(2, MouseAction::Press), (2, MouseAction::Release)]
    );
}

#[test]
fn right_and_middle_clicks_are_reported_without_copying() {
    let (mut rt, outgoing, clipboard) = setup("hello", true);
    let p = point(&rt, "terminal", 2.0, 0.5);
    rt.secondary_down(p.0, p.1);
    rt.secondary_up(p.0, p.1);
    rt.frame(0.0);
    let sent = mouse_commands(&outgoing);
    assert_eq!(sent.len(), 2);
    assert!(
        sent.iter()
            .all(|(_, e)| e.button == Some(MouseButton::Right))
    );
    rt.button_down(p.0, p.1, rune::PointerButton::Middle);
    rt.button_up(p.0, p.1, rune::PointerButton::Middle);
    rt.frame(0.0);
    let sent = mouse_commands(&outgoing);
    assert_eq!(sent.len(), 2);
    assert!(
        sent.iter()
            .all(|(_, e)| e.button == Some(MouseButton::Middle))
    );
    assert_eq!(clipboard.text(), "");
}

#[test]
fn explicitly_selected_spaces_are_copied_and_blank_lines_never_send_ctrl_c() {
    let (mut rt, outgoing, clipboard) = setup("   hello", false);
    let start = point(&rt, "terminal", 0.0, 0.5);
    let end = point(&rt, "terminal", 3.0, 0.5);
    select(&mut rt, start, end);
    assert_eq!(clipboard.text(), "   ");
    let blank = point(&rt, "terminal", 2.0, 2.5);
    for _ in 0..3 {
        rt.pointer_down(blank.0, blank.1);
        rt.pointer_up(blank.0, blank.1);
        rt.frame(0.0);
    }
    rt.key(rune::KeyEvent {
        key: rune::Key::Text("c".into()),
        mods: Mods {
            ctrl: true,
            shift: true,
            ..Mods::default()
        },
    });
    rt.frame(0.0);
    assert!(commands(&outgoing).is_empty());
}

#[test]
fn shift_scroll_discards_previous_fractional_mouse_wheel_delta() {
    let (mut rt, outgoing, _) = setup("hello", true);
    let p = point(&rt, "terminal", 2.0, 0.5);
    let delta = terminal_view::cell_size(&rt.state.settings).1 * 0.75;
    rt.wheel_event(rune::WheelEvent::new(0.0, delta).at(p.0, p.1));
    rt.frame(0.0);
    assert!(mouse_commands(&outgoing).is_empty());
    rt.wheel_event(rune::WheelEvent::new(0.0, delta).at(p.0, p.1).mods(Mods {
        shift: true,
        ..Mods::default()
    }));
    rt.frame(0.0);
    rt.wheel_event(rune::WheelEvent::new(0.0, delta).at(p.0, p.1));
    rt.frame(0.0);
    assert!(mouse_commands(&outgoing).is_empty());
}

#[test]
fn copy_shortcut_recopies_the_selection_without_reaching_the_shell() {
    let (mut rt, outgoing, clipboard) = setup("hello", false);
    let start = point(&rt, "terminal", 0.0, 0.5);
    let end = point(&rt, "terminal", 5.0, 0.5);
    select(&mut rt, start, end);
    clipboard.set("other app");
    rt.key(rune::KeyEvent {
        key: rune::Key::Text("c".into()),
        mods: Mods {
            ctrl: true,
            shift: true,
            ..Mods::default()
        },
    });
    rt.frame(0.0);
    assert_eq!(clipboard.text(), "hello");
    assert!(commands(&outgoing).is_empty());
}

#[test]
fn disconnected_and_exited_terminal_text_can_still_be_selected() {
    for exited in [false, true] {
        let (mut rt, outgoing, clipboard) = setup("hello", true);
        rt.update(|s, _| {
            if exited {
                s.snapshot
                    .as_mut()
                    .unwrap()
                    .terminal
                    .as_mut()
                    .unwrap()
                    .exited = Some("0".into());
            } else {
                s.online = false;
            }
        });
        let start = point(&rt, "terminal", 0.0, 0.5);
        let end = point(&rt, "terminal", 5.0, 0.5);
        select(&mut rt, start, end);
        assert_eq!(clipboard.text(), "hello");
        assert!(mouse_commands(&outgoing).is_empty());
    }
}
