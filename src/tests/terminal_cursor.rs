use super::*;
use crate::terminal_view::color;
use griz_protocol::CursorShape;
use rune::runtime::ElementInfo;

fn cursor_runtime(output: &str) -> Runtime<State> {
    let (mut state, _) = mock();
    let mut vt = griz_vt::Terminal::new(80, 24).unwrap();
    vt.write(output.as_bytes());
    state
        .snapshot
        .as_mut()
        .unwrap()
        .terminal
        .as_mut()
        .unwrap()
        .screen = vt.screen().unwrap();
    make_runtime(state, (1100.0, 720.0))
}

fn element(rt: &Runtime<State>, key: &str) -> ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .unwrap()
}

#[test]
fn requested_cursor_shapes_have_cell_aligned_geometry() {
    for (parameter, shape) in [
        (2, CursorShape::Block),
        (4, CursorShape::Underline),
        (6, CursorShape::Bar),
    ] {
        let rt = cursor_runtime(&format!("X\x1b[H\x1b[{parameter} q"));
        let (cw, ch) = terminal_view::cell_size(&rt.state.settings);
        let grid = element(&rt, "screen-grid");
        let cursor = element(&rt, "terminal-cursor");
        let (w, h, top) = match shape {
            CursorShape::Block => (cw, ch, 0.0),
            CursorShape::Underline => (cw, 2.0, ch - 2.0),
            CursorShape::Bar => (2.0, ch, 0.0),
            _ => unreachable!(),
        };
        assert_eq!(cursor.rect, [grid.rect[0], grid.rect[1] + top, w, h]);
        assert_eq!(cursor.fill, color([200, 210, 220]));
        assert!(cursor.loop_phase.is_none());
        assert_eq!(
            rt.find("terminal-cursor-ink").is_some(),
            shape == CursorShape::Block
        );
    }
}

#[test]
fn hollow_cursor_keeps_the_character_visible_without_an_opaque_fill() {
    let mut rt = cursor_runtime("X\x1b[H\x1b[2 q");
    rt.update(|s, tx| {
        s.snapshot
            .as_mut()
            .unwrap()
            .terminal
            .as_mut()
            .unwrap()
            .screen
            .cursor_shape = CursorShape::HollowBlock;
        tx.instant();
    });
    rt.frame(0.0);
    let (cw, ch) = terminal_view::cell_size(&rt.state.settings);
    let cursor = element(&rt, "terminal-cursor");
    assert_eq!([cursor.rect[2], cursor.rect[3]], [cw, ch]);
    assert_eq!(cursor.fill.a, 0.0);
    assert!(rt.find("terminal-cursor-ink").is_none());
    assert!(rt.elements().iter().any(|e| e.text.as_deref() == Some("X")));
}

#[test]
fn block_cursor_redraws_contrasting_text_without_mutating_cell_styles() {
    for (rgb, ink) in [("#eeeeee", [0; 3]), ("#123456", [255; 3])] {
        let rt = cursor_runtime(&format!(
            "\x1b[1;3;4;38;2;100;150;200mX\x1b[H\x1b[2 q\x1b]12;{rgb}\x07"
        ));
        let cursor = element(&rt, "terminal-cursor");
        let glyph = element(&rt, "terminal-cursor-ink");
        assert_eq!(glyph.text.as_deref(), Some("X"));
        assert_eq!(glyph.rect, cursor.rect);
        assert_eq!(glyph.text_color, Some(color(ink)));
        let cell = &rt.state.screen().unwrap().cells[0][0];
        assert!(cell.bold && cell.italic);
        assert_eq!(cell.underline, griz_protocol::UnderlineStyle::Single);
        assert_eq!(cell.fg, [100, 150, 200]);
    }
}

#[test]
fn block_cursor_uses_the_cell_graphics_path_and_preserves_underlining() {
    let rt = cursor_runtime("\x1b[4m█\x1b[H\x1b[2 q");
    let elements = rt.elements();
    let index = elements
        .iter()
        .position(|e| e.key.as_deref() == Some("terminal-cursor-ink"))
        .unwrap();
    let graphic = &elements[index];
    assert!(graphic.text.is_none());
    let children: Vec<_> = elements
        .iter()
        .filter(|e| e.parent == Some(index))
        .collect();
    assert!(
        children
            .iter()
            .any(|e| e.rect == graphic.rect && e.fill == color([0; 3]))
    );
    assert!(
        children
            .iter()
            .any(|e| e.key.as_deref() == Some("terminal-graphic-underline"))
    );
}

#[test]
fn wide_grapheme_and_its_continuation_share_a_full_width_cursor() {
    for column in [2, 3] {
        let rt = cursor_runtime(&format!("A界e\u{301}\x1b[1;{column}H\x1b[2 q"));
        let (cw, ch) = terminal_view::cell_size(&rt.state.settings);
        let grid = element(&rt, "screen-grid");
        let cursor = element(&rt, "terminal-cursor");
        assert_eq!(cursor.rect, [grid.rect[0] + cw, grid.rect[1], 2.0 * cw, ch]);
        let ink = element(&rt, "terminal-cursor-ink");
        assert_eq!(ink.rect, cursor.rect);
        assert_eq!(ink.text.as_deref(), Some("界"));
    }
    let rt = cursor_runtime("e\u{301}\x1b[H\x1b[2 q");
    assert_eq!(
        element(&rt, "terminal-cursor-ink").text.as_deref(),
        Some("e\u{301}")
    );
}

#[test]
fn cursor_style_only_frames_update_geometry_and_color() {
    let mut rt = cursor_runtime("X\x1b[H\x1b[2 q");
    let mut vt = griz_vt::Terminal::new(80, 24).unwrap();
    vt.write(b"X\x1b[H\x1b[2 q");
    for (sequence, width, rgb) in [
        ("\x1b[6 q\x1b]12;#123456\x07", 2.0, [0x12, 0x34, 0x56]),
        (
            "\x1b[2 q\x1b]112\x07",
            terminal_view::cell_size(&rt.state.settings).0,
            [200, 210, 220],
        ),
    ] {
        vt.write(sequence.as_bytes());
        let screen = vt.screen().unwrap();
        rt.update(|s, tx| {
            let mut next = s.snapshot.clone().unwrap();
            next.terminal.as_mut().unwrap().screen = screen;
            // Also cover service/client JSON transport of the new cursor fields.
            s.apply(serde_json::from_value(serde_json::to_value(next).unwrap()).unwrap());
            tx.instant();
        });
        rt.frame(0.0);
        assert_eq!(element(&rt, "terminal-cursor").rect[2], width);
        assert_eq!(element(&rt, "terminal-cursor").fill, color(rgb));
    }
}

#[test]
fn hidden_and_out_of_bounds_cursors_are_not_drawn() {
    let rt = cursor_runtime("X\x1b[H\x1b[?25l");
    assert!(rt.find("terminal-cursor").is_none());
    for (x, y) in [(80, 0), (0, 24)] {
        let mut rt = cursor_runtime("X\x1b[H\x1b[2 q");
        rt.update(|s, tx| {
            let screen = &mut s
                .snapshot
                .as_mut()
                .unwrap()
                .terminal
                .as_mut()
                .unwrap()
                .screen;
            screen.cursor_x = x;
            screen.cursor_y = y;
            tx.instant();
        });
        rt.frame(0.0);
        assert!(rt.find("terminal-cursor").is_none());
    }
}

#[test]
fn blinking_cursor_and_ink_blink_together_and_reduce_motion_holds_visible() {
    let mut rt = cursor_runtime("X\x1b[H\x1b[1 q");
    assert!(element(&rt, "terminal-cursor").loop_phase.is_some());
    assert_eq!(element(&rt, "terminal-cursor").opacity, 1.0);
    rt.frame(0.5);
    assert_eq!(element(&rt, "terminal-cursor").opacity, 0.0);
    assert_eq!(element(&rt, "terminal-cursor-ink").opacity, 0.0);
    rt.frame(0.5);
    assert_eq!(element(&rt, "terminal-cursor").opacity, 1.0);
    rt.set_system_reduced_motion(true);
    rt.frame(0.5);
    assert_eq!(element(&rt, "terminal-cursor").opacity, 1.0);
    rt.frame(1.0);
    assert_eq!(element(&rt, "terminal-cursor").opacity, 1.0);
}
