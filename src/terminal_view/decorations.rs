//! Cell-relative SGR decorations. Geometry follows the terminal grid rather than font bounds.
use griz_protocol::{Cell, UnderlineStyle};
use rune::{Color, El, LineCap, Shape, el};

#[derive(Clone, Copy, Debug)]
struct Metrics {
    thickness: f32,
    underline_y: f32,
    strike_y: f32,
}

fn metrics(font_size: f32, height: f32) -> Metrics {
    let thickness = (font_size / 14.0).max(0.25).min(height / 6.0);
    Metrics {
        thickness,
        underline_y: height - thickness,
        strike_y: height * 0.52 - thickness / 2.0,
    }
}

fn line(key: &'static str, width: f32, thickness: f32, y: f32, color: Color) -> El {
    el().key(key)
        .size(width, thickness)
        .fill(color)
        .absolute(0.0, y)
}

fn patterned_underline(
    key: &'static str,
    style: UnderlineStyle,
    width: f32,
    height: f32,
    metrics: Metrics,
    color: Color,
) -> El {
    let y = height - metrics.thickness / 2.0;
    let mut shape = Shape::new();
    match style {
        UnderlineStyle::Curly => {
            let waves = (width / (metrics.thickness * 4.0).max(3.0))
                .ceil()
                .clamp(1.0, 8.0) as usize;
            let segments = waves * 4;
            let points = (0..=segments)
                .map(|i| {
                    let x = width * i as f32 / segments as f32;
                    let phase = i as f32 * std::f32::consts::FRAC_PI_2;
                    [x, y - metrics.thickness + phase.sin() * metrics.thickness]
                })
                .collect::<Vec<_>>();
            shape = shape.stroke_with(points, false, metrics.thickness, LineCap::Round, color);
        }
        UnderlineStyle::Dotted => {
            let count = (width / (metrics.thickness * 3.0).max(2.0))
                .ceil()
                .clamp(1.0, 16.0) as usize;
            let step = width / count as f32;
            for i in 0..count {
                let x = (i as f32 + 0.5) * step;
                shape = shape.line(
                    [x - 0.01, y],
                    [x + 0.01, y],
                    metrics.thickness,
                    LineCap::Round,
                    color,
                );
            }
        }
        UnderlineStyle::Dashed => {
            let count = (width / (metrics.thickness * 6.0).max(4.0))
                .ceil()
                .clamp(1.0, 8.0) as usize;
            let step = width / count as f32;
            for i in 0..count {
                let left = i as f32 * step + step * 0.12;
                let right = (i as f32 + 1.0) * step - step * 0.12;
                shape = shape.line(
                    [left, y],
                    [right, y],
                    metrics.thickness,
                    LineCap::Butt,
                    color,
                );
            }
        }
        _ => {}
    }
    el().key(key)
        .size(width, height)
        .shape(shape)
        .absolute(0.0, 0.0)
}

pub(super) struct Paint {
    pub width: f32,
    pub height: f32,
    pub font_size: f32,
    pub foreground: Color,
    pub underline: Color,
    pub graphic: bool,
}

pub(super) fn add(mut ink: El, cell: &Cell, paint: Paint) -> El {
    let Paint {
        width,
        height,
        font_size,
        foreground,
        underline,
        graphic,
    } = paint;
    let metrics = metrics(font_size, height);
    match cell.underline {
        UnderlineStyle::None => {}
        UnderlineStyle::Single => {
            ink = ink.child(line(
                if graphic {
                    "terminal-graphic-underline"
                } else {
                    "terminal-underline-single"
                },
                width,
                metrics.thickness,
                metrics.underline_y,
                underline,
            ));
        }
        UnderlineStyle::Double => {
            let mut double = el()
                .key("terminal-underline-double")
                .size(width, height)
                .absolute(0.0, 0.0);
            double = double.child(line(
                "terminal-underline-double-top",
                width,
                metrics.thickness,
                (height - 3.0 * metrics.thickness).max(0.0),
                underline,
            ));
            double = double.child(line(
                "terminal-underline-double-bottom",
                width,
                metrics.thickness,
                metrics.underline_y,
                underline,
            ));
            ink = ink.child(double);
        }
        UnderlineStyle::Curly => {
            ink = ink.child(patterned_underline(
                "terminal-underline-curly",
                cell.underline,
                width,
                height,
                metrics,
                underline,
            ));
        }
        UnderlineStyle::Dotted => {
            ink = ink.child(patterned_underline(
                "terminal-underline-dotted",
                cell.underline,
                width,
                height,
                metrics,
                underline,
            ));
        }
        UnderlineStyle::Dashed => {
            ink = ink.child(patterned_underline(
                "terminal-underline-dashed",
                cell.underline,
                width,
                height,
                metrics,
                underline,
            ));
        }
    }
    if cell.strikethrough {
        ink = ink.child(line(
            "terminal-strikethrough",
            width,
            metrics.thickness,
            metrics.strike_y,
            foreground,
        ));
    }
    if cell.overline {
        ink = ink.child(line(
            "terminal-overline",
            width,
            metrics.thickness,
            0.0,
            foreground,
        ));
    }
    ink
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoration_thickness_scales_with_font_size_and_preview_height() {
        let small = metrics(10.0, 15.0);
        let normal = metrics(14.0, 20.0);
        let large = metrics(28.0, 40.0);
        assert!(small.thickness < normal.thickness);
        assert!(normal.thickness < large.thickness);
        assert_eq!(normal.thickness, 1.0);
        assert_eq!(large.thickness, 2.0);
        assert!(small.underline_y < 15.0 && large.underline_y < 40.0);
    }
}
