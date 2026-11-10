//! Cell-sized terminal graphics, independent of font ink bounds and line leading.
//! Unsupported symbols (and graphemes with combining marks) stay on the text path.
use rune::{Color, El, el};

/// Rectangles use cell-relative coordinates, so the same geometry fits previews.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    coverage: f32,
}

impl Rect {
    fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            coverage: 1.0,
        }
    }
}

fn blocks(ch: char) -> Option<Vec<Rect>> {
    let code = ch as u32;
    let rect = match ch {
        '▀' => Rect::new(0.0, 0.0, 1.0, 0.5),
        '▁'..='█' => {
            let height = (code - 0x2580) as f32 / 8.0;
            Rect::new(0.0, 1.0 - height, 1.0, height)
        }
        '▉'..='▏' => Rect::new(0.0, 0.0, (0x2590 - code) as f32 / 8.0, 1.0),
        '▐' => Rect::new(0.5, 0.0, 0.5, 1.0),
        '░'..='▓' => Rect {
            coverage: (code - 0x2590) as f32 / 4.0,
            ..Rect::new(0.0, 0.0, 1.0, 1.0)
        },
        '▔' => Rect::new(0.0, 0.0, 1.0, 0.125),
        '▕' => Rect::new(0.875, 0.0, 0.125, 1.0),
        '▖'..='▟' => {
            // Bits: upper-left, upper-right, lower-left, lower-right.
            const QUADRANTS: [u8; 10] = [4, 8, 1, 13, 9, 7, 11, 2, 6, 14];
            let mask = QUADRANTS[(code - 0x2596) as usize];
            return Some(
                (0..4)
                    .filter(|i| mask & (1 << i) != 0)
                    .map(|i| Rect::new((i % 2) as f32 * 0.5, (i / 2) as f32 * 0.5, 0.5, 0.5))
                    .collect(),
            );
        }
        _ => return None,
    };
    Some(vec![rect])
}

// Two bits per arm, from low to high: left, right, up, down.
// 0 = absent, 1 = light, 2 = heavy. Zero entries are dashed glyphs: use the font.
const SOLID_BOX: [u8; 76] = [
    0x05, 0x0a, 0x50, 0xa0, 0x00, 0x00, 0x00, 0x00, // ─━│┃┄┅┆┇
    0x00, 0x00, 0x00, 0x00, 0x44, 0x48, 0x84, 0x88, // ┈┉┊┋┌┍┎┏
    0x41, 0x42, 0x81, 0x82, 0x14, 0x18, 0x24, 0x28, // ┐┑┒┓└┕┖┗
    0x11, 0x12, 0x21, 0x22, 0x54, 0x58, 0x64, 0x94, // ┘┙┚┛├┝┞┟
    0xa4, 0x68, 0x98, 0xa8, 0x51, 0x52, 0x61, 0x91, // ┠┡┢┣┤┥┦┧
    0xa1, 0x62, 0x92, 0xa2, 0x45, 0x46, 0x49, 0x4a, // ┨┩┪┫┬┭┮┯
    0x85, 0x86, 0x89, 0x8a, 0x15, 0x16, 0x19, 0x1a, // ┰┱┲┳┴┵┶┷
    0x25, 0x26, 0x29, 0x2a, 0x55, 0x56, 0x59, 0x5a, // ┸┹┺┻┼┽┾┿
    0x65, 0x95, 0xa5, 0x66, 0x69, 0x96, 0x99, 0x6a, // ╀╁╂╃╄╅╆╇
    0x9a, 0xa6, 0xa9, 0xaa, // ╈╉╊╋
];
const BOX_ENDS: [u8; 12] = [
    0x01, 0x10, 0x04, 0x40, 0x02, 0x20, 0x08, 0x80, // ╴╵╶╷╸╹╺╻
    0x09, 0x90, 0x06, 0x60, // ╼╽╾╿
];

fn box_strokes(ch: char, width: f32, height: f32) -> Option<Vec<Rect>> {
    let code = ch as u32;
    let arms = match code {
        0x2500..=0x254b => SOLID_BOX[(code - 0x2500) as usize],
        0x2574..=0x257f => BOX_ENDS[(code - 0x2574) as usize],
        _ => return None,
    };
    if arms == 0 {
        return None;
    }
    // Thickness follows the cell width, not the much taller line height. Keeping
    // this proportional also keeps miniature strokes scaled with their grid.
    let thickness = |i: u32| f32::from((arms >> (i * 2)) & 3u8) / 8.0;
    let (left, right, up, down) = (thickness(0), thickness(1), thickness(2), thickness(3));
    let vertical_reach = left.max(right) * width / height / 2.0;
    let horizontal_reach = up.max(down) / 2.0;
    let mut rects = Vec::new();
    if left > 0.0 {
        let t = left * width / height;
        rects.push(Rect::new(0.0, 0.5 - t / 2.0, 0.5 + horizontal_reach, t));
    }
    if right > 0.0 {
        let t = right * width / height;
        rects.push(Rect::new(
            0.5 - horizontal_reach,
            0.5 - t / 2.0,
            0.5 + horizontal_reach,
            t,
        ));
    }
    if up > 0.0 {
        rects.push(Rect::new(0.5 - up / 2.0, 0.0, up, 0.5 + vertical_reach));
    }
    if down > 0.0 {
        rects.push(Rect::new(
            0.5 - down / 2.0,
            0.5 - vertical_reach,
            down,
            0.5 + vertical_reach,
        ));
    }
    Some(rects)
}

pub(super) fn cell(text: &str, width: f32, height: f32, foreground: Color) -> Option<El> {
    let mut chars = text.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let rects = blocks(ch).or_else(|| box_strokes(ch, width, height))?;
    let mut graphic = el().size(width, height);
    for rect in rects {
        let fill = Color {
            a: foreground.a * rect.coverage,
            ..foreground
        };
        graphic = graphic.child(
            el().size(rect.width * width, rect.height * height)
                .fill(fill)
                .absolute(rect.x * width, rect.y * height),
        );
    }
    Some(graphic)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covered(rects: &[Rect], x: f32, y: f32) -> bool {
        rects
            .iter()
            .any(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
    }

    #[test]
    fn blocks_match_their_unicode_cell_coverage() {
        for code in 0x2580..=0x259f {
            let ch = char::from_u32(code).unwrap();
            let rects = blocks(ch).unwrap();
            for y in 0..8 {
                for x in 0..8 {
                    let expected = match code {
                        0x2580 => y < 4,
                        0x2581..=0x2588 => y >= 8 - (code - 0x2580),
                        0x2589..=0x258f => x < 0x2590 - code,
                        0x2590 => x >= 4,
                        0x2591..=0x2593 => true, // Coverage controls shade intensity.
                        0x2594 => y == 0,
                        0x2595 => x == 7,
                        _ => {
                            let quadrants = match ch {
                                '▖' => [false, false, true, false],
                                '▗' => [false, false, false, true],
                                '▘' => [true, false, false, false],
                                '▙' => [true, false, true, true],
                                '▚' => [true, false, false, true],
                                '▛' => [true, true, true, false],
                                '▜' => [true, true, false, true],
                                '▝' => [false, true, false, false],
                                '▞' => [false, true, true, false],
                                '▟' => [false, true, true, true],
                                _ => unreachable!(),
                            };
                            quadrants[(y / 4 * 2 + x / 4) as usize]
                        }
                    };
                    assert_eq!(
                        covered(&rects, (x as f32 + 0.5) / 8.0, (y as f32 + 0.5) / 8.0),
                        expected,
                        "{ch} at {x},{y}"
                    );
                }
            }
        }
        for (ch, coverage) in [('░', 0.25), ('▒', 0.5), ('▓', 0.75)] {
            assert_eq!(blocks(ch).unwrap()[0].coverage, coverage);
        }
    }

    #[test]
    fn box_strokes_reach_cell_edges_and_join_at_the_center() {
        for (width, height) in [(9.0, 20.0), (17.0, 40.0), (2.025, 4.5)] {
            for (ch, edges) in [
                ('─', [true, true, false, false]),
                ('┃', [false, false, true, true]),
                ('┌', [false, true, false, true]),
                ('┘', [true, false, true, false]),
                ('┣', [false, true, true, true]),
                ('┼', [true, true, true, true]),
                ('╹', [false, false, true, false]),
                ('╻', [false, false, false, true]),
                ('╼', [true, true, false, false]),
            ] {
                let rects = box_strokes(ch, width, height).unwrap();
                for (i, (x, y)) in [(0.001, 0.5), (0.999, 0.5), (0.5, 0.001), (0.5, 0.999)]
                    .into_iter()
                    .enumerate()
                {
                    assert_eq!(covered(&rects, x, y), edges[i], "{ch}, edge {i}");
                }
                assert!(covered(&rects, 0.499, 0.499) || covered(&rects, 0.501, 0.501));
            }
        }
        // A mixed-weight junction keeps each arm's specified weight.
        let rects = box_strokes('┍', 8.0, 20.0).unwrap();
        assert_eq!(rects[0].height * 20.0, 2.0);
        assert_eq!(rects[1].width * 8.0, 1.0);
    }

    #[test]
    fn ordinary_text_and_combining_graphemes_keep_the_font_path() {
        for text in ["", "A", "界", "e\u{301}", "█\u{301}", "██", "┄", "╭", "═"] {
            assert!(cell(text, 9.0, 20.0, Color::WHITE).is_none(), "{text}");
        }
    }
}
