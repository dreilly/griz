use super::*;
use crate::terminal_view::{color, foreground_for};
use rune::runtime::ElementInfo;

fn sgr_runtime(output: &str, font_size: f32) -> Runtime<State> {
    let (mut state, _) = mock();
    state.settings.font_size = font_size;
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

fn element<'a>(elements: &'a [ElementInfo], key: &str) -> &'a ElementInfo {
    elements
        .iter()
        .find(|element| element.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("missing {key}"))
}

fn element_index(elements: &[ElementInfo], key: &str) -> usize {
    elements
        .iter()
        .position(|element| element.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("missing {key}"))
}

fn direct_child<'a>(elements: &'a [ElementInfo], parent: usize, key: &str) -> &'a ElementInfo {
    elements
        .iter()
        .find(|element| element.parent == Some(parent) && element.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("missing {key} under element {parent}"))
}

#[test]
fn renders_faint_strike_overline_and_every_ghostty_underline_variant() {
    let rt = sgr_runtime(
        "\x1b[2mA\x1b[0;9mB\x1b[0;53mC\x1b[0;4:1;58;2;1;2;3mD\
         \x1b[0;4:2mE\x1b[0;4:3mF\x1b[0;4:4mG\x1b[0;4:5mH\x1b[?25l",
        14.0,
    );
    let elements = rt.elements();
    let (cw, ch) = terminal_view::cell_size(&rt.state.settings);
    let faint = elements
        .iter()
        .find(|element| element.text.as_deref() == Some("A"))
        .unwrap();
    assert_eq!(faint.text_color.unwrap().a, 0.5);
    assert_eq!(element(&elements, "terminal-strikethrough").rect[2], cw);
    assert_eq!(element(&elements, "terminal-overline").rect[2], cw);

    let single = element(&elements, "terminal-underline-single");
    assert_eq!(single.rect[2], cw);
    assert_eq!(single.fill, color([1, 2, 3]));
    let double = element(&elements, "terminal-underline-double");
    assert_eq!([double.rect[2], double.rect[3]], [cw, ch]);
    for key in [
        "terminal-underline-curly",
        "terminal-underline-dotted",
        "terminal-underline-dashed",
    ] {
        assert_eq!(element(&elements, key).rect[2], cw);
    }
    assert_eq!(
        elements
            .iter()
            .filter(|element| element.key.as_deref() == Some("terminal-underline-single"))
            .count(),
        1,
        "the straight underline must not also use Rune's font underline"
    );
}

#[test]
fn combined_styles_dim_only_ink_while_conceal_suppresses_every_rendering_path() {
    let mut rt = sgr_runtime(
        "\x1b[2;5;4;9;53;38;2;255;255;255;48;2;10;20;30;58;2;255;255;255mX\
         \x1b[0;2;5;8;4:3;9;53mA█\x1b[0mB\x1b[1;2H\x1b[2 q",
        14.0,
    );
    let screen = rt.state.screen().unwrap().clone();
    assert_eq!(screen.cells[0][1].text, "A");
    assert_eq!(screen.cells[0][2].text, "█");
    let elements = rt.elements();
    let x = elements
        .iter()
        .find(|element| element.text.as_deref() == Some("X"))
        .unwrap();
    let expected = foreground_for(&rt.state.settings, [255; 3]).alpha(0.5);
    assert_eq!(x.text_color, Some(expected));
    assert!(x.loop_phase.is_some());
    for key in [
        "terminal-underline-single",
        "terminal-strikethrough",
        "terminal-overline",
    ] {
        assert_eq!(element(&elements, key).fill.a, 0.5);
    }
    assert!(
        elements
            .iter()
            .any(|element| { element.fill == color([10, 20, 30]) && element.opacity == 1.0 })
    );
    assert!(
        !elements
            .iter()
            .any(|element| element.text.as_deref() == Some("A"))
    );
    assert!(rt.find("terminal-graphic-2-0").is_none());
    assert!(rt.find("terminal-underline-curly").is_none());
    assert!(rt.find("terminal-cursor").is_some());
    assert!(rt.find("terminal-cursor-ink").is_none());

    let mut preview = Runtime::new(
        (screen, rt.state.settings.clone()),
        |(screen, settings), _| terminal_view::preview(1, screen, settings, 240.0, 120.0),
        (240.0, 120.0),
        Env::new(true, 2.0),
    );
    preview.frame(0.0);
    assert!(
        !preview
            .elements()
            .iter()
            .any(|element| element.text.as_deref() == Some("A"))
    );
    assert!(preview.find("terminal-graphic-2-0").is_none());

    rt.frame(0.5);
    assert_eq!(
        rt.elements()
            .iter()
            .find(|element| element.text.as_deref() == Some("X"))
            .unwrap()
            .opacity,
        0.0
    );
    assert!(
        !rt.elements()
            .iter()
            .any(|element| element.text.as_deref() == Some("A"))
    );
}

#[test]
fn wide_glyph_graphics_and_block_cursor_share_cell_decoration_geometry() {
    let rt = sgr_runtime("\x1b[2;4:2;9;53m界█\x1b[1;2H\x1b[2 q", 14.0);
    let elements = rt.elements();
    let (cw, ch) = terminal_view::cell_size(&rt.state.settings);
    let normal_wide = elements
        .iter()
        .find(|element| {
            element.text.as_deref() == Some("界")
                && element.key.as_deref() != Some("terminal-cursor-ink")
        })
        .unwrap();
    assert_eq!([normal_wide.rect[2], normal_wide.rect[3]], [2.0 * cw, ch]);
    assert_eq!(normal_wide.text_color.unwrap().a, 0.5);

    let graphic_index = element_index(&elements, "terminal-graphic-2-0");
    assert_eq!(elements[graphic_index].rect[2], cw);
    for key in [
        "terminal-underline-double",
        "terminal-strikethrough",
        "terminal-overline",
    ] {
        assert_eq!(direct_child(&elements, graphic_index, key).rect[2], cw);
    }

    let cursor_index = element_index(&elements, "terminal-cursor-ink");
    assert_eq!(
        [
            elements[cursor_index].rect[2],
            elements[cursor_index].rect[3]
        ],
        [2.0 * cw, ch]
    );
    assert_eq!(elements[cursor_index].text.as_deref(), Some("界"));
    assert_eq!(elements[cursor_index].text_color.unwrap().a, 1.0);
    for key in [
        "terminal-underline-double",
        "terminal-strikethrough",
        "terminal-overline",
    ] {
        let decoration = direct_child(&elements, cursor_index, key);
        assert_eq!(decoration.rect[2], 2.0 * cw);
        assert_eq!(decoration.opacity, 1.0);
    }
}

#[test]
fn cell_blink_is_renderer_owned_keeps_background_steady_and_honors_reduce_motion() {
    let mut rt = sgr_runtime("\x1b[5;4;48;2;20;30;40mX\x1b[0;5;8mY\x1b[?25l", 14.0);
    let background = rt
        .elements()
        .iter()
        .find(|element| element.fill == color([20, 30, 40]))
        .unwrap()
        .rect;
    let text_opacity = |rt: &Runtime<State>| {
        rt.elements()
            .iter()
            .find(|element| element.text.as_deref() == Some("X"))
            .unwrap()
            .opacity
    };
    assert_eq!(text_opacity(&rt), 1.0);
    assert!(
        rt.elements()
            .iter()
            .find(|element| element.text.as_deref() == Some("X"))
            .unwrap()
            .loop_phase
            .is_some()
    );
    rt.frame(0.5);
    assert_eq!(text_opacity(&rt), 0.0);
    let elements = rt.elements();
    let steady = elements
        .iter()
        .find(|element| element.rect == background && element.fill == color([20, 30, 40]))
        .unwrap();
    assert_eq!(steady.opacity, 1.0);
    assert_eq!(element(&elements, "terminal-underline-single").opacity, 0.0);
    assert!(
        !rt.elements()
            .iter()
            .any(|element| element.text.as_deref() == Some("Y"))
    );

    rt.set_system_reduced_motion(true);
    rt.frame(0.5);
    assert_eq!(text_opacity(&rt), 1.0);
    rt.frame(1.0);
    assert_eq!(text_opacity(&rt), 1.0);
}

#[test]
fn decoration_thickness_and_wide_width_scale_with_font_size() {
    let mut previous = 0.0;
    for font_size in [10.0, 14.0, 28.0] {
        let rt = sgr_runtime("\x1b[4m界█\x1b[?25l", font_size);
        let elements = rt.elements();
        let (cw, _) = terminal_view::cell_size(&rt.state.settings);
        let underline = element(&elements, "terminal-underline-single");
        let graphic = element(&elements, "terminal-graphic-underline");
        assert_eq!(underline.rect[2], 2.0 * cw);
        assert_eq!(graphic.rect[2], cw);
        // Rune lands paint rectangles on the 2× device grid.
        assert!((underline.rect[3] - font_size / 14.0).abs() <= 0.25);
        assert_eq!(graphic.rect[3], underline.rect[3]);
        assert!(underline.rect[3] > previous);
        previous = underline.rect[3];
    }
}
