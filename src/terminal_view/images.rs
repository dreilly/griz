use griz_protocol::{
    ImageAsset, ImageKey, ImageLayer, ImagePlacement, MAX_IMAGE_BYTES, MAX_IMAGE_DIMENSION,
};
use rune::{El, Pixels, el};
use std::{cell::RefCell, collections::HashMap};

const CACHE_BYTES: usize = 32 * 1024 * 1024;
const CACHE_ENTRIES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SourceRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CacheKey {
    image: ImageKey,
    source: SourceRect,
}

struct Entry {
    pixels: Pixels,
    bytes: usize,
    used: u64,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<CacheKey, Entry>,
    bytes: usize,
    clock: u64,
}

impl Cache {
    fn pixels(&mut self, asset: &ImageAsset, source: SourceRect) -> Option<Pixels> {
        let key = CacheKey {
            image: asset.key,
            source,
        };
        self.clock = self.clock.saturating_add(1);
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.used = self.clock;
            return Some(entry.pixels.clone());
        }

        let rgba = crop(asset, source)?;
        let bytes = rgba.len();
        let pixels = Pixels::new();
        pixels.set(source.width, source.height, rgba);
        self.bytes = self.bytes.saturating_add(bytes);
        self.entries.insert(
            key,
            Entry {
                pixels: pixels.clone(),
                bytes,
                used: self.clock,
            },
        );
        while self.entries.len() > CACHE_ENTRIES || self.bytes > CACHE_BYTES {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| *key)
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(entry.bytes);
            }
        }
        Some(pixels)
    }
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

fn valid_asset(asset: &ImageAsset) -> bool {
    if asset.width == 0
        || asset.height == 0
        || asset.width > MAX_IMAGE_DIMENSION
        || asset.height > MAX_IMAGE_DIMENSION
        || asset.rgba.len() > MAX_IMAGE_BYTES
    {
        return false;
    }
    asset
        .width
        .checked_mul(asset.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok())
        == Some(asset.rgba.len())
}

fn source_rect(asset: &ImageAsset, placement: &ImagePlacement) -> Option<SourceRect> {
    if placement.source_width == 0
        || placement.source_height == 0
        || placement.source_width > MAX_IMAGE_DIMENSION
        || placement.source_height > MAX_IMAGE_DIMENSION
    {
        return None;
    }
    let right = placement.source_x.checked_add(placement.source_width)?;
    let bottom = placement.source_y.checked_add(placement.source_height)?;
    (right <= asset.width && bottom <= asset.height).then_some(SourceRect {
        x: placement.source_x,
        y: placement.source_y,
        width: placement.source_width,
        height: placement.source_height,
    })
}

fn crop(asset: &ImageAsset, source: SourceRect) -> Option<Vec<u8>> {
    let row_bytes = usize::try_from(source.width.checked_mul(4)?).ok()?;
    let bytes = row_bytes.checked_mul(usize::try_from(source.height).ok()?)?;
    if bytes > MAX_IMAGE_BYTES {
        return None;
    }
    if source.x == 0
        && source.y == 0
        && source.width == asset.width
        && source.height == asset.height
    {
        return Some(asset.rgba.clone());
    }

    let asset_row = usize::try_from(asset.width.checked_mul(4)?).ok()?;
    let x = usize::try_from(source.x.checked_mul(4)?).ok()?;
    let first_row = usize::try_from(source.y).ok()?;
    let mut rgba = Vec::with_capacity(bytes);
    for row in first_row..first_row.checked_add(usize::try_from(source.height).ok()?)? {
        let start = row.checked_mul(asset_row)?.checked_add(x)?;
        let end = start.checked_add(row_bytes)?;
        rgba.extend_from_slice(asset.rgba.get(start..end)?);
    }
    Some(rgba)
}

fn geometry(
    screen: &griz_protocol::Screen,
    placement: &ImagePlacement,
    cell_width: f32,
    cell_height: f32,
) -> Option<[f32; 4]> {
    if screen.cell_width == 0
        || screen.cell_height == 0
        || placement.width == 0
        || placement.height == 0
        || !cell_width.is_finite()
        || !cell_height.is_finite()
        || cell_width <= 0.0
        || cell_height <= 0.0
    {
        return None;
    }
    let source_x = i64::from(placement.col)
        .checked_mul(i64::from(screen.cell_width))?
        .checked_add(i64::from(placement.offset_x))?;
    let source_y = i64::from(placement.row)
        .checked_mul(i64::from(screen.cell_height))?
        .checked_add(i64::from(placement.offset_y))?;
    let scale_x = f64::from(cell_width) / f64::from(screen.cell_width);
    let scale_y = f64::from(cell_height) / f64::from(screen.cell_height);
    let [x, y, width, height] = [
        source_x as f64 * scale_x,
        source_y as f64 * scale_y,
        f64::from(placement.width) * scale_x,
        f64::from(placement.height) * scale_y,
    ];
    if ![x, y, width, height].into_iter().all(f64::is_finite) || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let grid_width = f64::from(screen.cols) * f64::from(cell_width);
    let grid_height = f64::from(screen.rows) * f64::from(cell_height);
    if x >= grid_width || y >= grid_height || x + width <= 0.0 || y + height <= 0.0 {
        return None;
    }
    let rect = [x as f32, y as f32, width as f32, height as f32];
    rect.into_iter().all(f32::is_finite).then_some(rect)
}

pub(super) fn add_layer(
    mut grid: El,
    screen: &griz_protocol::Screen,
    cell_width: f32,
    cell_height: f32,
    layer: ImageLayer,
) -> El {
    let mut placements: Vec<_> = screen
        .images
        .iter()
        .enumerate()
        .filter(|(_, placement)| placement.layer == layer)
        .collect();
    placements.sort_by_key(|(index, placement)| (placement.z, *index));

    for (index, placement) in placements {
        let Some(asset) = screen
            .image_assets
            .iter()
            .rev()
            .find(|asset| asset.key == placement.key && valid_asset(asset))
        else {
            continue;
        };
        let Some(source) = source_rect(asset, placement) else {
            continue;
        };
        let Some(rect) = geometry(screen, placement, cell_width, cell_height) else {
            continue;
        };
        let Some(pixels) = CACHE.with(|cache| cache.borrow_mut().pixels(asset, source)) else {
            continue;
        };
        grid = grid.child(
            el().key(format!(
                "terminal-image-{}-{}-{}-{}-{index}",
                placement.key.namespace, placement.key.id, placement.key.generation, placement.z
            ))
            .size(rect[2], rect[3])
            .pixels(&pixels)
            .absolute(rect[0], rect[1])
            .inert(),
        );
    }
    grid
}

#[cfg(test)]
pub(crate) fn reset_cache() {
    CACHE.with(|cache| *cache.borrow_mut() = Cache::default());
}

#[cfg(test)]
pub(crate) fn cache_usage() -> (usize, usize) {
    CACHE.with(|cache| {
        let cache = cache.borrow();
        (cache.entries.len(), cache.bytes)
    })
}
