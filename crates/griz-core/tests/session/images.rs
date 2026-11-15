use super::*;
use griz_protocol::image_transport::ImageReceiver;

const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEklEQVR4nGP4z8DwHwyBNBgAAEnICff5q7YNAAAAAElFTkSuQmCC";

fn image_view(
    stream: &mut UnixStream,
    images: &mut ImageReceiver,
    mut predicate: impl FnMut(&ViewState) -> bool,
) -> ViewState {
    loop {
        match images.receive(stream).unwrap() {
            Event::View { state } if predicate(&state) => return state,
            Event::Error { message, .. } => panic!("core error: {message}"),
            _ => {}
        }
    }
}

fn screen(state: &ViewState) -> &Screen {
    &state.terminal.as_ref().unwrap().screen
}

#[test]
fn pi_png_crosses_the_real_pty_and_chunk_transport_and_reconnects() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let mut images = ImageReceiver::new();
    let initial = image_view(&mut stream, &mut images, |_| true);
    let id = initial.active_terminal.unwrap();
    input(&mut stream, 1, id, format!(
        "stty -echo; printf '\\033[2J\\033[H\\033_Ga=T,f=100,q=2,C=1,c=4,r=2,i=101;{PNG}\\033\\\\'; printf 'image-%s\\n' ready\r"
    ).as_bytes());
    let first = image_view(&mut stream, &mut images, |state| {
        !screen(state).images.is_empty()
    });
    let asset = &screen(&first).image_assets[0];
    assert_eq!((asset.width, asset.height), (2, 2));
    assert_eq!(
        asset.rgba,
        [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255
        ]
    );
    let key = asset.key;

    let mut reconnected = service.attach(80, 24);
    let mut reconnect_images = ImageReceiver::new();
    let attached = image_view(&mut reconnected, &mut reconnect_images, |state| {
        !screen(state).images.is_empty()
    });
    assert_eq!(screen(&attached).image_assets[0].key, key);
    assert_eq!(screen(&attached).image_assets[0].rgba, asset.rgba);

    command(
        &mut stream,
        2,
        CoreCommand::SetPreview {
            target: Some(griz_protocol::PreviewTarget::Terminal {
                workspace_id: initial.active_workspace,
                terminal_id: id,
            }),
        },
    );
    let preview = image_view(&mut stream, &mut images, |state| state.ack >= 2);
    let preview_screen = &preview.preview.as_ref().unwrap().terminals[0].screen;
    assert_eq!(preview_screen.image_assets[0].key, key);

    input(
        &mut stream,
        3,
        id,
        b"printf '\\033_Ga=d,d=I,i=101,q=2\\033\\\\'\r",
    );
    let deleted = image_view(&mut stream, &mut images, |state| {
        screen(state).images.is_empty()
    });
    assert!(screen(&deleted).image_assets.is_empty());
    assert!(
        deleted.preview.unwrap().terminals[0]
            .screen
            .image_assets
            .is_empty()
    );
}

#[test]
fn synchronized_image_replacement_keeps_completed_pixels_for_existing_and_new_clients() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let mut images = ImageReceiver::new();
    let initial = image_view(&mut stream, &mut images, |_| true);
    let id = initial.active_terminal.unwrap();
    // A 1x1 opaque red image. C=1 keeps the shell's cursor at its anchor.
    input(&mut stream, 1, id, b"stty -echo; printf '\\033[2J\\033[H\\033_Ga=T,f=32,s=1,v=1,c=2,r=2,i=7,q=2,C=1;/wAA/w==\\033\\\\'\r");
    let prior = image_view(&mut stream, &mut images, |state| {
        !screen(state).images.is_empty()
    });
    let old = screen(&prior).image_assets[0].clone();
    assert_eq!(old.rgba, [255, 0, 0, 255]);

    let gate = service.dir.join("image-held");
    input(&mut stream, 2, id, format!(
        "printf '\\033[?2026h\\033_Ga=T,f=32,s=1,v=1,c=2,r=2,i=7,q=2,C=1;AAD//w==\\033\\\\'; touch '{}'; read gate; printf '\\033[?2026l'\r", gate.display()
    ).as_bytes());
    let deadline = Instant::now() + Duration::from_millis(700);
    while !gate.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(gate.exists(), "shell did not enter hold");
    command(&mut stream, 3, CoreCommand::SetPreview { target: None });
    let held = image_view(&mut stream, &mut images, |state| state.ack >= 3);
    assert_eq!(screen(&held).image_assets[0].key, old.key);
    assert_eq!(screen(&held).image_assets[0].rgba, old.rgba);

    let mut attached = service.attach(80, 24);
    let mut attached_images = ImageReceiver::new();
    let during_hold = image_view(&mut attached, &mut attached_images, |_| true);
    assert_eq!(screen(&during_hold).image_assets[0].key, old.key);
    assert_eq!(screen(&during_hold).image_assets[0].rgba, old.rgba);

    input(&mut stream, 4, id, b"release\r");
    let released = image_view(&mut stream, &mut images, |state| {
        screen(state)
            .image_assets
            .first()
            .is_some_and(|asset| asset.key != old.key)
    });
    assert_eq!(screen(&released).image_assets[0].rgba, [0, 0, 255, 255]);
    assert_eq!(
        old.rgba,
        [255, 0, 0, 255],
        "completed frame ownership must survive replacement"
    );
}
