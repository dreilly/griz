//! A deliberately small cell renderer. The authoritative screen lives in griz-core.
use crate::{
    State,
    settings::{Settings, Theme},
};
use griz_protocol::{Cell, CursorShape, Screen, TerminalId};
use rune::{Color, Cursor, El, PointerButton, TextStyle, el, text};

mod decorations;
mod graphics;

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

/// Soften only bright neutral foregrounds, including explicit ANSI/truecolor whites.
/// Keep dim grays, chromatic colors, and all backgrounds unchanged. Compress rather
/// than clamp highlights to retain tonal variation in the bright gray ramp.
pub fn foreground_for(settings: &Settings, [r, g, b]: [u8; 3]) -> Color {
    const KNEE: u8 = 160;
    if r == g && g == b && r > KNEE {
        let white = u16::from(settings.terminal_whites.white_point());
        let knee = u16::from(KNEE);
        let range = u16::from(255 - KNEE);
        let value = knee + (u16::from(r - KNEE) * (white - knee) + range / 2) / range;
        color([value as u8; 3])
    } else {
        color([r, g, b])
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
        .cursor(
            if screen.is_some_and(|s| s.mouse_tracking && s.scrollback_offset == 0) {
                Cursor::Default
            } else {
                Cursor::Text
            },
        )
        .pointer_buttons([PointerButton::Middle])
        .on_drag(move |state: &mut State, drag, tx| {
            if let Some(id) = id {
                crate::terminal_mouse::drag(state, id, drag, tx);
            }
        })
        .on_pointer_move_local(move |state: &mut State, point, tx| {
            if let Some(id) = id {
                crate::terminal_mouse::motion(state, id, point.x, point.y, tx);
            }
        })
        .on_secondary_press_local(move |state: &mut State, point, tx| {
            if let Some(id) = id {
                crate::terminal_mouse::secondary(state, id, point.x, point.y, tx);
            }
        })
        .on_wheel(move |state: &mut State, wheel, tx| {
            id.is_some_and(|id| crate::terminal_mouse::wheel(state, id, wheel, tx))
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
                .color(foreground_for(&state.settings, [255; 3]))
                .absolute(PAD, PAD),
        );
    }
    if let Some(selection) = state
        .terminal_mouse
        .selection
        .as_ref()
        .filter(|s| Some(s.id) == id)
        && let Some(screen) = screen
    {
        for row in 0..usize::from(screen.rows) {
            if let Some((start, end)) = selection.columns(row) {
                viewport = viewport.child(
                    el().key(format!("terminal-selection-{row}"))
                        .size((end - start) as f32 * cell_width, cell_height)
                        .fill(color([115, 170, 235]).alpha(0.32))
                        .absolute(
                            PAD + start as f32 * cell_width,
                            PAD + row as f32 * cell_height,
                        )
                        .inert(),
                );
            }
        }
    }
    if active && !state.preedit.is_empty() && screen.is_none_or(|s| s.scrollback_offset == 0) {
        let (x, y) = screen.map(|s| (s.cursor_x, s.cursor_y)).unwrap_or_default();
        viewport = viewport.child(
            text(state.preedit.clone())
                .font(font(&state.settings, cell_height, false, false))
                .color(foreground_for(&state.settings, [255; 3]))
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

const FAINT_ALPHA: f32 = 0.5;

fn blink(ink: El) -> El {
    ink.cycle(rune::Cycle {
        period: 1.0,
        phase: 0.0,
        stops: &[(0.0, 1.0), (0.499, 1.0), (0.5, 0.0), (1.0, 0.0)],
        still: 1.0,
        fps: 2.0,
    })
}

/// Use the same glyph/graphic/decorations path for normal cells and block-cursor contrast ink.
/// Cursor contrast deliberately ignores faint, but cell blink still applies to its redraw.
fn cell_ink(
    cell: &Cell,
    settings: &Settings,
    width: f32,
    height: f32,
    foreground: Color,
    underline: Color,
    honor_faint: bool,
) -> Option<(El, bool)> {
    if cell.concealed {
        return None;
    }
    let faint = if honor_faint && cell.faint {
        FAINT_ALPHA
    } else {
        1.0
    };
    let foreground = foreground.alpha(foreground.a * faint);
    let underline = underline.alpha(underline.a * faint);
    let (ink, graphic) =
        if let Some(graphic) = graphics::cell(&cell.text, width, height, foreground) {
            (graphic, true)
        } else {
            (
                text(cell.text.clone())
                    .font(font(settings, height, cell.bold, cell.italic))
                    .color(foreground)
                    .size(width, height),
                false,
            )
        };
    let ink = decorations::add(
        ink,
        cell,
        decorations::Paint {
            width,
            height,
            font_size: settings.font_size,
            foreground,
            underline,
            graphic,
        },
    );
    Some((if cell.blink { blink(ink) } else { ink }, graphic))
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
            let decorated = !cell.underline.is_none() || cell.strikethrough || cell.overline;
            if cell.width == 0 || cell.concealed || cell.text.trim().is_empty() && !decorated {
                continue;
            }
            let width = f32::from(cell.width) * cell_width;
            let foreground = foreground_for(settings, cell.fg);
            let underline = foreground_for(settings, cell.underline_color);
            let Some((glyph, graphic)) = cell_ink(
                cell,
                settings,
                width,
                cell_height,
                foreground,
                underline,
                true,
            ) else {
                continue;
            };
            // Retain graphic keys for inspection without adding identity to every glyph.
            let glyph = if graphic {
                glyph.key(format!("terminal-graphic-{x}-{y}"))
            } else {
                glyph
            };
            grid = grid.child(glyph.absolute(x as f32 * cell_width, y as f32 * cell_height));
        }
    }
    if active
        && screen.cursor_visible
        && screen.cursor_x < screen.cols
        && screen.cursor_y < screen.rows
    {
        let row = screen.cells.get(usize::from(screen.cursor_y));
        let mut x = usize::from(screen.cursor_x);
        // A cursor on the continuation of a wide grapheme covers the whole glyph.
        if x > 0 && row.and_then(|r| r.get(x)).is_some_and(|c| c.width == 0) {
            x -= 1;
        }
        let cell = row.and_then(|r| r.get(x));
        let width = f32::from(cell.map_or(1, |c| c.width.max(1)))
            .min(f32::from(screen.cols) - x as f32)
            * cell_width;
        let rgb = screen.cursor_color.unwrap_or([200, 210, 220]);
        let fill = color(rgb);
        let thickness = 2.0_f32.min(cell_width).min(cell_height);
        let (w, h, top) = match screen.cursor_shape {
            CursorShape::Bar => (thickness, cell_height, 0.0),
            CursorShape::Underline => (width, thickness, cell_height - thickness),
            CursorShape::Block | CursorShape::HollowBlock => (width, cell_height, 0.0),
        };
        let mut cursor = el()
            .key("terminal-cursor")
            .ime_cursor()
            .clip_soft(0.0)
            .size(w, h)
            .absolute(
                x as f32 * cell_width,
                f32::from(screen.cursor_y) * cell_height + top,
            );
        cursor = if screen.cursor_shape == CursorShape::HollowBlock {
            cursor.border(thickness.min(1.0), fill)
        } else {
            cursor.fill(fill)
        };
        if screen.cursor_shape == CursorShape::Block
            && let Some(cell) = cell
        {
            // Choose black/white by linear luminance so even OSC 12 dark cursor
            // colors keep the character legible. Do not soften or faint this contrast ink.
            let luminance = 0.2126 * fill.r + 0.7152 * fill.g + 0.0722 * fill.b;
            let ink = color(if luminance > 0.179 { [0; 3] } else { [255; 3] });
            if let Some((cell_ink, _)) =
                cell_ink(cell, settings, width, cell_height, ink, ink, false)
            {
                cursor = cursor.child(cell_ink.key("terminal-cursor-ink").absolute(0.0, 0.0));
            }
        }
        if screen.cursor_blinking {
            // Renderer-owned, bounded to two frames/second; no IPC or whole-view
            // timer. Reduce Motion holds it visible and hidden windows stop waking.
            cursor = blink(cursor);
        }
        grid = grid.child(cursor);
    }
    grid
}
