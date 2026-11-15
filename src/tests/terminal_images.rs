use super::*;
use griz_protocol::{ImageAsset, ImageKey, ImageLayer, ImagePlacement, Screen};
use rune::render::{Gpu, OFFSCREEN_FORMAT, Offscreen, Renderer};
use rune::{Cx, El, el};
use std::{path::PathBuf, sync::Arc};

const BLACK: [u8; 4] = [0, 0, 0, 255];

#[derive(Clone)]
struct GridState {
    screen: Screen,
    settings: Settings,
    cell_width: f32,
    cell_height: f32,
    active: bool,
}

fn screen(cols: u16, rows: u16) -> Screen {
    let mut screen = griz_vt::Terminal::new(cols, rows)
        .unwrap()
        .screen()
        .unwrap();
    screen.cursor_visible = false;
    screen.images.clear();
    screen.image_assets.clear();
    screen
}

fn key(namespace: u64, id: u32, generation: u64) -> ImageKey {
    ImageKey {
        namespace,
        id,
        generation,
    }
}

fn asset(key: ImageKey, width: u32, height: u32, rgba: Vec<u8>) -> Arc<ImageAsset> {
    Arc::new(ImageAsset {
        key,
        width,
        height,
        rgba,
    })
}

// Explicit destination and source geometry keep rendering fixtures easy to audit.
#[allow(clippy::too_many_arguments)]
fn placement(
    key: ImageKey,
    layer: ImageLayer,
    z: i32,
    col: i32,
    row: i32,
    width: u32,
    height: u32,
    source: [u32; 4],
) -> ImagePlacement {
    ImagePlacement {
        key,
        layer,
        z,
        col,
        row,
        offset_x: 0,
        offset_y: 0,
        width,
        height,
        source_x: source[0],
        source_y: source[1],
        source_width: source[2],
        source_height: source[3],
    }
}

fn grid_view(state: &GridState, _: &mut Cx<GridState>) -> El {
    el().size(
        f32::from(state.screen.cols) * state.cell_width + 2.0 * terminal_view::PAD,
        f32::from(state.screen.rows) * state.cell_height + 2.0 * terminal_view::PAD,
    )
    .fill(terminal_view::color([0, 0, 0]))
    .child(terminal_view::screen_grid(
        &state.screen,
        &state.settings,
        state.cell_width,
        state.cell_height,
        state.active,
    ))
}

fn grid_runtime(state: GridState) -> Runtime<GridState> {
    let size = (
        f32::from(state.screen.cols) * state.cell_width + 2.0 * terminal_view::PAD,
        f32::from(state.screen.rows) * state.cell_height + 2.0 * terminal_view::PAD,
    );
    Runtime::new(state, grid_view, size, Env::new(true, 1.0))
}

struct Capture {
    gpu: Gpu,
    renderer: Renderer,
    target: Offscreen,
}

impl Capture {
    fn new(width: u32, height: u32) -> Self {
        let gpu = Gpu::headless();
        let renderer = Renderer::new(&gpu.device, OFFSCREEN_FORMAT);
        let target = Offscreen::new(&gpu.device, width, height);
        Self {
            gpu,
            renderer,
            target,
        }
    }

    fn draw<S: 'static>(&mut self, rt: &mut Runtime<S>) -> Vec<u8> {
        rt.frame(0.0);
        self.renderer.prepare(
            &self.gpu.device,
            &self.gpu.queue,
            &mut rt.text.atlas,
            &rt.instances,
            (self.target.width, self.target.height),
        );
        self.renderer
            .upload_images(&self.gpu.device, &self.gpu.queue, &mut rt.images);
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.renderer
            .draw(&mut encoder, &self.target.view, [0.0, 0.0, 0.0, 1.0]);
        self.target.copy(&mut encoder);
        self.gpu.queue.submit([encoder.finish()]);
        self.target.read(&self.gpu.device)
    }
}

fn pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let start = ((y * width + x) * 4) as usize;
    rgba[start..start + 4].try_into().unwrap()
}

fn near(got: [u8; 4], expected: [u8; 4]) {
    assert!(
        got.into_iter()
            .zip(expected)
            .all(|(a, b)| a.abs_diff(b) <= 3),
        "{got:?} != {expected:?}"
    );
}

fn image_key(key: ImageKey, z: i32, index: usize) -> String {
    format!(
        "terminal-image-{}-{}-{}-{z}-{index}",
        key.namespace, key.id, key.generation
    )
}

#[test]
fn colored_crops_scale_from_authoritative_pixels_and_clip_at_the_grid() {
    terminal_view::reset_image_cache();
    let mut screen = screen(2, 2);
    screen.cell_width = 4;
    screen.cell_height = 2;
    let crop_key = key(10, 1, 1);
    let clip_key = key(10, 2, 1);
    screen.image_assets = vec![
        asset(
            crop_key,
            4,
            2,
            [
                [255, 0, 0, 255],
                [0, 255, 0, 255],
                [0, 0, 255, 255],
                [255, 255, 255, 255],
                [0, 255, 255, 255],
                [255, 0, 255, 255],
                [255, 255, 0, 255],
                [0, 0, 0, 255],
            ]
            .concat(),
        ),
        asset(
            clip_key,
            2,
            1,
            [[255, 0, 0, 255], [0, 0, 255, 255]].concat(),
        ),
    ];
    screen.images = vec![
        placement(crop_key, ImageLayer::BelowText, 0, 0, 0, 4, 2, [1, 0, 2, 2]),
        ImagePlacement {
            offset_x: 2,
            ..placement(
                clip_key,
                ImageLayer::BelowText,
                0,
                -1,
                1,
                4,
                2,
                [0, 0, 2, 1],
            )
        },
    ];
    let state = GridState {
        screen,
        settings: Settings::default(),
        cell_width: 8.0,
        cell_height: 4.0,
        active: false,
    };
    let mut rt = grid_runtime(state);
    let (width, height) = (40, 32);
    let mut capture = Capture::new(width, height);
    let rgba = capture.draw(&mut rt);
    let x = terminal_view::PAD as u32;
    let y = terminal_view::PAD as u32;
    near(pixel(&rgba, width, x + 1, y), [0, 255, 0, 255]);
    near(pixel(&rgba, width, x + 6, y), [0, 0, 255, 255]);
    near(pixel(&rgba, width, x + 1, y + 3), [255, 0, 255, 255]);
    near(pixel(&rgba, width, x + 6, y + 3), [255, 255, 0, 255]);
    near(pixel(&rgba, width, x + 2, y + 6), [0, 0, 255, 255]);
    assert_eq!(pixel(&rgba, width, x - 1, y + 6), BLACK);
    assert_eq!(pixel(&rgba, width, x + 6, y + 6), BLACK);

    if let Some(dir) = std::env::var_os("GRIZ_TERMINAL_IMAGE_STILLS") {
        let path = PathBuf::from(dir).join("terminal-image-crop-scale-clip.png");
        rune::render::write_png(&path, width, height, &rgba).unwrap();
        eprintln!("wrote {}", path.display());
    }
}

#[test]
fn ghostty_layers_z_alpha_and_cursor_order_are_preserved() {
    terminal_view::reset_image_cache();
    let mut screen = screen(5, 2);
    screen.cell_width = 8;
    screen.cell_height = 16;
    screen.cells[0][0].bg = [0, 255, 0];
    for column in [1, 2] {
        screen.cells[0][column].text = "█".into();
        screen.cells[0][column].fg = [0, 255, 0];
    }
    screen.cursor_visible = true;
    screen.cursor_x = 3;
    screen.cursor_y = 0;
    screen.cursor_color = Some([255, 255, 0]);

    let red = key(20, 1, 1);
    let blue = key(20, 2, 1);
    let half_red = key(20, 3, 1);
    screen.image_assets = vec![
        asset(red, 1, 1, vec![255, 0, 0, 255]),
        asset(blue, 1, 1, vec![0, 0, 255, 255]),
        asset(half_red, 1, 1, vec![255, 0, 0, 128]),
    ];
    screen.images = vec![
        placement(
            red,
            ImageLayer::BelowBackground,
            0,
            0,
            0,
            8,
            16,
            [0, 0, 1, 1],
        ),
        placement(red, ImageLayer::BelowText, 0, 1, 0, 8, 16, [0, 0, 1, 1]),
        placement(red, ImageLayer::AboveText, 0, 2, 0, 8, 16, [0, 0, 1, 1]),
        placement(blue, ImageLayer::AboveText, 0, 3, 0, 8, 16, [0, 0, 1, 1]),
        placement(
            half_red,
            ImageLayer::AboveText,
            2,
            4,
            0,
            8,
            16,
            [0, 0, 1, 1],
        ),
        placement(blue, ImageLayer::AboveText, -1, 4, 0, 8, 16, [0, 0, 1, 1]),
    ];
    let mut rt = grid_runtime(GridState {
        screen,
        settings: Settings::default(),
        cell_width: 8.0,
        cell_height: 16.0,
        active: true,
    });
    let width = 64;
    let mut capture = Capture::new(width, 56);
    let rgba = capture.draw(&mut rt);
    let at = |column: u32| {
        pixel(
            &rgba,
            width,
            terminal_view::PAD as u32 + column * 8 + 4,
            terminal_view::PAD as u32 + 8,
        )
    };
    near(at(0), [0, 255, 0, 255]);
    near(at(1), [0, 255, 0, 255]);
    near(at(2), [255, 0, 0, 255]);
    near(at(3), [255, 255, 0, 255]);
    near(at(4), [188, 0, 187, 255]);
}

#[test]
fn previews_scale_images_with_the_fitted_grid() {
    terminal_view::reset_image_cache();
    let mut screen = screen(2, 2);
    screen.cell_width = 4;
    screen.cell_height = 2;
    let image = key(30, 1, 1);
    screen.image_assets = vec![asset(image, 1, 1, vec![255, 128, 0, 255])];
    screen.images = vec![placement(
        image,
        ImageLayer::BelowText,
        0,
        0,
        0,
        4,
        2,
        [0, 0, 1, 1],
    )];
    let mut rt = Runtime::new(
        (screen, Settings::default()),
        |(screen, settings), _| terminal_view::preview(77, screen, settings, 30.0, 30.0),
        (30.0, 30.0),
        Env::new(true, 1.0),
    );
    rt.frame(0.0);
    let elements = rt.elements();
    let grid = elements
        .iter()
        .find(|element| element.key.as_deref() == Some("preview-screen-grid-77"))
        .unwrap();
    let image = elements
        .iter()
        .find(|element| element.key.as_deref() == Some(&image_key(image, 0, 0)))
        .unwrap();
    assert!((image.rect[2] - grid.rect[2] / 2.0).abs() < 0.05);
    assert!((image.rect[3] - grid.rect[3] / 2.0).abs() < 0.05);
    let mut capture = Capture::new(30, 30);
    let rgba = capture.draw(&mut rt);
    near(
        pixel(
            &rgba,
            30,
            (image.rect[0] + image.rect[2] / 2.0) as u32,
            (image.rect[1] + image.rect[3] / 2.0) as u32,
        ),
        [255, 128, 0, 255],
    );
}

#[test]
fn deletion_replacement_and_namespaces_do_not_reuse_the_wrong_pixels() {
    terminal_view::reset_image_cache();
    let red_key = key(40, 7, 1);
    let blue_key = key(40, 7, 2);
    let other_namespace = key(41, 7, 2);
    let mut initial = screen(2, 2);
    initial.image_assets = vec![asset(red_key, 1, 1, vec![255, 0, 0, 255])];
    initial.images = vec![placement(
        red_key,
        ImageLayer::BelowText,
        0,
        0,
        0,
        8,
        16,
        [0, 0, 1, 1],
    )];
    let mut rt = grid_runtime(GridState {
        screen: initial,
        settings: Settings::default(),
        cell_width: 8.0,
        cell_height: 16.0,
        active: false,
    });
    let mut capture = Capture::new(40, 56);
    let at = |rgba: &[u8], column: u32| {
        pixel(
            rgba,
            40,
            terminal_view::PAD as u32 + column * 8 + 4,
            terminal_view::PAD as u32 + 8,
        )
    };
    near(at(&capture.draw(&mut rt), 0), [255, 0, 0, 255]);

    rt.update(|state, tx| {
        state.screen.image_assets = vec![asset(blue_key, 1, 1, vec![0, 0, 255, 255])];
        state.screen.images = vec![placement(
            blue_key,
            ImageLayer::BelowText,
            0,
            0,
            0,
            8,
            16,
            [0, 0, 1, 1],
        )];
        tx.instant();
    });
    near(at(&capture.draw(&mut rt), 0), [0, 0, 255, 255]);

    rt.update(|state, tx| {
        state.screen.images.clear();
        state.screen.image_assets.clear();
        tx.instant();
    });
    let no_image = capture.draw(&mut rt);
    assert_eq!(at(&no_image, 0), BLACK);
    assert!(!rt.elements().iter().any(|element| {
        element
            .key
            .as_deref()
            .is_some_and(|key| key.starts_with("terminal-image-"))
    }));

    rt.update(|state, tx| {
        state.screen.image_assets = vec![
            asset(blue_key, 1, 1, vec![0, 0, 255, 255]),
            asset(other_namespace, 1, 1, vec![0, 255, 0, 255]),
        ];
        state.screen.images = vec![
            placement(
                blue_key,
                ImageLayer::BelowText,
                0,
                0,
                0,
                8,
                16,
                [0, 0, 1, 1],
            ),
            placement(
                other_namespace,
                ImageLayer::BelowText,
                0,
                1,
                0,
                8,
                16,
                [0, 0, 1, 1],
            ),
        ];
        tx.instant();
    });
    let collision = capture.draw(&mut rt);
    near(at(&collision, 0), [0, 0, 255, 255]);
    near(at(&collision, 1), [0, 255, 0, 255]);
}

#[test]
fn placeholder_is_not_ink_normal_text_is_unchanged_and_no_image_frames_are_stable() {
    terminal_view::reset_image_cache();
    let mut plain = screen(2, 2);
    plain.cells[0][0].text = "\u{10eeee}\u{305}\u{305}".into();
    plain.cells[0][1].text = "A".into();
    plain.cells[1][0].text = "█".into();
    plain.cells[1][0].fg = [20, 180, 80];
    let mut rt = grid_runtime(GridState {
        screen: plain.clone(),
        settings: Settings::default(),
        cell_width: 8.0,
        cell_height: 16.0,
        active: false,
    });
    let mut capture = Capture::new(40, 56);
    let before = capture.draw(&mut rt);
    assert!(!rt.elements().iter().any(|element| {
        element
            .text
            .as_deref()
            .is_some_and(|text| text.starts_with("\u{10eeee}"))
    }));
    assert!(
        rt.elements()
            .iter()
            .any(|element| element.text.as_deref() == Some("A"))
    );

    let transient = key(50, 1, 1);
    rt.update(|state, tx| {
        state.screen.image_assets = vec![asset(transient, 1, 1, vec![255, 0, 0, 255])];
        state.screen.images = vec![placement(
            transient,
            ImageLayer::AboveText,
            0,
            0,
            1,
            8,
            16,
            [0, 0, 1, 1],
        )];
        tx.instant();
    });
    capture.draw(&mut rt);
    rt.update(move |state, tx| {
        state.screen = plain;
        tx.instant();
    });
    let after = capture.draw(&mut rt);
    assert_eq!(after, before);
}

#[test]
fn malformed_resources_are_ignored_and_the_cpu_crop_cache_is_bounded_and_reused() {
    terminal_view::reset_image_cache();
    let image = key(60, 1, 1);
    let mut malformed = screen(2, 2);
    malformed.image_assets = vec![asset(image, 8193, 1, vec![])];
    malformed.images = vec![placement(
        image,
        ImageLayer::BelowText,
        0,
        0,
        0,
        8,
        16,
        [8192, 0, 2, 1],
    )];
    let mut rt = grid_runtime(GridState {
        screen: malformed,
        settings: Settings::default(),
        cell_width: 8.0,
        cell_height: 16.0,
        active: false,
    });
    rt.frame(0.0);
    assert_eq!(terminal_view::image_cache_usage(), (0, 0));

    for generation in 1..=300 {
        rt.update(move |state, tx| {
            let image = key(61, 1, generation);
            state.screen.image_assets = vec![asset(image, 1, 1, vec![1, 2, 3, 255])];
            state.screen.images = vec![placement(
                image,
                ImageLayer::BelowText,
                0,
                0,
                0,
                1,
                1,
                [0, 0, 1, 1],
            )];
            tx.instant();
        });
        rt.frame(0.0);
    }
    let (entries, bytes) = terminal_view::image_cache_usage();
    assert_eq!(entries, 256);
    assert!(bytes <= 32 * 1024 * 1024);
    let before = terminal_view::image_cache_usage();
    rt.invalidate();
    rt.frame(0.0);
    assert_eq!(terminal_view::image_cache_usage(), before);
}
