use super::*;
use crate::settings::Font;

fn graphic_screen() -> griz_protocol::Screen {
    let mut vt = griz_vt::Terminal::new(80, 24).unwrap();
    // The same full/half blocks and heavy vertical border used by OpenCode.
    vt.write(
        "\x1b[38;2;100;150;200m\x1b[48;2;20;30;40m█▀▄┃╹A界e\u{301}\r\n██\r\n\x1b[4m▌".as_bytes(),
    );
    vt.screen().unwrap()
}

fn children<'a>(
    elements: &'a [rune::runtime::ElementInfo],
    key: &str,
) -> Vec<&'a rune::runtime::ElementInfo> {
    let index = elements
        .iter()
        .position(|e| e.key.as_deref() == Some(key))
        .unwrap();
    elements
        .iter()
        .filter(|e| e.parent == Some(index))
        .collect()
}

#[test]
fn terminal_graphics_fill_cells_at_every_font_and_size_without_changing_text() {
    for font in Font::ALL {
        for font_size in [10.0, 14.0, 28.0] {
            let (mut state, _) = mock();
            state.settings.font = font;
            state.settings.font_size = font_size;
            state
                .snapshot
                .as_mut()
                .unwrap()
                .terminal
                .as_mut()
                .unwrap()
                .screen = graphic_screen();
            let rt = make_runtime(state, (1400.0, 900.0));
            let elements = rt.elements();
            let graphic = |key: &str| {
                elements
                    .iter()
                    .find(|e| e.key.as_deref() == Some(key))
                    .unwrap()
            };
            let full = graphic("terminal-graphic-0-0");
            let (cw, ch) = terminal_view::cell_size(&rt.state.settings);
            assert_eq!([full.rect[2], full.rect[3]], [cw, ch]);
            assert_eq!(
                children(&elements, "terminal-graphic-0-0")[0].rect,
                full.rect
            );
            let top = children(&elements, "terminal-graphic-1-0")[0];
            let bottom = children(&elements, "terminal-graphic-2-0")[0];
            assert_eq!(top.rect, [full.rect[0] + cw, full.rect[1], cw, ch / 2.0]);
            assert_eq!(
                bottom.rect,
                [
                    full.rect[0] + 2.0 * cw,
                    full.rect[1] + ch / 2.0,
                    cw,
                    ch / 2.0
                ]
            );
            assert_eq!(top.fill, terminal_view::color([100, 150, 200]));
            // Full blocks in both directions share edges, with no font leading.
            let below = children(&elements, "terminal-graphic-0-1")[0];
            let beside = children(&elements, "terminal-graphic-1-1")[0];
            assert_eq!(below.rect[1], full.rect[1] + ch);
            assert_eq!(beside.rect[0], below.rect[0] + cw);
            let border = children(&elements, "terminal-graphic-3-0");
            assert_eq!(
                border.iter().map(|e| e.rect[1]).reduce(f32::min).unwrap(),
                full.rect[1]
            );
            assert_eq!(
                border
                    .iter()
                    .map(|e| e.rect[1] + e.rect[3])
                    .reduce(f32::max)
                    .unwrap(),
                full.rect[1] + ch
            );
            let cap = children(&elements, "terminal-graphic-4-0")[0];
            assert_eq!(cap.rect[1], full.rect[1]);
            assert_eq!(cap.rect[3], ch / 2.0);
            assert!(elements.iter().any(|e| e.text.as_deref() == Some("A")));
            assert!(
                elements
                    .iter()
                    .any(|e| e.text.as_deref() == Some("界") && e.rect[2] == 2.0 * cw)
            );
            assert!(
                elements
                    .iter()
                    .any(|e| e.text.as_deref() == Some("e\u{301}"))
            );
            assert!(!elements.iter().any(|e| e.text.as_deref() == Some("█")));
            assert!(
                elements
                    .iter()
                    .any(|e| e.fill == terminal_view::color([20, 30, 40]) && e.rect[3] == ch)
            );
            assert!(rt.find("terminal-graphic-underline").is_some());
        }
    }
}

#[test]
fn terminal_graphics_scale_to_preview_cells_without_using_font_bounds() {
    let mut rt = Runtime::new(
        (graphic_screen(), Settings::default()),
        |(screen, settings), _| terminal_view::preview(1, screen, settings, 240.0, 120.0),
        (240.0, 120.0),
        Env::new(true, 2.0),
    );
    rt.frame(0.0);
    let elements = rt.elements();
    let grid = elements
        .iter()
        .find(|e| e.key.as_deref() == Some("preview-screen-grid-1"))
        .unwrap();
    let full = children(&elements, "terminal-graphic-0-0")[0];
    let top = children(&elements, "terminal-graphic-1-0")[0];
    let bottom = children(&elements, "terminal-graphic-2-0")[0];
    // Rune quantizes layout coordinates; compare cell sizes, not accumulated error.
    assert!((full.rect[2] - grid.rect[2] / 80.0).abs() < 0.01);
    assert!((full.rect[3] - grid.rect[3] / 24.0).abs() < 0.01);
    assert!((top.rect[3] - full.rect[3] / 2.0).abs() < 0.01);
    assert!((bottom.rect[1] - full.rect[1] - full.rect[3] / 2.0).abs() < 0.01);
    assert!(rt.find("terminal-cursor").is_none());
}
