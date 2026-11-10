//! A deliberately small cell renderer. The authoritative screen lives in griz-core.
use crate::{
    State,
    settings::{Settings, Theme},
};
use griz_protocol::{Screen, TerminalId};
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

pub fn viewport(state: &State, id: Option<TerminalId>, split: bool) -> El {
    let (cell_width, cell_height) = cell_size(&state.settings);
    let active = id == state.active_terminal();
    let screen = id.and_then(|id| state.pane_frame(id)).map(|t| &t.screen);
    let mut viewport = el()
        .key(if split {
            format!("terminal-viewport-{}", id.unwrap_or(0))
        } else {
            "terminal".into()
        })
        .role("region")
        .label("Terminal")
        .focusable(active)
        .grow(1.0)
        .w_full()
        .min_h(0.0)
        .min_w(0.0)
        .fill(background_for(state.settings.theme))
        .no_focus_ring()
        .clip_soft(0.0)
        .instant_changes()
        .on_press_tx(move |state: &mut State, tx| {
            if let Some(id) = id {
                if state.active_terminal() == Some(id) {
                    state.acknowledge_focused_agents();
                } else {
                    state.change(crate::Command::SelectTerminal { terminal_id: id });
                }
            }
            tx.blur();
        })
        .on_key(move |state: &mut State, event, tx| {
            // Keyboard focus can outlive a catalog change. Never route a stale pane's
            // keystrokes to the newly selected terminal.
            if id != state.active_terminal() {
                return true;
            }
            crate::terminal_key(state, event, tx)
        })
        .on_layout_tx(move |state: &mut State, [_, _, width, height], tx| {
            if let Some(id) = id {
                state.layout_pane(id, width, height);
            }
            tx.live_only();
        });
    if let Some(screen) = screen {
        viewport = viewport.child(
            screen_grid(screen, &state.settings, cell_width, cell_height, active).key(if split {
                format!("screen-grid-{}", id.unwrap_or(0))
            } else {
                "screen-grid".into()
            }),
        );
    } else {
        viewport = viewport.child(
            text(state.status.clone())
                .font(state.settings.ui_font(TextStyle::new(14.0, 400.0)))
                .color(Color::WHITE)
                .absolute(PAD, PAD),
        );
    }
    if active && !state.preedit.is_empty() {
        let (x, y) = screen.map(|s| (s.cursor_x, s.cursor_y)).unwrap_or_default();
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

/// A fitted, inert miniature. This deliberately does not use `viewport`: previews
/// must never focus a terminal, register an IME cursor, or resize the underlying PTY.
pub fn preview(
    id: TerminalId,
    screen: &Screen,
    settings: &Settings,
    width: f32,
    height: f32,
) -> El {
    let (cw, ch) = cell_size(settings);
    let scale = ((width - 12.0).max(0.0) / (f32::from(screen.cols) * cw).max(1.0))
        .min((height - 12.0).max(0.0) / (f32::from(screen.rows) * ch).max(1.0))
        .min(1.0);
    let mut miniature = settings.clone();
    miniature.font_size *= scale;
    let (cw, ch) = (cw * scale, ch * scale);
    el().size(width, height)
        .fill(background_for(settings.theme))
        .clip_soft(0.0)
        .inert()
        .child(
            screen_grid(screen, &miniature, cw, ch, false)
                .key(format!("preview-screen-grid-{id}"))
                .absolute(
                    ((width - f32::from(screen.cols) * cw) / 2.0).max(0.0),
                    ((height - f32::from(screen.rows) * ch) / 2.0).max(0.0),
                ),
        )
}

fn font(settings: &Settings, height: f32, bold: bool, italic: bool) -> TextStyle {
    let style = TextStyle::new(settings.font_size, if bold { 700.0 } else { 400.0 })
        .family(settings.font.family())
        .lh(height)
        .features("'liga' 0, 'calt' 0");
    if italic { style.italic() } else { style }
}

fn screen_grid(
    screen: &Screen,
    settings: &Settings,
    cell_width: f32,
    cell_height: f32,
    active: bool,
) -> El {
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
    if active
        && screen.cursor_visible
        && screen.cursor_x < screen.cols
        && screen.cursor_y < screen.rows
    {
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
