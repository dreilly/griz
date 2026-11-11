use super::*;

fn row(screen: &Screen, index: usize) -> String {
    screen.cells[index]
        .iter()
        .map(|cell| cell.text.as_str())
        .collect::<String>()
        .trim_end()
        .into()
}

fn terminal() -> Terminal {
    let mut vt = Terminal::new(12, 3).unwrap();
    vt.write(b"line0\r\nline1\r\nline2\r\nline3\r\nline4\r\nline5");
    vt
}

#[test]
fn scrollback_moves_both_directions_clamps_and_keeps_clients_independent() {
    let mut vt = terminal();
    let mut first = Viewport::default();
    let mut second = Viewport::default();
    assert_eq!(row(&vt.screen().unwrap(), 0), "line3");
    vt.scroll(&mut first, -2).unwrap();
    let screen = vt.screen_at(&first).unwrap();
    assert_eq!(row(&screen, 0), "line1");
    assert_eq!(screen.scrollback_offset, 2);
    assert!(!screen.cursor_visible);
    assert_eq!(row(&vt.screen().unwrap(), 0), "line3");
    vt.scroll(&mut second, -1).unwrap();
    assert_eq!(row(&vt.screen_at(&second).unwrap(), 0), "line2");
    assert_eq!(row(&vt.screen_at(&first).unwrap(), 0), "line1");
    vt.scroll(&mut first, i32::MIN).unwrap();
    assert_eq!(row(&vt.screen_at(&first).unwrap(), 0), "line0");
    vt.scroll(&mut first, 1).unwrap();
    assert_eq!(row(&vt.screen_at(&first).unwrap(), 0), "line1");
    vt.scroll(&mut first, i32::MAX).unwrap();
    assert_eq!(vt.screen_at(&first).unwrap().scrollback_offset, 0);
    vt.write(b"\r\nline6");
    assert_eq!(row(&vt.screen_at(&first).unwrap(), 0), "line4");
}

#[test]
fn history_stays_anchored_during_output_and_resize() {
    let mut vt = terminal();
    let mut viewport = Viewport::default();
    vt.scroll(&mut viewport, -2).unwrap();
    vt.write(b"\r\nline6\r\nline7");
    assert_eq!(row(&vt.screen_at(&viewport).unwrap(), 0), "line1");
    assert_eq!(vt.screen_at(&viewport).unwrap().scrollback_offset, 4);
    assert_eq!(row(&vt.screen().unwrap(), 0), "line5");
    vt.resize(16, 4).unwrap();
    assert_eq!(row(&vt.screen_at(&viewport).unwrap(), 0), "line1");
    assert_eq!(vt.screen().unwrap().scrollback_offset, 0);
}

#[test]
fn alternate_screen_never_shows_primary_history_and_refs_outlive_terminal() {
    let mut vt = terminal();
    let mut viewport = Viewport::default();
    vt.scroll(&mut viewport, -2).unwrap();
    vt.write(b"\x1b[?1049h\x1b[Halternate");
    assert_eq!(row(&vt.screen_at(&viewport).unwrap(), 0), "alternate");
    assert_eq!(vt.screen_at(&viewport).unwrap().scrollback_offset, 0);
    vt.write(b"\x1b[?1049l");
    assert_eq!(row(&vt.screen_at(&viewport).unwrap(), 0), "line1");
    drop(vt);
    drop(viewport);
}

#[test]
fn reset_invalidates_history_without_breaking_future_scrolling() {
    let mut vt = terminal();
    let mut viewport = Viewport::default();
    vt.scroll(&mut viewport, -2).unwrap();
    vt.write(b"\x1bclive");
    let screen = vt.screen_at(&viewport).unwrap();
    assert_eq!(screen.scrollback_offset, 0);
    assert_eq!(row(&screen, 0), "live");
    vt.scroll(&mut viewport, -10).unwrap();
    assert_eq!(vt.screen_at(&viewport).unwrap().scrollback_offset, 0);
}
