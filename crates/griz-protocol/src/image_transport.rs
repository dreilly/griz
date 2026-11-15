//! Bounded, stateful transport for image resources referenced by [`Event::View`].
use crate::{
    Event, IMAGE_CHUNK_BYTES, ImageAsset, ImageChunk, ImageKey, MAX_IMAGE_BYTES,
    MAX_IMAGE_DIMENSION, MAX_IMAGE_PLACEMENTS, MAX_PANES_PER_TAB, MAX_SCREEN_IMAGE_BYTES, Screen,
    ViewState, receive, send,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{self, Read, Write};
use std::sync::Arc;

/// At most the active tab's eight panes plus an eight-pane preview can contribute resources.
pub const MAX_VIEW_IMAGE_RESOURCES: usize = MAX_IMAGE_PLACEMENTS * MAX_PANES_PER_TAB * 2;
/// A view may contain one screen-sized image budget for every live and preview pane.
pub const MAX_VIEW_IMAGE_BYTES: usize = MAX_IMAGE_BYTES * MAX_PANES_PER_TAB * 2;

/// Sends complete image resources immediately before the view that references them.
///
/// Only keys referenced by the preceding view are cached. Dropping this value starts a fresh
/// connection generation and causes all resources in the next view to be sent again.
#[derive(Default)]
pub struct ImageSender {
    previous_view_keys: BTreeSet<ImageKey>,
}

impl ImageSender {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn send(&mut self, writer: &mut impl Write, event: &Event) -> io::Result<()> {
        match event {
            Event::Image { .. } => Err(invalid("raw image events must be emitted by ImageSender")),
            Event::View { state } => self.send_view(writer, event, state),
            _ => send(writer, event),
        }
    }

    fn send_view(
        &mut self,
        writer: &mut impl Write,
        event: &Event,
        state: &ViewState,
    ) -> io::Result<()> {
        let assets = collect_view_assets(state)?;
        for (key, asset) in &assets {
            if self.previous_view_keys.contains(key) {
                continue;
            }
            for (offset, bytes) in asset.rgba.chunks(IMAGE_CHUNK_BYTES).enumerate() {
                let offset = offset
                    .checked_mul(IMAGE_CHUNK_BYTES)
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| invalid("image chunk offset overflow"))?;
                send(
                    writer,
                    &Event::Image {
                        chunk: ImageChunk {
                            key: *key,
                            width: asset.width,
                            height: asset.height,
                            offset,
                            data: STANDARD.encode(bytes),
                        },
                    },
                )?;
            }
        }
        send(writer, event)?;
        self.previous_view_keys = assets.into_keys().collect();
        Ok(())
    }
}

struct Assembly {
    key: ImageKey,
    width: u32,
    height: u32,
    expected_bytes: usize,
    rgba: Vec<u8>,
}

/// Receives events while assembling and validating image chunks before exposing a view.
///
/// [`Event::Image`] is consumed internally and is never returned. Every screen in a returned
/// view owns an [`Arc<ImageAsset>`] for each placement it references.
#[derive(Default)]
pub struct ImageReceiver {
    cache: BTreeMap<ImageKey, Arc<ImageAsset>>,
    staged: BTreeMap<ImageKey, Arc<ImageAsset>>,
    assembly: Option<Assembly>,
    pending_bytes: usize,
}

impl ImageReceiver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn receive(&mut self, reader: &mut impl Read) -> io::Result<Event> {
        loop {
            let event: Event = receive(reader)?;
            match event {
                Event::Image { chunk } => {
                    if let Err(error) = self.accept_chunk(chunk) {
                        self.clear_pending();
                        return Err(error);
                    }
                }
                Event::View { mut state } => {
                    if self.assembly.is_some() {
                        self.clear_pending();
                        return Err(invalid(
                            "view arrived before an image resource was complete",
                        ));
                    }
                    if let Err(error) = self.hydrate_view(&mut state) {
                        self.clear_pending();
                        return Err(error);
                    }
                    return Ok(Event::View { state });
                }
                other => {
                    if self.assembly.is_some() || !self.staged.is_empty() {
                        self.clear_pending();
                        return Err(invalid("non-view event interrupted image resources"));
                    }
                    return Ok(other);
                }
            }
        }
    }

    fn accept_chunk(&mut self, chunk: ImageChunk) -> io::Result<()> {
        let expected_bytes = image_byte_len(chunk.width, chunk.height)?;
        let max_encoded = IMAGE_CHUNK_BYTES.div_ceil(3) * 4;
        if chunk.data.is_empty() || chunk.data.len() > max_encoded {
            return Err(invalid("invalid encoded image chunk length"));
        }
        let decoded = STANDARD
            .decode(chunk.data.as_bytes())
            .map_err(|_| invalid("invalid image chunk base64"))?;

        if self.assembly.is_none() {
            if chunk.offset != 0 {
                return Err(invalid("first image chunk offset is not zero"));
            }
            if self.cache.contains_key(&chunk.key) || self.staged.contains_key(&chunk.key) {
                return Err(invalid("duplicate image resource"));
            }
            if self.staged.len() >= MAX_VIEW_IMAGE_RESOURCES {
                return Err(invalid("too many image resources for one view"));
            }
            self.assembly = Some(Assembly {
                key: chunk.key,
                width: chunk.width,
                height: chunk.height,
                expected_bytes,
                rgba: Vec::with_capacity(expected_bytes),
            });
        }

        let assembly = self.assembly.as_mut().unwrap();
        if chunk.key != assembly.key
            || chunk.width != assembly.width
            || chunk.height != assembly.height
            || expected_bytes != assembly.expected_bytes
        {
            return Err(invalid("image chunks are not sequential"));
        }
        if usize::try_from(chunk.offset).ok() != Some(assembly.rgba.len()) {
            return Err(invalid("invalid image chunk offset"));
        }
        let remaining = assembly.expected_bytes - assembly.rgba.len();
        let expected_chunk = remaining.min(IMAGE_CHUNK_BYTES);
        if decoded.len() != expected_chunk {
            return Err(invalid("invalid decoded image chunk length"));
        }
        self.pending_bytes = self
            .pending_bytes
            .checked_add(decoded.len())
            .filter(|total| *total <= MAX_VIEW_IMAGE_BYTES)
            .ok_or_else(|| invalid("image resources exceed the view byte budget"))?;
        assembly.rgba.extend_from_slice(&decoded);

        if assembly.rgba.len() == assembly.expected_bytes {
            let assembly = self.assembly.take().unwrap();
            let asset = Arc::new(ImageAsset {
                key: assembly.key,
                width: assembly.width,
                height: assembly.height,
                rgba: assembly.rgba,
            });
            self.staged.insert(asset.key, asset);
        }
        Ok(())
    }

    fn hydrate_view(&mut self, state: &mut ViewState) -> io::Result<()> {
        let mut referenced = BTreeSet::new();
        for_each_screen(state, |screen| {
            validate_screen(screen, |key| {
                let asset = self
                    .staged
                    .get(&key)
                    .or_else(|| self.cache.get(&key))
                    .ok_or_else(|| invalid("view references a missing image resource"))?;
                referenced.insert(key);
                Ok(asset.clone())
            })
        })?;

        if referenced.len() > MAX_VIEW_IMAGE_RESOURCES {
            return Err(invalid("too many image resources for one view"));
        }
        if self.staged.keys().any(|key| !referenced.contains(key)) {
            return Err(invalid(
                "image resource is not referenced by its following view",
            ));
        }

        let mut resolved = BTreeMap::new();
        let mut total_bytes = 0usize;
        for key in referenced {
            let asset = self
                .staged
                .get(&key)
                .or_else(|| self.cache.get(&key))
                .ok_or_else(|| invalid("view references a missing image resource"))?
                .clone();
            total_bytes = total_bytes
                .checked_add(asset.rgba.len())
                .filter(|total| *total <= MAX_VIEW_IMAGE_BYTES)
                .ok_or_else(|| invalid("image resources exceed the view byte budget"))?;
            resolved.insert(key, asset);
        }

        for_each_screen_mut(state, |screen| {
            let mut seen = HashSet::new();
            screen.image_assets.clear();
            for placement in &screen.images {
                if seen.insert(placement.key) {
                    screen.image_assets.push(resolved[&placement.key].clone());
                }
            }
        });
        self.cache = resolved;
        self.staged.clear();
        self.pending_bytes = 0;
        Ok(())
    }

    fn clear_pending(&mut self) {
        self.staged.clear();
        self.assembly = None;
        self.pending_bytes = 0;
    }
}

fn collect_view_assets(state: &ViewState) -> io::Result<BTreeMap<ImageKey, Arc<ImageAsset>>> {
    let mut available = BTreeMap::<ImageKey, Arc<ImageAsset>>::new();
    for_each_screen(state, |screen| {
        for asset in &screen.image_assets {
            validate_asset(asset)?;
            if let Some(old) = available.insert(asset.key, asset.clone())
                && !Arc::ptr_eq(&old, asset)
                && old.as_ref() != asset.as_ref()
            {
                return Err(invalid("one image key identifies different resources"));
            }
        }
        Ok(())
    })?;

    let mut referenced = BTreeSet::new();
    for_each_screen(state, |screen| {
        validate_screen(screen, |key| {
            let asset = available
                .get(&key)
                .ok_or_else(|| invalid("view references an image without owned asset data"))?;
            referenced.insert(key);
            Ok(asset.clone())
        })
    })?;
    if referenced.len() > MAX_VIEW_IMAGE_RESOURCES {
        return Err(invalid("too many image resources for one view"));
    }

    let mut assets = BTreeMap::new();
    let mut total_bytes = 0usize;
    for key in referenced {
        let asset = available.remove(&key).unwrap();
        total_bytes = total_bytes
            .checked_add(asset.rgba.len())
            .filter(|total| *total <= MAX_VIEW_IMAGE_BYTES)
            .ok_or_else(|| invalid("image resources exceed the view byte budget"))?;
        assets.insert(key, asset);
    }
    Ok(assets)
}

fn validate_screen(
    screen: &Screen,
    mut resolve: impl FnMut(ImageKey) -> io::Result<Arc<ImageAsset>>,
) -> io::Result<()> {
    if screen.images.len() > MAX_IMAGE_PLACEMENTS {
        return Err(invalid("too many image placements on one screen"));
    }
    let mut keys = BTreeSet::new();
    let mut bytes = 0usize;
    for placement in &screen.images {
        let asset = resolve(placement.key)?;
        validate_asset(&asset)?;
        if placement.width == 0
            || placement.height == 0
            || placement.source_width == 0
            || placement.source_height == 0
            || placement
                .source_x
                .checked_add(placement.source_width)
                .is_none_or(|right| right > asset.width)
            || placement
                .source_y
                .checked_add(placement.source_height)
                .is_none_or(|bottom| bottom > asset.height)
        {
            return Err(invalid("invalid image placement dimensions"));
        }
        if keys.insert(placement.key) {
            bytes = bytes
                .checked_add(asset.rgba.len())
                .filter(|total| *total <= MAX_SCREEN_IMAGE_BYTES)
                .ok_or_else(|| invalid("images exceed the screen byte budget"))?;
        }
    }
    Ok(())
}

fn validate_asset(asset: &ImageAsset) -> io::Result<()> {
    let expected = image_byte_len(asset.width, asset.height)?;
    if asset.rgba.len() != expected {
        return Err(invalid("image RGBA length does not match its dimensions"));
    }
    Ok(())
}

fn image_byte_len(width: u32, height: u32) -> io::Result<usize> {
    if width == 0 || height == 0 || width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(invalid("invalid image dimensions"));
    }
    usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| *bytes <= MAX_IMAGE_BYTES)
        .ok_or_else(|| invalid("image exceeds the resource byte budget"))
}

fn for_each_screen(
    state: &ViewState,
    mut visit: impl FnMut(&Screen) -> io::Result<()>,
) -> io::Result<()> {
    if let Some(terminal) = &state.terminal {
        visit(&terminal.screen)?;
    }
    for terminal in &state.terminals {
        visit(&terminal.screen)?;
    }
    if let Some(preview) = &state.preview {
        for terminal in &preview.terminals {
            visit(&terminal.screen)?;
        }
    }
    Ok(())
}

fn for_each_screen_mut(state: &mut ViewState, mut visit: impl FnMut(&mut Screen)) {
    if let Some(terminal) = &mut state.terminal {
        visit(&mut terminal.screen);
    }
    for terminal in &mut state.terminals {
        visit(&mut terminal.screen);
    }
    if let Some(preview) = &mut state.preview {
        for terminal in &mut preview.terminals {
            visit(&mut terminal.screen);
        }
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ImageLayer, ImagePlacement, PaneLayout, PreviewFrame, PreviewTarget, TerminalFrame,
    };

    fn asset(id: u32, width: u32, height: u32, fill: u8) -> Arc<ImageAsset> {
        Arc::new(ImageAsset {
            key: ImageKey {
                namespace: 7,
                id,
                generation: 1,
            },
            width,
            height,
            rgba: vec![fill; width as usize * height as usize * 4],
        })
    }

    fn screen(asset: Option<Arc<ImageAsset>>) -> Screen {
        let images = asset
            .as_ref()
            .map(|asset| {
                vec![ImagePlacement {
                    key: asset.key,
                    layer: ImageLayer::AboveText,
                    z: 0,
                    col: 0,
                    row: 0,
                    offset_x: 0,
                    offset_y: 0,
                    width: asset.width,
                    height: asset.height,
                    source_x: 0,
                    source_y: 0,
                    source_width: asset.width,
                    source_height: asset.height,
                }]
            })
            .unwrap_or_default();
        Screen {
            cols: 2,
            rows: 2,
            cells: vec![],
            row_soft_wrapped: vec![],
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: false,
            cursor_shape: crate::CursorShape::Block,
            cursor_blinking: false,
            cursor_color: None,
            title: String::new(),
            application_cursor: false,
            bracketed_paste: false,
            mouse_tracking: false,
            scrollback_offset: 0,
            cell_width: 8,
            cell_height: 16,
            images,
            image_assets: asset.into_iter().collect(),
        }
    }

    fn view(asset: Option<Arc<ImageAsset>>) -> Event {
        let frame = TerminalFrame {
            id: 1,
            shell_pid: 2,
            screen: screen(asset),
            exited: None,
        };
        Event::View {
            state: ViewState {
                core_pid: 3,
                default_cwd: String::new(),
                ack: 0,
                workspaces: vec![],
                active_workspace: 0,
                active_terminal: Some(1),
                terminals: vec![frame.clone()],
                terminal: Some(frame),
                preview: None,
            },
        }
    }

    fn raw_events(mut bytes: &[u8]) -> Vec<Event> {
        let mut events = Vec::new();
        while !bytes.is_empty() {
            events.push(receive(&mut bytes).unwrap());
        }
        events
    }

    #[test]
    fn unchanged_asset_is_deduplicated_across_screens_and_views() {
        let asset = asset(1, 200, 100, 9);
        let mut event = view(Some(asset));
        let Event::View { state } = &mut event else {
            unreachable!()
        };
        state.preview = Some(Box::new(PreviewFrame {
            target: PreviewTarget::Terminal {
                workspace_id: 1,
                terminal_id: 1,
            },
            layout: PaneLayout::Leaf { terminal_id: 1 },
            selected: 1,
            terminals: state.terminals.clone(),
        }));
        let mut sender = ImageSender::new();
        let mut bytes = Vec::new();
        sender.send(&mut bytes, &event).unwrap();
        sender.send(&mut bytes, &event).unwrap();

        let events = raw_events(&bytes);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::Image { .. }))
                .count(),
            2
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::View { .. }))
                .count(),
            2
        );
        let json = serde_json::to_value(&event).unwrap();
        assert!(json.to_string().find("rgba").is_none());
        assert!(json.to_string().find("image_assets").is_none());
    }

    #[test]
    fn deletion_reappearance_and_reconnect_resend_the_asset() {
        let asset = asset(1, 1, 1, 4);
        let present = view(Some(asset.clone()));
        let absent = view(None);
        let mut bytes = Vec::new();
        let mut sender = ImageSender::new();
        sender.send(&mut bytes, &present).unwrap();
        sender.send(&mut bytes, &absent).unwrap();
        sender.send(&mut bytes, &present).unwrap();
        let image_count = raw_events(&bytes)
            .into_iter()
            .filter(|event| matches!(event, Event::Image { .. }))
            .count();
        assert_eq!(image_count, 2);

        let mut reconnect = Vec::new();
        ImageSender::new().send(&mut reconnect, &present).unwrap();
        assert!(matches!(raw_events(&reconnect)[0], Event::Image { .. }));

        let mut receiver = ImageReceiver::new();
        let mut wire = bytes.as_slice();
        let first = receiver.receive(&mut wire).unwrap();
        let Event::View { state: first } = first else {
            unreachable!()
        };
        let retained = first.terminal.unwrap().screen.image_assets[0].clone();
        let _ = receiver.receive(&mut wire).unwrap();
        let third = receiver.receive(&mut wire).unwrap();
        let Event::View { state: third } = third else {
            unreachable!()
        };
        assert_eq!(
            retained.rgba,
            third.terminal.unwrap().screen.image_assets[0].rgba
        );
    }

    #[test]
    fn rejects_missing_invalid_and_incomplete_resources() {
        let small_asset = asset(1, 1, 1, 3);
        let event = view(Some(small_asset.clone()));
        let mut missing = Vec::new();
        send(&mut missing, &event).unwrap();
        assert_eq!(
            ImageReceiver::new()
                .receive(&mut missing.as_slice())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );

        let bad = Event::Image {
            chunk: ImageChunk {
                key: small_asset.key,
                width: 1,
                height: 1,
                offset: 0,
                data: "%%%".into(),
            },
        };
        let mut invalid_base64 = Vec::new();
        send(&mut invalid_base64, &bad).unwrap();
        assert!(
            ImageReceiver::new()
                .receive(&mut invalid_base64.as_slice())
                .is_err()
        );

        let mut invalid_offset = Vec::new();
        send(
            &mut invalid_offset,
            &Event::Image {
                chunk: ImageChunk {
                    key: small_asset.key,
                    width: 1,
                    height: 1,
                    offset: 1,
                    data: STANDARD.encode([0, 0, 0, 0]),
                },
            },
        )
        .unwrap();
        assert!(
            ImageReceiver::new()
                .receive(&mut invalid_offset.as_slice())
                .is_err()
        );

        let large = asset(2, 200, 100, 1);
        let mut incomplete = Vec::new();
        send(
            &mut incomplete,
            &Event::Image {
                chunk: ImageChunk {
                    key: large.key,
                    width: large.width,
                    height: large.height,
                    offset: 0,
                    data: STANDARD.encode(&large.rgba[..IMAGE_CHUNK_BYTES]),
                },
            },
        )
        .unwrap();
        send(&mut incomplete, &view(Some(large))).unwrap();
        assert!(
            ImageReceiver::new()
                .receive(&mut incomplete.as_slice())
                .is_err()
        );
    }

    #[test]
    fn near_limit_asset_spans_bounded_wire_messages_and_hydrates_once() {
        let asset = asset(9, 2048, 2047, 0x5a);
        assert!(asset.rgba.len() < MAX_IMAGE_BYTES);
        let event = view(Some(asset.clone()));
        let mut wire = Vec::new();
        ImageSender::new().send(&mut wire, &event).unwrap();

        let events = raw_events(&wire);
        let chunks: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::Image { chunk } => Some(chunk),
                _ => None,
            })
            .collect();
        assert!(chunks.len() > 1);
        assert!(
            chunks
                .iter()
                .all(|chunk| { STANDARD.decode(&chunk.data).unwrap().len() <= IMAGE_CHUNK_BYTES })
        );

        let mut receiver = ImageReceiver::new();
        let Event::View { state } = receiver.receive(&mut wire.as_slice()).unwrap() else {
            unreachable!()
        };
        let selected = &state.terminal.as_ref().unwrap().screen.image_assets[0];
        let pane = &state.terminals[0].screen.image_assets[0];
        assert_eq!(selected.rgba.len(), asset.rgba.len());
        assert!(Arc::ptr_eq(selected, pane));
    }
}
