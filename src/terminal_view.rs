//! A deliberately small cell renderer. The authoritative screen lives in griz-core.
use crate::{
    State,
    settings::{Settings, Theme},
};
use griz_protocol::Screen;
use rune::{Color, El, Span, TextStyle, el, text};

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

pub fn background_for(theme: Theme) -> Color {
    color(match theme {
        Theme::Dark => [24, 26, 29],
        Theme::Midnight => [18, 25, 40],
    })
}

pub fn cell_size(settings: &Settings) -> (f32, f32) {
    // All offered families are monospace. Measure the selected face rather than assuming
    // every font has Menlo's advance. Keep integral point cells to avoid cumulative drift.
    let style = TextStyle::new(settings.font_size, 400.0).family(settings.font.family());
    let width = rune::text::advance("M", style).ceil().max(1.0);
    let height = (settings.font_size * 20.0 / 14.0).ceil();
    (width, height)
}

pub fn grid_for(width: f32, height: f32, settings: &Settings) -> (u16, u16) {
    let (cell_width, cell_height) = cell_size(settings);
    griz_protocol::dimensions(
        ((width - 2.0 * PAD).max(0.0) / cell_width) as u16,
        ((height - 2.0 * PAD).max(0.0) / cell_height) as u16,
    )
}

pub fn viewport(state: &State) -> El {
    let (cell_width, cell_height) = cell_size(&state.settings);
    let mut viewport = el()
        .key("terminal")
        .role("region")
        .label("Terminal")
        .focusable(true)
        .grow(1.0)
        .w_full()
        .min_h(0.0)
        .min_w(0.0)
        .fill(background_for(state.settings.theme))
        .no_focus_ring()
        .clip_soft(0.0)
        .instant_changes()
        .on_key(crate::terminal_key)
        .on_layout_tx(|state: &mut State, [_, _, width, height], tx| {
            state.viewport_size = Some((width, height));
            let grid = grid_for(width, height, &state.settings);
            if state.grid != grid {
                state.grid = grid;
                state.resize();
            }
            tx.live_only();
        });
    if let Some(screen) = state.screen() {
        viewport = viewport.child(screen_grid(
            screen,
            &state.settings,
            cell_width,
            cell_height,
        ));
    } else {
        viewport = viewport.child(
            text(state.status.clone())
                .font(state.settings.ui_font(TextStyle::new(14.0, 400.0)))
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
                .font(font(&state.settings, cell_height, false, false))
                .color(Color::WHITE)
                .fill(color([40, 40, 40]))
                .absolute(
                    PAD + f32::from(x) * cell_width,
                    PAD + f32::from(y) * cell_height,
                ),
        );
    }
    viewport
}

fn font(settings: &Settings, height: f32, bold: bool, italic: bool) -> TextStyle {
    let style = TextStyle::new(settings.font_size, if bold { 700.0 } else { 400.0 })
        .family(settings.font.family())
        .lh(height)
        .features("'liga' 0, 'calt' 0");
    if italic { style.italic() } else { style }
}

fn screen_grid(screen: &Screen, settings: &Settings, cell_width: f32, cell_height: f32) -> El {
    let mut grid = el()
        .key("screen-grid")
        .size(
            f32::from(screen.cols) * cell_width,
            f32::from(screen.rows) * cell_height,
        )
        .absolute(PAD, PAD)
        .inert();
    for (y, row) in screen.cells.iter().enumerate() {
        // Keep the protocol's black base background transparent for the chosen theme.
        let mut x = 0;
        while x < row.len() {
            let start = x;
            let bg = row[x].bg;
            while x < row.len() && row[x].bg == bg {
                x += 1;
            }
            if bg != [0, 0, 0] {
                grid = grid.child(
                    el().size((x - start) as f32 * cell_width, cell_height)
                        .fill(color(bg))
                        .absolute(start as f32 * cell_width, y as f32 * cell_height),
                );
            }
        }
        for (x, cell) in row.iter().enumerate() {
            if cell.width == 0 || cell.text.trim().is_empty() && !cell.underline {
                continue;
            }
            let mut glyph = text(cell.text.clone())
                .font(font(settings, cell_height, cell.bold, cell.italic))
                .color(color(cell.fg))
                .size(f32::from(cell.width) * cell_width, cell_height)
                .absolute(x as f32 * cell_width, y as f32 * cell_height);
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
                .size(2.0, cell_height)
                .fill(color([200, 210, 220]))
                .absolute(
                    f32::from(screen.cursor_x) * cell_width,
                    f32::from(screen.cursor_y) * cell_height,
                ),
        );
    }
    grid
}
