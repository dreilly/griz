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

fn write_numbered_rows(vt: &mut Terminal, start: usize, end: usize, styled: bool) {
    let mut bytes = Vec::with_capacity((end - start) * 32);
    for number in start..end {
        if styled {
            bytes.extend_from_slice(
                format!(
                    "\x1b[1;38;2;{};{};{}m",
                    number % 256,
                    (number * 3) % 256,
                    (number * 7) % 256
                )
                .as_bytes(),
            );
        }
        bytes.extend_from_slice(format!("row-{number:04}\x1b[0m\r\n").as_bytes());
    }
    vt.write(&bytes);
}

fn write_wrapped_rows(vt: &mut Terminal, start: usize, end: usize) {
    let mut bytes = Vec::with_capacity((end - start) * 80);
    let fill = "x".repeat(60);
    for number in start..end {
        bytes.extend_from_slice(format!("row-{number:04}-{fill}\r\n").as_bytes());
    }
    vt.write(&bytes);
}

fn row_marker(screen: &Screen, index: usize) -> Option<usize> {
    row(screen, index)
        .strip_prefix("row-")?
        .get(..4)?
        .parse::<usize>()
        .ok()
}

fn marker_number(screen: &Screen) -> Option<usize> {
    (0..screen.rows as usize).find_map(|index| row_marker(screen, index))
}

fn align_viewport_to_marker(vt: &mut Terminal, viewport: &mut Viewport) -> usize {
    for _ in 0..3 {
        let screen = vt.screen_at(viewport).unwrap();
        if let Some(marker) = row_marker(&screen, 0) {
            return marker;
        }
        vt.scroll(viewport, 1).unwrap();
    }
    panic!("viewport did not reach a logical line start");
}

fn top_screen(vt: &mut Terminal) -> Screen {
    let mut viewport = Viewport::default();
    vt.scroll(&mut viewport, i32::MIN).unwrap();
    vt.screen_at(&viewport).unwrap()
}

fn contains_row(screen: &Screen, expected: usize) -> bool {
    (0..screen.rows as usize).any(|index| row(screen, index) == format!("row-{expected:04}"))
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
fn screen_at_preserves_soft_wrap_metadata_in_history() {
    let mut vt = Terminal::new(4, 2).unwrap();
    vt.write(b"abcdef\r\nhard\r\ntail");
    let live = vt.screen().unwrap();
    assert_eq!(row(&live, 0), "hard");
    assert_eq!(row(&live, 1), "tail");
    assert_eq!(live.row_soft_wrapped.as_slice(), &[false, false]);

    let mut viewport = Viewport::default();
    vt.scroll(&mut viewport, -2).unwrap();
    let history = vt.screen_at(&viewport).unwrap();
    assert_eq!(row(&history, 0), "abcd");
    assert_eq!(row(&history, 1), "ef");
    assert_eq!(history.row_soft_wrapped.as_slice(), &[true, false]);
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

#[test]
fn griz_default_retains_materially_more_than_ghosttys_one_page_default() {
    const GHOSTTY_DEFAULT_BYTES: usize = 10_000;
    const ROWS: usize = 3_000;

    let mut default = Terminal::new(80, 4).unwrap();
    let mut one_page = Terminal::new_with_scrollback_limits(
        80,
        4,
        ScrollbackLimits {
            max_bytes: GHOSTTY_DEFAULT_BYTES,
            max_lines: usize::MAX,
        },
    )
    .unwrap();
    write_numbered_rows(&mut default, 0, ROWS, false);
    write_numbered_rows(&mut one_page, 0, ROWS, false);

    let default_top = top_screen(&mut default);
    let one_page_top = top_screen(&mut one_page);
    assert_eq!(marker_number(&default_top), Some(0));
    assert!(
        marker_number(&one_page_top).unwrap() >= 500,
        "one-page history unexpectedly retained nearly all {ROWS} rows"
    );
    assert!(
        default_top.scrollback_offset > one_page_top.scrollback_offset + 1_000,
        "default depth {} was not materially greater than one-page depth {}",
        default_top.scrollback_offset,
        one_page_top.scrollback_offset
    );
}

#[test]
fn small_line_budget_prunes_oldest_rows_and_retains_recent_rows() {
    let mut vt = Terminal::new_with_scrollback_limits(
        120,
        4,
        ScrollbackLimits {
            max_bytes: usize::MAX,
            max_lines: 24,
        },
    )
    .unwrap();
    write_numbered_rows(&mut vt, 0, 1_500, false);

    assert!(marker_number(&top_screen(&mut vt)).unwrap() > 0);
    assert!(contains_row(&vt.screen().unwrap(), 1_499));
}

#[test]
fn small_byte_budget_prunes_style_heavy_rows_and_retains_recent_rows() {
    let mut vt = Terminal::new_with_scrollback_limits(
        96,
        5,
        ScrollbackLimits {
            max_bytes: 1,
            max_lines: usize::MAX,
        },
    )
    .unwrap();
    write_numbered_rows(&mut vt, 0, 1_500, true);

    assert!(marker_number(&top_screen(&mut vt)).unwrap() > 0);
    assert!(contains_row(&vt.screen().unwrap(), 1_499));
}

#[test]
fn zero_byte_budget_disables_and_erases_scrollback() {
    let mut vt = Terminal::new_with_scrollback_limits(
        20,
        3,
        ScrollbackLimits {
            max_bytes: 0,
            max_lines: usize::MAX,
        },
    )
    .unwrap();
    write_numbered_rows(&mut vt, 0, 30, false);

    let top = top_screen(&mut vt);
    assert_eq!(top.scrollback_offset, 0);
    assert!(contains_row(&top, 29));
}

#[test]
fn client_viewports_stay_independent_across_pruning_and_resize_reflow() {
    let mut vt = Terminal::new_with_scrollback_limits(
        40,
        4,
        ScrollbackLimits {
            max_bytes: usize::MAX,
            max_lines: 6_000,
        },
    )
    .unwrap();
    write_wrapped_rows(&mut vt, 0, 5_000);
    let oldest_before = marker_number(&top_screen(&mut vt)).unwrap();

    let mut first = Viewport::default();
    let mut second = Viewport::default();
    vt.scroll(&mut first, -1_000).unwrap();
    vt.scroll(&mut second, -200).unwrap();
    let first_anchor = align_viewport_to_marker(&mut vt, &mut first);
    let second_anchor = align_viewport_to_marker(&mut vt, &mut second);
    assert_ne!(first_anchor, second_anchor);

    write_wrapped_rows(&mut vt, 5_000, 6_000);
    let oldest_after = marker_number(&top_screen(&mut vt)).unwrap();
    assert!(oldest_after > oldest_before);
    assert_eq!(
        row_marker(&vt.screen_at(&first).unwrap(), 0),
        Some(first_anchor)
    );
    assert_eq!(
        row_marker(&vt.screen_at(&second).unwrap(), 0),
        Some(second_anchor)
    );
    assert!(vt.screen_at(&first).unwrap().scrollback_offset > 0);
    assert!(vt.screen_at(&second).unwrap().scrollback_offset > 0);

    vt.resize(80, 5).unwrap();
    assert_eq!(
        row_marker(&vt.screen_at(&first).unwrap(), 0),
        Some(first_anchor)
    );
    assert_eq!(
        row_marker(&vt.screen_at(&second).unwrap(), 0),
        Some(second_anchor)
    );
    assert_eq!(vt.screen().unwrap().scrollback_offset, 0);
}
