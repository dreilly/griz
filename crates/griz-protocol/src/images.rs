//! Static terminal image metadata. Pixels travel as bounded resource chunks, not in views.
use serde::{Deserialize, Serialize};

pub const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_IMAGE_DIMENSION: u32 = 8192;
pub const MAX_SCREEN_IMAGE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_IMAGE_PLACEMENTS: usize = 256;
pub const IMAGE_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImageKey {
    /// Unique emulator instance; prevents image IDs colliding across terminals or shell restarts.
    pub namespace: u64,
    pub id: u32,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageAsset {
    pub key: ImageKey,
    pub width: u32,
    pub height: u32,
    /// Tightly packed, straight-alpha sRGB RGBA8.
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ImageLayer {
    BelowBackground,
    BelowText,
    AboveText,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImagePlacement {
    pub key: ImageKey,
    pub layer: ImageLayer,
    pub z: i32,
    pub col: i32,
    pub row: i32,
    pub offset_x: u32,
    pub offset_y: u32,
    pub width: u32,
    pub height: u32,
    pub source_x: u32,
    pub source_y: u32,
    pub source_width: u32,
    pub source_height: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageChunk {
    pub key: ImageKey,
    pub width: u32,
    pub height: u32,
    /// Offset in the decoded RGBA resource, not the base64 text.
    pub offset: u32,
    pub data: String,
}
