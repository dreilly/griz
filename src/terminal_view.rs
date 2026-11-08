//! A deliberately small cell renderer. The authoritative screen lives in griz-core.
use crate::State;
use griz_protocol::Screen;
use rune::{Color, El, Family, Span, TextStyle, el, text};

pub const CELL_WIDTH: f32 = 9.0;
pub const CELL_HEIGHT: f32 = 20.0;
pub const PAD: f32 = 12.0;

pub fn color([r, g, b]: [u8; 3]) -> Color {
    let decode = |v| rune::color::srgb_decode(f32::from(v) / 255.0);
    Color {
        r: decode(r),
        g: decode(g),
        b: decode(b),
        a: 1.0,
    }
}

pub fn grid(width: f32, height: f32) -> (u16, u16) {
    griz_protocol::dimensions(
        ((width - 2.0 * PAD).max(0.0) / CELL_WIDTH) as u16,
        ((height - 2.0 * PAD).max(0.0) / CELL_HEIGHT) as u16,
    )
}

pub fn viewport(state: &State) -> El {
    let mut viewport = el()
        .key("terminal")
        .role("region")
        .label("Terminal")
        .focusable(true)
        .grow(1.0)
        .w_full()
        .min_h(0.0)
        .min_w(0.0)
        .fill(Color::BLACK)
        .clip_soft(0.0)
        .instant_changes()
        .on_key(crate::terminal_key)
        .on_layout_tx(|state: &mut State, [_, _, width, height], tx| {
            let grid = grid(width, height);
            if state.grid != grid {
                state.grid = grid;
                state.resize();
            }
            tx.live_only();
        });
    if let Some(screen) = state.screen() {
        viewport = viewport.child(screen_grid(screen));
    } else {
        viewport = viewport.child(
            text(state.status.clone())
                .color(Color::WHITE)
                .absolute(PAD, PAD),
        );
    }
    if !state.preedit.is_empty() {
        let (x, y) = state
            .screen()
            .map(|s| (s.cursor_x, s.cursor_y))
            .unwrap_or_default();
        viewport = viewport.child(
            text(state.preedit.clone())
                .font(font(false, false))
                .color(Color::WHITE)
                .fill(color([40, 40, 40]))
                .absolute(
                    PAD + f32::from(x) * CELL_WIDTH,
                    PAD + f32::from(y) * CELL_HEIGHT,
                ),
        );
    }
    viewport
}

fn font(bold: bool, italic: bool) -> TextStyle {
    let style = TextStyle::new(14.0, if bold { 700.0 } else { 400.0 })
        .family(Family::Named("Menlo"))
        .lh(CELL_HEIGHT)
        .features("'liga' 0, 'calt' 0");
    if italic { style.italic() } else { style }
}

fn screen_grid(screen: &Screen) -> El {
    let mut grid = el()
        .key("screen-grid")
        .size(
            f32::from(screen.cols) * CELL_WIDTH,
            f32::from(screen.rows) * CELL_HEIGHT,
        )
        .absolute(PAD, PAD)
        .inert();
    for (y, row) in screen.cells.iter().enumerate() {
        // Coalesce adjacent backgrounds. Blank black cells need no nodes at all.
        let mut x = 0;
        while x < row.len() {
            let start = x;
            let bg = row[x].bg;
            while x < row.len() && row[x].bg == bg {
                x += 1;
            }
            if bg != [0, 0, 0] {
                grid = grid.child(
                    el().size((x - start) as f32 * CELL_WIDTH, CELL_HEIGHT)
                        .fill(color(bg))
                        .absolute(start as f32 * CELL_WIDTH, y as f32 * CELL_HEIGHT),
                );
            }
        }
        for (x, cell) in row.iter().enumerate() {
            if cell.width == 0 || cell.text.trim().is_empty() && !cell.underline {
                continue;
            }
            let mut glyph = text(cell.text.clone())
                .font(font(cell.bold, cell.italic))
                .color(color(cell.fg))
                .size(f32::from(cell.width) * CELL_WIDTH, CELL_HEIGHT)
                .absolute(x as f32 * CELL_WIDTH, y as f32 * CELL_HEIGHT);
            if cell.underline {
                glyph = glyph.spans(vec![
                    Span::new(0, cell.text.len()).underline(color(cell.fg)),
                ]);
            }
            grid = grid.child(glyph);
        }
    }
    if screen.cursor_visible && screen.cursor_x < screen.cols && screen.cursor_y < screen.rows {
        grid = grid.child(
            el().key("terminal-cursor")
                .ime_cursor()
                .size(2.0, CELL_HEIGHT)
                .fill(color([200, 210, 220]))
                .absolute(
                    f32::from(screen.cursor_x) * CELL_WIDTH,
                    f32::from(screen.cursor_y) * CELL_HEIGHT,
                ),
        );
    }
    grid
}
