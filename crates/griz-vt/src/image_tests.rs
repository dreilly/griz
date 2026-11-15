use super::*;
use std::sync::Arc;

fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        encoded.push(ALPHABET[((value >> 18) & 63) as usize] as char);
        encoded.push(ALPHABET[((value >> 12) & 63) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            ALPHABET[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            ALPHABET[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

fn kitty(controls: &str, payload: &str) -> Vec<u8> {
    format!("\x1b_G{controls};{payload}\x1b\\").into_bytes()
}

fn raw_rgba(id: u32, width: u32, height: u32, pixels: &[u8], action: &str) -> Vec<u8> {
    kitty(
        &format!("a={action},f=32,s={width},v={height},i={id},q=2,C=1"),
        &base64(pixels),
    )
}

fn png_bytes(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(rgba).unwrap();
    }
    bytes
}

/// Matches pi-tui's encodeKitty controls and 4096-character chunk boundaries.
fn pi_kitty_png(id: u32, columns: u32, rows: u32, png: &[u8]) -> Vec<u8> {
    const CHUNK_SIZE: usize = 4096;
    let data = base64(png);
    let controls = format!("a=T,f=100,q=2,C=1,c={columns},r={rows},i={id}");
    if data.len() <= CHUNK_SIZE {
        return kitty(&controls, &data);
    }
    let mut encoded = Vec::new();
    for (index, chunk) in data.as_bytes().chunks(CHUNK_SIZE).enumerate() {
        let last = (index + 1) * CHUNK_SIZE >= data.len();
        let controls = if index == 0 {
            format!("{controls},m=1")
        } else if last {
            "m=0".into()
        } else {
            "m=1".into()
        };
        encoded.extend_from_slice(&kitty(&controls, std::str::from_utf8(chunk).unwrap()));
    }
    encoded
}

fn asset(screen: &Screen) -> &Arc<ImageAsset> {
    assert_eq!(screen.image_assets.len(), 1);
    &screen.image_assets[0]
}

#[test]
fn exact_pi_png_chunks_decode_and_capture_without_recopying() {
    let width = 64;
    let height = 32;
    let mut state = 0x1234_5678u32;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..width * height {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        rgba.extend_from_slice(&[state as u8, (state >> 8) as u8, (state >> 16) as u8, 255]);
    }
    let png = png_bytes(width, height, &rgba);
    let sequence = pi_kitty_png(101, 8, 2, &png);
    assert!(
        png.len() * 4 / 3 > 4096,
        "fixture must exercise Pi chunking"
    );
    assert!(sequence.windows(3).any(|window| window == b"m=1"));
    assert!(sequence.windows(3).any(|window| window == b"m=0"));

    let mut terminal = Terminal::new(20, 6).unwrap();
    for chunk in sequence.chunks(997) {
        terminal.write(chunk);
    }
    let first = terminal.screen().unwrap();
    assert_eq!((first.cell_width, first.cell_height), (8, 16));
    assert_eq!(first.images.len(), 1);
    assert_eq!((first.images[0].width, first.images[0].height), (64, 32));
    assert_eq!((asset(&first).width, asset(&first).height), (width, height));
    assert_eq!(asset(&first).rgba, rgba);

    let second = terminal.screen().unwrap();
    assert!(Arc::ptr_eq(asset(&first), asset(&second)));
    assert_eq!(terminal.image_cache.len(), 1);
}

#[test]
fn raw_rgba_replacement_generation_delete_and_namespace_are_stable() {
    let first_pixels = [255, 0, 0, 255, 0, 255, 0, 128];
    let replacement = [0, 0, 255, 255, 255, 255, 0, 64];
    let mut terminal = Terminal::new(8, 3).unwrap();
    terminal.write(&raw_rgba(7, 2, 1, &first_pixels, "T"));
    let frozen = terminal.screen().unwrap();
    let first_key = frozen.images[0].key;
    assert_eq!(asset(&frozen).rgba, first_pixels);

    terminal.write(&raw_rgba(7, 2, 1, &replacement, "T"));
    let replaced = terminal.screen().unwrap();
    assert_eq!(replaced.images.len(), 1);
    assert_eq!(replaced.images[0].key.id, first_key.id);
    assert_eq!(replaced.images[0].key.namespace, first_key.namespace);
    assert_ne!(replaced.images[0].key.generation, first_key.generation);
    assert_eq!(asset(&replaced).rgba, replacement);
    assert_eq!(
        asset(&frozen).rgba,
        first_pixels,
        "frozen frames retain their Arc"
    );
    assert_eq!(
        terminal.image_cache.len(),
        1,
        "obsolete generations are pruned"
    );

    terminal.write(b"\x1b_Ga=d,d=I,i=7,q=2\x1b\\");
    let deleted = terminal.screen().unwrap();
    assert!(deleted.images.is_empty());
    assert!(deleted.image_assets.is_empty());
    assert!(terminal.image_cache.is_empty());

    let mut other = Terminal::new(8, 3).unwrap();
    other.write(&raw_rgba(7, 2, 1, &first_pixels, "T"));
    let other_key = other.screen().unwrap().images[0].key;
    assert_ne!(other_key.namespace, first_key.namespace);
}

#[test]
fn placements_export_crop_scroll_layers_and_cell_geometry() {
    let pixels: Vec<u8> = (0..16)
        .flat_map(|pixel| [pixel as u8, 0, 255 - pixel as u8, 255])
        .collect();
    let mut terminal = Terminal::new(12, 4).unwrap();
    terminal.set_cell_size(10, 20).unwrap();
    terminal.write(&raw_rgba(9, 4, 4, &pixels, "t"));
    for (cursor, placement, z) in [
        (b"\x1b[1;2H".as_slice(), 1, -1_073_741_825),
        (b"\x1b[2;3H".as_slice(), 2, -1),
        (b"\x1b[3;4H".as_slice(), 3, 2),
    ] {
        terminal.write(cursor);
        terminal.write(&kitty(
            &format!("a=p,i=9,p={placement},q=2,C=1,x=1,y=1,w=2,h=3,c=2,r=1,X=3,Y=4,z={z}"),
            "",
        ));
    }
    let screen = terminal.screen().unwrap();
    assert_eq!((screen.cell_width, screen.cell_height), (10, 20));
    assert_eq!(screen.images.len(), 3);
    assert_eq!(
        screen
            .images
            .iter()
            .map(|image| image.layer)
            .collect::<Vec<_>>(),
        vec![
            ImageLayer::BelowBackground,
            ImageLayer::BelowText,
            ImageLayer::AboveText
        ]
    );
    for (index, image) in screen.images.iter().enumerate() {
        assert_eq!((image.col, image.row), ((index + 1) as i32, index as i32));
        assert_eq!((image.offset_x, image.offset_y), (3, 4));
        assert_eq!((image.width, image.height), (17, 16));
        assert_eq!(
            (
                image.source_x,
                image.source_y,
                image.source_width,
                image.source_height
            ),
            (1, 1, 2, 3)
        );
    }

    terminal.write(b"\x1b[4;1H\r\n");
    let scrolled = terminal.screen().unwrap();
    assert_eq!(scrolled.images.len(), 2);
    assert_eq!(
        scrolled
            .images
            .iter()
            .map(|image| image.row)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
}

#[test]
fn alternate_screen_isolated_images_restore_and_reset() {
    let pixels = [1, 2, 3, 255];
    let mut terminal = Terminal::new(8, 3).unwrap();
    terminal.write(&raw_rgba(1, 1, 1, &pixels, "T"));
    let primary = terminal.screen().unwrap();
    assert_eq!(primary.images.len(), 1);

    terminal.write(b"\x1b[?1049h");
    assert!(terminal.screen().unwrap().images.is_empty());
    terminal.write(&raw_rgba(2, 1, 1, &pixels, "T"));
    assert_eq!(terminal.screen().unwrap().images[0].key.id, 2);
    terminal.write(b"\x1b[?1049l");
    let restored = terminal.screen().unwrap();
    assert_eq!(restored.images.len(), 1);
    assert_eq!(restored.images[0].key.id, 1);
    assert_eq!(asset(&primary).rgba, pixels);

    terminal.write(b"\x1bc");
    assert!(terminal.screen().unwrap().images.is_empty());
    terminal.write(&raw_rgba(1, 1, 1, &[10, 20, 30, 255], "T"));
    let after_reset = terminal.screen().unwrap();
    assert_ne!(after_reset.images[0].key, primary.images[0].key);
    assert_eq!(asset(&after_reset).rgba, [10, 20, 30, 255]);
}

#[test]
fn virtual_placeholder_and_relative_placements_use_ghosttys_resolved_geometry() {
    let mut terminal = Terminal::new(12, 4).unwrap();
    terminal.write(&raw_rgba(7, 2, 1, &[255, 0, 0, 255, 0, 255, 0, 255], "t"));
    terminal.write(&kitty("a=p,U=1,i=7,p=1,c=2,r=1,q=2", ""));
    terminal.write(
        "\x1b[38;2;0;0;7m\u{10eeee}\u{305}\u{305}\u{10eeee}\u{305}\u{30d}\x1b[0m".as_bytes(),
    );
    let virtual_screen = terminal.screen().unwrap();
    assert!(!virtual_screen.images.is_empty());
    assert_eq!(virtual_screen.image_assets.len(), 1);
    assert!(
        virtual_screen
            .images
            .iter()
            .all(|image| image.key.id == 7 && image.layer == ImageLayer::BelowText)
    );

    terminal.write(&raw_rgba(8, 1, 1, &[0, 0, 255, 255], "t"));
    terminal.write(&kitty("a=p,i=8,p=2,P=7,Q=1,H=1,V=1,c=1,r=1,q=2,C=1", ""));
    let relative = terminal.screen().unwrap();
    let child = relative
        .images
        .iter()
        .find(|image| image.key.id == 8)
        .expect("Ghostty should resolve a relative child of the virtual placement");
    assert_eq!((child.col, child.row), (1, 1));
}

#[test]
fn size_queries_and_resizes_keep_authoritative_pixels() {
    let mut terminal = Terminal::new(80, 24).unwrap();
    terminal.write(b"\x1b[16t");
    assert_eq!(terminal.take_responses().unwrap(), b"\x1b[6;16;8t");

    terminal.set_cell_size(11, 22).unwrap();
    terminal.resize(100, 30).unwrap();
    terminal.write(b"\x1b[16t\x1b[14t\x1b[18t");
    assert_eq!(
        terminal.take_responses().unwrap(),
        b"\x1b[6;22;11t\x1b[4;660;1100t\x1b[8;30;100t"
    );
    let screen = terminal.screen().unwrap();
    assert_eq!((screen.cell_width, screen.cell_height), (11, 22));
    assert!(terminal.set_cell_size(0, 22).is_err());
    assert_eq!(terminal.screen().unwrap().cell_width, 11);
}

#[test]
fn generic_kitty_capability_query_replies_before_device_attributes_without_a_placement() {
    let mut terminal = Terminal::new(8, 3).unwrap();
    terminal.write(b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c");
    let responses = terminal.take_responses().unwrap();
    assert!(
        responses.starts_with(b"\x1b_Gi=31;OK\x1b\\"),
        "responses={responses:?}"
    );
    assert!(responses.ends_with(b"\x1b[?62;22c"));
    assert!(terminal.screen().unwrap().images.is_empty());
}

#[test]
fn unsafe_media_and_malformed_png_are_rejected() {
    let mut terminal = Terminal::new(8, 3).unwrap();
    for medium in ["f", "t", "s"] {
        terminal.write(&kitty(
            &format!("a=T,t={medium},f=32,s=1,v=1,i=40,q=2,C=1"),
            &base64(b"/tmp/griz-must-not-load"),
        ));
        assert!(terminal.screen().unwrap().images.is_empty());
    }
    terminal.write(&kitty("a=T,f=100,i=41,q=2,C=1", "bm90IGEgcG5n"));
    let oversized = vec![0u8; (MAX_IMAGE_DIMENSION as usize + 1) * 4];
    terminal.write(&raw_rgba(42, MAX_IMAGE_DIMENSION + 1, 1, &oversized, "T"));
    assert!(terminal.screen().unwrap().images.is_empty());
}

#[test]
fn excess_placements_are_bounded() {
    let mut terminal = Terminal::new(8, 3).unwrap();
    terminal.write(&raw_rgba(50, 1, 1, &[9, 8, 7, 255], "t"));
    for placement in 1..=300 {
        terminal.write(&kitty(&format!("a=p,i=50,p={placement},q=2,C=1"), ""));
    }
    let bounded = terminal.screen().unwrap();
    assert!(bounded.images.len() <= MAX_IMAGE_PLACEMENTS);
    assert_eq!(bounded.image_assets.len(), 1);
    assert!(
        bounded
            .image_assets
            .iter()
            .map(|asset| asset.rgba.len())
            .sum::<usize>()
            <= MAX_SCREEN_IMAGE_BYTES
    );
}
