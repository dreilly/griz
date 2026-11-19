//! Safe, single-threaded wrapper around libghostty-vt 1.0.

#[cfg(test)]
mod image_tests;
mod program_status;
#[cfg(test)]
mod program_status_tests;
#[cfg(test)]
mod scrollback_tests;

use griz_protocol::{
    Cell, ImageAsset, ImageKey, ImageLayer, ImagePlacement, KeyAction, KeyInput, MAX_IMAGE_BYTES,
    MAX_IMAGE_DIMENSION, MAX_IMAGE_PLACEMENTS, MAX_SCREEN_IMAGE_BYTES, MouseAction, MouseButton,
    MouseInput, ProgramStatusRecord, Screen, TerminalKey, UnderlineStyle,
};
use std::collections::{HashMap, HashSet};
use std::error;
use std::ffi::{c_int, c_void};
use std::fmt;
use std::io::Cursor;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;
use std::slice;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

const CELL_BOLD: u8 = 1 << 0;
const CELL_ITALIC: u8 = 1 << 1;
const CELL_FAINT: u8 = 1 << 2;
const CELL_BLINK: u8 = 1 << 3;
const CELL_CONCEALED: u8 = 1 << 4;
const CELL_STRIKETHROUGH: u8 = 1 << 5;
const CELL_OVERLINE: u8 = 1 << 6;
#[cfg(test)]
const OUT_OF_MEMORY: c_int = -1;
const INVALID_VALUE: c_int = -2;
const REJECTED: c_int = -7;
const DEFAULT_CELL_WIDTH: u16 = 8;
const DEFAULT_CELL_HEIGHT: u16 = 16;

static NEXT_IMAGE_NAMESPACE: AtomicU64 = AtomicU64::new(1);
static PNG_DECODER_RESULT: OnceLock<c_int> = OnceLock::new();

/// Default estimated scrollback page allocation per terminal.
///
/// Ghostty prunes complete pages, so actual allocation can exceed this by
/// roughly one page. The physical-line limit may be reached first.
pub const DEFAULT_SCROLLBACK_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Default physical history rows retained per terminal.
///
/// Ghostty prunes complete pages, so actual retained rows can exceed this
/// limit by the rows remaining in an eligible page. The byte limit may be
/// reached first for narrow, wide-glyph, or style-heavy history.
pub const DEFAULT_SCROLLBACK_MAX_LINES: usize = 10_000;

#[derive(Clone, Copy)]
struct ScrollbackLimits {
    max_bytes: usize,
    max_lines: usize,
}

impl Default for ScrollbackLimits {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_SCROLLBACK_MAX_BYTES,
            max_lines: DEFAULT_SCROLLBACK_MAX_LINES,
        }
    }
}

#[repr(C)]
struct GrizVt {
    _private: [u8; 0],
}

#[repr(C)]
struct GrizVtCell {
    text_offset: usize,
    text_len: usize,
    width: u8,
    wide_spacer_head: u8,
    fg: [u8; 3],
    bg: [u8; 3],
    flags: u8,
    underline: u8,
    underline_color: [u8; 3],
}

#[repr(C)]
struct GrizVtImage {
    id: u32,
    generation: u64,
    width: u32,
    height: u32,
    format: u8,
    pixels: *const u8,
    pixels_len: usize,
}

#[repr(C)]
struct GrizVtImagePlacement {
    image_index: usize,
    layer: u8,
    z: i32,
    col: i32,
    row: i32,
    offset_x: u32,
    offset_y: u32,
    width: u32,
    height: u32,
    source_x: u32,
    source_y: u32,
    source_width: u32,
    source_height: u32,
}

#[repr(C)]
struct GrizVtSnapshot {
    cols: u16,
    rows: u16,
    cursor_x: u16,
    cursor_y: u16,
    cursor_visible: u8,
    cursor_shape: u8,
    cursor_blinking: u8,
    cursor_color_has_value: u8,
    cursor_color: [u8; 3],
    application_cursor: u8,
    bracketed_paste: u8,
    mouse_tracking: u8,
    scrollback_offset: u64,
    row_soft_wrapped: *mut u8,
    row_soft_wrapped_len: usize,
    cells: *mut GrizVtCell,
    cells_len: usize,
    text: *mut u8,
    text_len: usize,
    _text_cap: usize,
    title: *mut u8,
    title_len: usize,
    images: *mut GrizVtImage,
    images_len: usize,
    image_placements: *mut GrizVtImagePlacement,
    image_placements_len: usize,
}

#[repr(C)]
struct GhosttyAllocator {
    _private: [u8; 0],
}

#[repr(C)]
struct GhosttySysImage {
    width: u32,
    height: u32,
    data: *mut u8,
    data_len: usize,
}

type DecodePngFn = unsafe extern "C" fn(
    *mut c_void,
    *const GhosttyAllocator,
    *const u8,
    usize,
    *mut GhosttySysImage,
) -> bool;

struct PngAllocation {
    allocator: *const GhosttyAllocator,
    data: *mut u8,
    len: usize,
}

impl Drop for PngAllocation {
    fn drop(&mut self) {
        if !self.data.is_null() {
            // SAFETY: this guard exclusively owns the callback allocation until disarmed.
            unsafe { griz_vt_png_free(self.allocator, self.data, self.len) };
        }
    }
}

unsafe extern "C" {
    fn griz_vt_install_png_decoder(decoder: DecodePngFn) -> c_int;
    fn griz_vt_png_alloc(allocator: *const GhosttyAllocator, len: usize) -> *mut u8;
    fn griz_vt_png_free(allocator: *const GhosttyAllocator, data: *mut u8, len: usize);
    fn griz_vt_new(
        cols: u16,
        rows: u16,
        scrollback_max_bytes: usize,
        scrollback_max_lines: usize,
        out: *mut *mut GrizVt,
        report: extern "C" fn(*mut c_void, *const program_status::RawReport),
        prompt: extern "C" fn(*mut c_void),
        userdata: *mut c_void,
    ) -> c_int;
    fn griz_vt_free(vt: *mut GrizVt);
    fn griz_vt_write(vt: *mut GrizVt, data: *const u8, len: usize);
    fn griz_vt_resize(vt: *mut GrizVt, cols: u16, rows: u16) -> c_int;
    fn griz_vt_set_cell_size(vt: *mut GrizVt, width: u16, height: u16) -> c_int;
    fn griz_vt_render_held(vt: *const GrizVt) -> u8;
    fn griz_vt_end_render_hold(vt: *mut GrizVt) -> c_int;
    fn griz_vt_mouse_input(
        vt: *mut GrizVt,
        action: u8,
        button: i8,
        x: f32,
        y: f32,
        cell_width: u16,
        cell_height: u16,
        shift: u8,
        alt: u8,
        ctrl: u8,
        data: *mut *mut u8,
        len: *mut usize,
    ) -> c_int;
    fn griz_vt_key_input(
        vt: *mut GrizVt,
        action: u8,
        key: u16,
        unshifted_codepoint: u32,
        consumed_shift: u8,
        consumed_alt: u8,
        text: *const u8,
        text_len: usize,
        shift: u8,
        alt: u8,
        ctrl: u8,
        super_key: u8,
        data: *mut *mut u8,
        len: *mut usize,
        kitty_flags: *mut u8,
        modify_other_keys: *mut u8,
    ) -> c_int;
    fn griz_vt_paste(
        vt: *mut GrizVt,
        data: *const u8,
        len: usize,
        allow_unsafe: u8,
        out_data: *mut *mut u8,
        out_len: *mut usize,
        out_pasted: *mut u8,
    ) -> c_int;
    fn griz_vt_set_viewport(vt: *mut GrizVt, position: *mut c_void) -> c_int;
    fn griz_vt_scroll(vt: *mut GrizVt, rows: i32, out: *mut *mut c_void) -> c_int;
    fn griz_vt_viewport_free(position: *mut c_void);
    fn griz_vt_snapshot(vt: *mut GrizVt, out: *mut *mut GrizVtSnapshot) -> c_int;
    fn griz_vt_snapshot_free(snapshot: *mut GrizVtSnapshot);
    fn griz_vt_take_responses(vt: *mut GrizVt, data: *mut *mut u8, len: *mut usize) -> c_int;
    #[cfg(test)]
    fn griz_vt_fail_next_capture_allocation(vt: *mut GrizVt);
    fn griz_vt_bytes_free(data: *mut u8);
}

/// A libghostty-vt operation error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
    operation: &'static str,
    code: c_int,
}

impl Error {
    /// The libghostty-vt result code returned by the failed operation.
    pub fn code(self) -> c_int {
        self.code
    }

    /// The operation that failed.
    pub fn operation(self) -> &'static str {
        self.operation
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libghostty-vt {} failed with result code {}",
            self.operation, self.code
        )
    }
}

impl error::Error for Error {}

/// Result of encoding clipboard text against a terminal's current modes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PasteResult {
    /// Nothing was available to paste (normally an empty string).
    Empty,
    /// Ghostty refused potentially command-injecting text pending confirmation.
    Rejected,
    /// Complete bytes ready for the PTY. Controls are sanitized by Ghostty.
    Written(Vec<u8>),
}

/// One client's scroll position, tracked through output, pruning, and reflow.
/// A default viewport follows live output. It is safe to drop after its terminal.
#[derive(Default)]
pub struct Viewport {
    raw: *mut c_void,
    _single_threaded: PhantomData<Rc<()>>,
}

impl Viewport {
    /// Whether this viewport follows the live terminal area.
    pub fn is_live(&self) -> bool {
        self.raw.is_null()
    }
}

impl Drop for Viewport {
    fn drop(&mut self) {
        // SAFETY: this is an owned tracked reference (or null). Ghostty permits
        // freeing it even after the terminal has been destroyed.
        unsafe { griz_vt_viewport_free(self.raw) };
    }
}

/// A terminal emulator backed by libghostty-vt.
///
/// The terminal is intentionally not `Send` or `Sync`; use it from its owning
/// actor thread.
pub struct Terminal {
    raw: NonNull<GrizVt>,
    statuses: Box<program_status::Records>,
    image_namespace: u64,
    image_cache: HashMap<ImageKey, Arc<ImageAsset>>,
    cell_width: u16,
    cell_height: u16,
    _single_threaded: PhantomData<Rc<()>>,
}

impl Terminal {
    /// Creates a terminal with the given cell dimensions and Griz's bounded
    /// per-terminal scrollback defaults.
    pub fn new(cols: u16, rows: u16) -> Result<Self, Error> {
        Self::new_with_scrollback_limits(cols, rows, ScrollbackLimits::default())
    }

    fn new_with_scrollback_limits(
        cols: u16,
        rows: u16,
        limits: ScrollbackLimits,
    ) -> Result<Self, Error> {
        install_png_decoder()?;
        let mut raw = std::ptr::null_mut();
        let mut statuses = Box::<program_status::Records>::default();
        // SAFETY: out is valid; userdata is a stable Box kept alive until after
        // the terminal is freed. Callbacks run synchronously during VT writes.
        check("create", unsafe {
            griz_vt_new(
                cols,
                rows,
                limits.max_bytes,
                limits.max_lines,
                &mut raw,
                program_status::report,
                program_status::prompt,
                (&mut *statuses as *mut program_status::Records).cast(),
            )
        })?;
        let raw = NonNull::new(raw).ok_or(Error {
            operation: "create",
            code: INVALID_VALUE,
        })?;
        Ok(Self {
            raw,
            statuses,
            image_namespace: NEXT_IMAGE_NAMESPACE.fetch_add(1, Ordering::Relaxed),
            image_cache: HashMap::new(),
            cell_width: DEFAULT_CELL_WIDTH,
            cell_height: DEFAULT_CELL_HEIGHT,
            _single_threaded: PhantomData,
        })
    }

    /// Feeds arbitrary VT bytes into the terminal.
    pub fn write(&mut self, bytes: &[u8]) {
        // SAFETY: the terminal handle is live and `bytes` remains valid for the
        // duration of this synchronous call.
        unsafe { griz_vt_write(self.raw.as_ptr(), bytes.as_ptr(), bytes.len()) }
    }

    /// Current OSC 7501 records, least recently updated first. Missing fields
    /// stay missing; app inheritance is resolved by the presentation layer.
    pub fn program_status(&self) -> &[ProgramStatusRecord] {
        &self.statuses.records
    }

    pub fn take_program_status_changed(&mut self) -> bool {
        self.statuses.take_changed()
    }

    /// Drop transient reports on PTY process exit; retain done/error results.
    pub fn program_exited(&mut self) {
        self.statuses.expire_transient();
    }

    /// Resizes the terminal grid. Primary-screen wrapped content may reflow.
    /// Ghostty also ends any synchronized-output render hold on every resize.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Error> {
        // SAFETY: the terminal handle is live and the scalar arguments have the
        // exact types expected by the shim. The shim retains the configured pixels.
        check("resize", unsafe {
            griz_vt_resize(self.raw.as_ptr(), cols, rows)
        })
    }

    /// Sets authoritative cell pixel geometry without changing the terminal grid.
    /// Future grid resizes retain these dimensions. Width and height must be nonzero.
    pub fn set_cell_size(&mut self, width: u16, height: u16) -> Result<(), Error> {
        // SAFETY: the terminal handle is live and the shim validates scalar dimensions.
        check("set cell size", unsafe {
            griz_vt_set_cell_size(self.raw.as_ptr(), width, height)
        })?;
        self.cell_width = width;
        self.cell_height = height;
        Ok(())
    }

    /// Whether Ghostty has an active synchronized-output render hold.
    pub fn render_held(&self) -> bool {
        // SAFETY: the terminal handle is live and the shim returns a copied boolean.
        unsafe { griz_vt_render_held(self.raw.as_ptr()) != 0 }
    }

    /// Ends a stalled synchronized-output hold without resetting terminal contents.
    /// Ghostty's host-side mode setter intentionally does not invoke the hold callback,
    /// so the shim updates its callback-derived boolean after a successful change.
    pub fn end_render_hold(&mut self) -> Result<(), Error> {
        // SAFETY: the terminal handle is live and uniquely owned by this wrapper.
        check("end render hold", unsafe {
            griz_vt_end_render_hold(self.raw.as_ptr())
        })
    }

    /// Encodes a pixel-positioned mouse event using the terminal's active
    /// tracking and output-format modes. An event ignored by those modes
    /// returns an empty byte vector.
    pub fn mouse_input(&mut self, event: &MouseInput) -> Result<Vec<u8>, Error> {
        let action = match event.action {
            MouseAction::Press => 0,
            MouseAction::Release => 1,
            MouseAction::Motion => 2,
        };
        let button = match event.button {
            None => -1,
            Some(MouseButton::Left) => 0,
            Some(MouseButton::Right) => 1,
            Some(MouseButton::Middle) => 2,
            Some(MouseButton::WheelUp) => 3,
            Some(MouseButton::WheelDown) => 4,
        };
        let mut data = std::ptr::null_mut();
        let mut len = 0;
        // SAFETY: the terminal handle is live, scalar values are copied, and
        // successful output is transferred to the caller-owned C allocation.
        check("encode mouse input", unsafe {
            griz_vt_mouse_input(
                self.raw.as_ptr(),
                action,
                button,
                event.x,
                event.y,
                event.cell_width,
                event.cell_height,
                event.shift.into(),
                event.alt.into(),
                event.ctrl.into(),
                &mut data,
                &mut len,
            )
        })?;
        let bytes = if len == 0 {
            Vec::new()
        } else {
            // SAFETY: the shim returns `len` initialized bytes on success.
            unsafe { slice::from_raw_parts(data, len) }.to_vec()
        };
        // SAFETY: the pointer was allocated by the shim and may be null.
        unsafe { griz_vt_bytes_free(data) };
        Ok(bytes)
    }

    /// Encodes one native press/repeat using this terminal's live keyboard modes.
    /// Rune's logical handler does not provide releases, so callers must not synthesize them.
    pub fn key_input(&mut self, event: &KeyInput) -> Result<Vec<u8>, Error> {
        event.validate().map_err(|_| Error {
            operation: "validate key input",
            code: INVALID_VALUE,
        })?;
        let key = event
            .physical
            .map_or_else(|| logical_key(event), |physical| physical.value());
        let consumed_alt = cfg!(target_os = "macos")
            && event.modifiers.alt
            && !event.modifiers.ctrl
            && !event.text.is_ascii();
        // Rune does not expose the platform's consumed-modifier mask. A lone Shift on a
        // character key is nevertheless unambiguous: it produced the supplied text and must
        // not turn `:` into a Kitty `Shift+;` shortcut. Option-generated Unicode is likewise
        // text on macOS, so Shift may be consumed alongside that already-consumed Option.
        let consumed_shift = event.key == TerminalKey::Character
            && event.modifiers.shift
            && !event.modifiers.ctrl
            && !event.modifiers.super_key
            && (!event.modifiers.alt || consumed_alt);
        let mut data = std::ptr::null_mut();
        let mut len = 0;
        let mut kitty_flags = 0;
        let mut modify_other_keys = 0;
        // SAFETY: the terminal handle is live; event text remains valid for this synchronous
        // call; successful output is transferred to the caller-owned C allocation.
        check("encode key input", unsafe {
            griz_vt_key_input(
                self.raw.as_ptr(),
                match event.action {
                    KeyAction::Press => 0,
                    KeyAction::Repeat => 1,
                },
                key,
                unshifted_codepoint(event),
                consumed_shift.into(),
                consumed_alt.into(),
                event.text.as_ptr(),
                event.text.len(),
                event.modifiers.shift.into(),
                event.modifiers.alt.into(),
                event.modifiers.ctrl.into(),
                event.modifiers.super_key.into(),
                &mut data,
                &mut len,
                &mut kitty_flags,
                &mut modify_other_keys,
            )
        })?;
        let bytes = copy_c_bytes(data, len);

        // Ghostty owns every negotiated encoding and distinct keypad identity. Retain Griz's
        // historical Shift+Enter CSI-u only for an unnegotiated main Return key.
        if event.key == TerminalKey::Enter
            && event.physical.is_none_or(|physical| physical.value() == 58)
            && event.modifiers.shift
            && kitty_flags == 0
            && modify_other_keys == 0
        {
            let modifier = 1
                + u8::from(event.modifiers.shift)
                + 2 * u8::from(event.modifiers.alt)
                + 4 * u8::from(event.modifiers.ctrl)
                + 8 * u8::from(event.modifiers.super_key);
            return Ok(format!("\x1b[13;{modifier}u").into_bytes());
        }
        Ok(bytes)
    }

    /// Scroll one client's viewport by rows (negative is up).
    pub fn scroll(&mut self, viewport: &mut Viewport, rows: i32) -> Result<(), Error> {
        // SAFETY: both handles are live; viewport references remain safe even
        // after their original terminal is freed.
        check("restore viewport", unsafe {
            griz_vt_set_viewport(self.raw.as_ptr(), viewport.raw)
        })?;
        let mut raw = std::ptr::null_mut();
        // SAFETY: the terminal and output pointer are valid.
        let result = check("scroll viewport", unsafe {
            griz_vt_scroll(self.raw.as_ptr(), rows, &mut raw)
        });
        // Always restore the live area, including on capture/allocation failure.
        // SAFETY: null requests the live viewport on this terminal.
        let restore = check("restore live viewport", unsafe {
            griz_vt_set_viewport(self.raw.as_ptr(), std::ptr::null_mut())
        });
        result?;
        *viewport = Viewport {
            raw,
            _single_threaded: PhantomData,
        };
        restore
    }

    /// Capture one client's history without changing any other client's view.
    pub fn screen_at(&mut self, viewport: &Viewport) -> Result<Screen, Error> {
        // SAFETY: the terminal and tracked reference are valid.
        check("restore viewport", unsafe {
            griz_vt_set_viewport(self.raw.as_ptr(), viewport.raw)
        })?;
        let result = self.screen();
        // SAFETY: null requests the live viewport on this terminal.
        let restore = check("restore live viewport", unsafe {
            griz_vt_set_viewport(self.raw.as_ptr(), std::ptr::null_mut())
        });
        let screen = result?;
        restore?;
        Ok(screen)
    }

    /// Captures the current visible viewport through Ghostty's RenderState API.
    pub fn screen(&mut self) -> Result<Screen, Error> {
        let mut raw_snapshot = std::ptr::null_mut();
        // SAFETY: the terminal handle and out pointer are valid.
        check("render snapshot", unsafe {
            griz_vt_snapshot(self.raw.as_ptr(), &mut raw_snapshot)
        })?;
        let raw_snapshot = NonNull::new(raw_snapshot).ok_or(Error {
            operation: "render snapshot",
            code: INVALID_VALUE,
        })?;
        let snapshot = Snapshot(raw_snapshot);
        // SAFETY: Snapshot owns a live C allocation until this function exits, and
        // the borrowed Ghostty pixel views remain valid until the next VT mutation.
        unsafe {
            snapshot.to_screen(
                self.image_namespace,
                &mut self.image_cache,
                self.cell_width,
                self.cell_height,
            )
        }
    }

    /// Encodes clipboard text using live terminal modes. Errors and rejections return no bytes.
    pub fn paste(&mut self, text: &str, allow_unsafe: bool) -> Result<PasteResult, Error> {
        let mut data = std::ptr::null_mut();
        let mut len = 0;
        let mut pasted = 0;
        // SAFETY: all pointers remain valid for this synchronous call; successful output is
        // transferred to the caller-owned C allocation.
        let code = unsafe {
            griz_vt_paste(
                self.raw.as_ptr(),
                text.as_ptr(),
                text.len(),
                allow_unsafe.into(),
                &mut data,
                &mut len,
                &mut pasted,
            )
        };
        if code == REJECTED {
            return Ok(PasteResult::Rejected);
        }
        check("encode paste", code)?;
        let bytes = copy_c_bytes(data, len);
        if pasted == 0 {
            if !bytes.is_empty() {
                return Err(Error {
                    operation: "encode paste",
                    code: INVALID_VALUE,
                });
            }
            Ok(PasteResult::Empty)
        } else {
            Ok(PasteResult::Written(bytes))
        }
    }

    /// Returns and clears bytes generated in reply to terminal queries.
    pub fn take_responses(&mut self) -> Result<Vec<u8>, Error> {
        let mut data = std::ptr::null_mut();
        let mut len = 0;
        // SAFETY: all pointers are valid for this synchronous ownership transfer.
        check("capture terminal responses", unsafe {
            griz_vt_take_responses(self.raw.as_ptr(), &mut data, &mut len)
        })?;
        Ok(copy_c_bytes(data, len))
    }

    #[cfg(test)]
    fn fail_next_capture_allocation(&mut self) {
        // SAFETY: the terminal handle is live; this only arms a deterministic shim test hook.
        unsafe { griz_vt_fail_next_capture_allocation(self.raw.as_ptr()) }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // SAFETY: this is the unique owned handle and it is freed exactly once.
        unsafe { griz_vt_free(self.raw.as_ptr()) }
    }
}

struct Snapshot(NonNull<GrizVtSnapshot>);

impl Snapshot {
    unsafe fn to_screen(
        &self,
        namespace: u64,
        image_cache: &mut HashMap<ImageKey, Arc<ImageAsset>>,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<Screen, Error> {
        // SAFETY: guaranteed by the caller and the lifetime of `self`.
        let snapshot = unsafe { self.0.as_ref() };
        let expected_cells = usize::from(snapshot.cols) * usize::from(snapshot.rows);
        if snapshot.cells_len != expected_cells
            || snapshot.row_soft_wrapped_len != usize::from(snapshot.rows)
            || (snapshot.row_soft_wrapped_len != 0 && snapshot.row_soft_wrapped.is_null())
            || (snapshot.cells_len != 0 && snapshot.cells.is_null())
            || (snapshot.text_len != 0 && snapshot.text.is_null())
            || (snapshot.title_len != 0 && snapshot.title.is_null())
            || (snapshot.images_len != 0 && snapshot.images.is_null())
            || (snapshot.image_placements_len != 0 && snapshot.image_placements.is_null())
            || snapshot.image_placements_len > MAX_IMAGE_PLACEMENTS
        {
            return Err(Error {
                operation: "decode render snapshot",
                code: INVALID_VALUE,
            });
        }

        let row_soft_wrapped = if snapshot.row_soft_wrapped_len == 0 {
            Vec::new()
        } else {
            // SAFETY: validated above; the C snapshot owns one byte per visible row.
            let raw = unsafe {
                slice::from_raw_parts(snapshot.row_soft_wrapped, snapshot.row_soft_wrapped_len)
            };
            raw.iter().map(|wrapped| *wrapped != 0).collect()
        };
        let raw_cells = if snapshot.cells_len == 0 {
            &[][..]
        } else {
            // SAFETY: validated above; the C snapshot owns this complete array.
            unsafe { slice::from_raw_parts(snapshot.cells, snapshot.cells_len) }
        };
        let text = if snapshot.text_len == 0 {
            &[][..]
        } else {
            // SAFETY: validated above; the C snapshot owns this complete buffer.
            unsafe { slice::from_raw_parts(snapshot.text, snapshot.text_len) }
        };
        let title = if snapshot.title_len == 0 {
            &[][..]
        } else {
            // SAFETY: validated above; the C snapshot owns this complete buffer.
            unsafe { slice::from_raw_parts(snapshot.title, snapshot.title_len) }
        };
        let raw_images = if snapshot.images_len == 0 {
            &[][..]
        } else {
            // SAFETY: validated above; descriptors borrow pixels owned by the live terminal.
            unsafe { slice::from_raw_parts(snapshot.images, snapshot.images_len) }
        };
        let raw_placements = if snapshot.image_placements_len == 0 {
            &[][..]
        } else {
            // SAFETY: validated above; the C snapshot owns this complete array.
            unsafe {
                slice::from_raw_parts(snapshot.image_placements, snapshot.image_placements_len)
            }
        };

        let mut referenced_indices = HashSet::new();
        for placement in raw_placements {
            if placement.image_index >= raw_images.len() {
                return Err(Error {
                    operation: "decode render snapshot",
                    code: INVALID_VALUE,
                });
            }
            referenced_indices.insert(placement.image_index);
        }

        let mut assets_by_index = HashMap::new();
        let mut image_assets = Vec::new();
        let mut screen_image_bytes = 0usize;
        for (index, image) in raw_images.iter().enumerate() {
            if !referenced_indices.contains(&index) {
                continue;
            }
            let key = ImageKey {
                namespace,
                id: image.id,
                generation: image.generation,
            };
            let Some(rgba_len) = image_rgba_len(image.width, image.height) else {
                continue;
            };
            let Some(new_total) = screen_image_bytes.checked_add(rgba_len) else {
                continue;
            };
            if new_total > MAX_SCREEN_IMAGE_BYTES {
                continue;
            }

            let asset = if let Some(cached) = image_cache.get(&key).filter(|cached| {
                cached.width == image.width
                    && cached.height == image.height
                    && cached.rgba.len() == rgba_len
            }) {
                Arc::clone(cached)
            } else {
                let Some(rgba) = (unsafe { copy_image_rgba(image, rgba_len) }) else {
                    continue;
                };
                let asset = Arc::new(ImageAsset {
                    key,
                    width: image.width,
                    height: image.height,
                    rgba,
                });
                image_cache.insert(key, Arc::clone(&asset));
                asset
            };
            screen_image_bytes = new_total;
            assets_by_index.insert(index, Arc::clone(&asset));
            image_assets.push(asset);
        }

        let mut images = Vec::with_capacity(raw_placements.len());
        for placement in raw_placements {
            let Some(asset) = assets_by_index.get(&placement.image_index) else {
                continue;
            };
            let Some(layer) = image_layer(placement.layer) else {
                continue;
            };
            let source_right = placement.source_x.checked_add(placement.source_width);
            let source_bottom = placement.source_y.checked_add(placement.source_height);
            if placement.width == 0
                || placement.height == 0
                || placement.source_width == 0
                || placement.source_height == 0
                || source_right.is_none_or(|right| right > asset.width)
                || source_bottom.is_none_or(|bottom| bottom > asset.height)
            {
                continue;
            }
            images.push(ImagePlacement {
                key: asset.key,
                layer,
                z: placement.z,
                col: placement.col,
                row: placement.row,
                offset_x: placement.offset_x,
                offset_y: placement.offset_y,
                width: placement.width,
                height: placement.height,
                source_x: placement.source_x,
                source_y: placement.source_y,
                source_width: placement.source_width,
                source_height: placement.source_height,
            });
        }
        images.sort_by_key(|placement| (placement.layer, placement.z));
        let visible_keys: HashSet<_> = images.iter().map(|placement| placement.key).collect();
        image_assets.retain(|asset| visible_keys.contains(&asset.key));
        image_cache.retain(|key, _| visible_keys.contains(key));

        let mut cells = Vec::with_capacity(usize::from(snapshot.rows));
        for row in raw_cells.chunks_exact(usize::from(snapshot.cols)) {
            let mut decoded_row = Vec::with_capacity(usize::from(snapshot.cols));
            for cell in row {
                let end = cell.text_offset.checked_add(cell.text_len).ok_or(Error {
                    operation: "decode render snapshot",
                    code: INVALID_VALUE,
                })?;
                let cell_text = text.get(cell.text_offset..end).ok_or(Error {
                    operation: "decode render snapshot",
                    code: INVALID_VALUE,
                })?;
                decoded_row.push(Cell {
                    text: String::from_utf8_lossy(cell_text).into_owned(),
                    width: cell.width,
                    wide_spacer_head: cell.wide_spacer_head != 0,
                    fg: cell.fg,
                    bg: cell.bg,
                    bold: cell.flags & CELL_BOLD != 0,
                    italic: cell.flags & CELL_ITALIC != 0,
                    faint: cell.flags & CELL_FAINT != 0,
                    blink: cell.flags & CELL_BLINK != 0,
                    concealed: cell.flags & CELL_CONCEALED != 0,
                    strikethrough: cell.flags & CELL_STRIKETHROUGH != 0,
                    overline: cell.flags & CELL_OVERLINE != 0,
                    underline: match cell.underline {
                        1 => UnderlineStyle::Single,
                        2 => UnderlineStyle::Double,
                        3 => UnderlineStyle::Curly,
                        4 => UnderlineStyle::Dotted,
                        5 => UnderlineStyle::Dashed,
                        _ => UnderlineStyle::None,
                    },
                    underline_color: cell.underline_color,
                });
            }
            cells.push(decoded_row);
        }

        Ok(Screen {
            cols: snapshot.cols,
            rows: snapshot.rows,
            cells,
            row_soft_wrapped,
            cursor_x: snapshot.cursor_x,
            cursor_y: snapshot.cursor_y,
            cursor_visible: snapshot.cursor_visible != 0,
            cursor_shape: match snapshot.cursor_shape {
                1 => griz_protocol::CursorShape::Bar,
                2 => griz_protocol::CursorShape::Underline,
                3 => griz_protocol::CursorShape::HollowBlock,
                _ => griz_protocol::CursorShape::Block,
            },
            cursor_blinking: snapshot.cursor_blinking != 0,
            cursor_color: (snapshot.cursor_color_has_value != 0).then_some(snapshot.cursor_color),
            title: String::from_utf8_lossy(title).into_owned(),
            application_cursor: snapshot.application_cursor != 0,
            bracketed_paste: snapshot.bracketed_paste != 0,
            mouse_tracking: snapshot.mouse_tracking != 0,
            scrollback_offset: snapshot.scrollback_offset,
            cell_width,
            cell_height,
            images,
            image_assets,
        })
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        // SAFETY: this is the unique owned snapshot and it is freed exactly once.
        unsafe { griz_vt_snapshot_free(self.0.as_ptr()) }
    }
}

fn image_rgba_len(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 || width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return None;
    }
    usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(4)
        .filter(|len| *len <= MAX_IMAGE_BYTES)
}

fn image_layer(raw: u8) -> Option<ImageLayer> {
    match raw {
        0 => Some(ImageLayer::BelowBackground),
        1 => Some(ImageLayer::BelowText),
        2 => Some(ImageLayer::AboveText),
        _ => None,
    }
}

unsafe fn copy_image_rgba(image: &GrizVtImage, rgba_len: usize) -> Option<Vec<u8>> {
    let bytes_per_pixel = match image.format {
        0 => 3,
        1 => 4,
        3 => 2,
        4 => 1,
        _ => return None,
    };
    let pixels = usize::try_from(image.width)
        .ok()?
        .checked_mul(usize::try_from(image.height).ok()?)?;
    let source_len = pixels.checked_mul(bytes_per_pixel)?;
    if source_len != image.pixels_len || (source_len != 0 && image.pixels.is_null()) {
        return None;
    }
    // SAFETY: the C snapshot validated this borrowed terminal-owned view and no VT mutation can
    // occur while screen conversion holds &mut Terminal.
    let source = unsafe { slice::from_raw_parts(image.pixels, image.pixels_len) };
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(rgba_len).ok()?;
    match image.format {
        0 => {
            for rgb in source.as_chunks::<3>().0 {
                rgba.extend_from_slice(rgb);
                rgba.push(255);
            }
        }
        1 => rgba.extend_from_slice(source),
        3 => {
            for gray_alpha in source.as_chunks::<2>().0 {
                rgba.extend_from_slice(&[
                    gray_alpha[0],
                    gray_alpha[0],
                    gray_alpha[0],
                    gray_alpha[1],
                ]);
            }
        }
        4 => {
            for gray in source {
                rgba.extend_from_slice(&[*gray, *gray, *gray, 255]);
            }
        }
        _ => return None,
    }
    (rgba.len() == rgba_len).then_some(rgba)
}

fn install_png_decoder() -> Result<(), Error> {
    let code = *PNG_DECODER_RESULT.get_or_init(|| {
        // SAFETY: process-global installation is serialized by OnceLock and happens before this
        // crate creates a terminal. The callback has the exact C ABI expected by Ghostty.
        unsafe { griz_vt_install_png_decoder(decode_png) }
    });
    check("install PNG decoder", code)
}

unsafe extern "C" fn decode_png(
    _userdata: *mut c_void,
    allocator: *const GhosttyAllocator,
    data: *const u8,
    data_len: usize,
    out: *mut GhosttySysImage,
) -> bool {
    std::panic::catch_unwind(|| {
        // SAFETY: Ghostty guarantees callback arguments are borrowed for this call.
        unsafe { decode_png_inner(allocator, data, data_len, out) }
    })
    .unwrap_or(false)
}

unsafe fn decode_png_inner(
    allocator: *const GhosttyAllocator,
    data: *const u8,
    data_len: usize,
    out: *mut GhosttySysImage,
) -> bool {
    if out.is_null()
        || data_len == 0
        || data_len > MAX_IMAGE_BYTES
        || (data_len != 0 && data.is_null())
    {
        return false;
    }
    // SAFETY: validated above; Ghostty keeps the encoded bytes alive for this callback.
    let encoded = unsafe { slice::from_raw_parts(data, data_len) };
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(encoded),
        png::Limits {
            bytes: MAX_IMAGE_BYTES,
        },
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    let Ok(mut reader) = decoder.read_info() else {
        return false;
    };
    if reader.info().animation_control.is_some() {
        return false;
    }
    let width = reader.info().width;
    let height = reader.info().height;
    let Some(rgba_len) = image_rgba_len(width, height) else {
        return false;
    };
    let Some(decoded_len) = reader.output_buffer_size() else {
        return false;
    };
    if decoded_len == 0 || decoded_len > MAX_IMAGE_BYTES {
        return false;
    }
    let mut decoded = Vec::new();
    if decoded.try_reserve_exact(decoded_len).is_err() {
        return false;
    }
    decoded.resize(decoded_len, 0);
    let Ok(info) = reader.next_frame(&mut decoded) else {
        return false;
    };
    if info.width != width || info.height != height || info.bit_depth != png::BitDepth::Eight {
        return false;
    }
    let source_len = info.buffer_size();
    let samples = info.color_type.samples();
    let Some(expected_source_len) = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(usize::try_from(height).ok()?))
        .and_then(|pixels| pixels.checked_mul(samples))
    else {
        return false;
    };
    if source_len != expected_source_len || source_len > decoded.len() {
        return false;
    }

    // SAFETY: the allocator is valid for this callback and Ghostty takes ownership on success.
    let pixels = unsafe { griz_vt_png_alloc(allocator, rgba_len) };
    if pixels.is_null() {
        return false;
    }
    let mut allocation = PngAllocation {
        allocator,
        data: pixels,
        len: rgba_len,
    };
    // SAFETY: Ghostty allocated exactly rgba_len writable bytes above.
    let destination = unsafe { slice::from_raw_parts_mut(allocation.data, rgba_len) };
    if !convert_png_to_rgba(&decoded[..source_len], info.color_type, destination) {
        return false;
    }

    // SAFETY: out is validated and Ghostty zeroed it before invoking the callback.
    unsafe {
        *out = GhosttySysImage {
            width,
            height,
            data: allocation.data,
            data_len: rgba_len,
        };
    }
    allocation.data = std::ptr::null_mut();
    true
}

fn convert_png_to_rgba(source: &[u8], color: png::ColorType, destination: &mut [u8]) -> bool {
    let pixels = destination.len() / 4;
    if !destination.len().is_multiple_of(4)
        || source.len() != pixels.saturating_mul(color.samples())
    {
        return false;
    }
    match color {
        png::ColorType::Rgba => destination.copy_from_slice(source),
        png::ColorType::Rgb => {
            for (rgb, rgba) in source
                .as_chunks::<3>()
                .0
                .iter()
                .zip(destination.as_chunks_mut::<4>().0)
            {
                rgba.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for (gray_alpha, rgba) in source
                .as_chunks::<2>()
                .0
                .iter()
                .zip(destination.as_chunks_mut::<4>().0)
            {
                rgba.copy_from_slice(&[gray_alpha[0], gray_alpha[0], gray_alpha[0], gray_alpha[1]]);
            }
        }
        png::ColorType::Grayscale => {
            for (gray, rgba) in source.iter().zip(destination.as_chunks_mut::<4>().0) {
                rgba.copy_from_slice(&[*gray, *gray, *gray, 255]);
            }
        }
        png::ColorType::Indexed => return false,
    }
    true
}

fn logical_key(event: &KeyInput) -> u16 {
    match event.key {
        TerminalKey::Character => character_key(&event.text),
        TerminalKey::Enter => 58,
        TerminalKey::Backspace => 53,
        TerminalKey::Delete => 68,
        TerminalKey::Left => 76,
        TerminalKey::Right => 77,
        TerminalKey::Up => 78,
        TerminalKey::Down => 75,
        TerminalKey::Home => 71,
        TerminalKey::End => 69,
        TerminalKey::PageUp => 74,
        TerminalKey::PageDown => 73,
        TerminalKey::Insert => 72,
        TerminalKey::PrintScreen => 148,
        TerminalKey::ScrollLock => 149,
        TerminalKey::Pause => 150,
        TerminalKey::Escape => 120,
        TerminalKey::Tab => 64,
        TerminalKey::Function { number } => 120 + u16::from(number),
        TerminalKey::ContextMenu => 55,
    }
}

fn unshifted_codepoint(event: &KeyInput) -> u32 {
    if let Some(character) = event.unshifted_codepoint {
        return u32::from(character);
    }
    // Older clients and logical-only/scripted events have no layout metadata. Use a
    // single logical character, never a guessed US physical-key character. Without
    // this field Ghostty's Kitty encoder treats even Ctrl+V as ordinary text.
    let mut chars = event.text.chars();
    match (chars.next(), chars.next()) {
        (Some(character), None) => u32::from(character.to_ascii_lowercase()),
        _ => 0,
    }
}

fn character_key(text: &str) -> u16 {
    let mut chars = text.chars();
    let Some(character) = chars.next() else {
        return 0;
    };
    if chars.next().is_some() {
        return 0;
    }
    match character.to_ascii_lowercase() {
        '`' | '~' => 1,
        '\\' | '|' => 2,
        '[' | '{' => 3,
        ']' | '}' => 4,
        ',' | '<' => 5,
        '0' | ')' => 6,
        '1' | '!' => 7,
        '2' | '@' => 8,
        '3' | '#' => 9,
        '4' | '$' => 10,
        '5' | '%' => 11,
        '6' | '^' => 12,
        '7' | '&' => 13,
        '8' | '*' => 14,
        '9' | '(' => 15,
        '=' | '+' => 16,
        'a'..='z' => 20 + (character.to_ascii_lowercase() as u16 - 'a' as u16),
        '-' | '_' => 46,
        '.' | '>' => 47,
        '\'' | '"' => 48,
        ';' | ':' => 49,
        '/' | '?' => 50,
        ' ' => 63,
        _ => 0,
    }
}

fn copy_c_bytes(data: *mut u8, len: usize) -> Vec<u8> {
    let bytes = if len == 0 {
        Vec::new()
    } else {
        // SAFETY: successful shim calls return `len` initialized bytes.
        unsafe { slice::from_raw_parts(data, len) }.to_vec()
    };
    // SAFETY: the pointer was allocated by the shim and may be null.
    unsafe { griz_vt_bytes_free(data) };
    bytes
}

fn check(operation: &'static str, code: c_int) -> Result<(), Error> {
    if code == 0 {
        Ok(())
    } else {
        Err(Error { operation, code })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use griz_protocol::{KeyModifiers, PhysicalKey};

    fn key(key: TerminalKey, text: &str, modifiers: KeyModifiers) -> KeyInput {
        KeyInput {
            action: KeyAction::Press,
            key,
            physical: None,
            unshifted_codepoint: None,
            text: text.into(),
            modifiers,
        }
    }

    #[test]
    fn key_encoder_uses_live_modes_modifiers_functions_unicode_and_alt_text() {
        let mut terminal = Terminal::new(80, 24).unwrap();
        assert_eq!(
            terminal
                .key_input(&key(TerminalKey::Up, "", KeyModifiers::default()))
                .unwrap(),
            b"\x1b[A"
        );
        terminal.write(b"\x1b[?1h");
        assert_eq!(
            terminal
                .key_input(&key(TerminalKey::Up, "", KeyModifiers::default()))
                .unwrap(),
            b"\x1bOA"
        );
        assert_eq!(
            terminal
                .key_input(&key(
                    TerminalKey::Up,
                    "",
                    KeyModifiers {
                        shift: true,
                        ..KeyModifiers::default()
                    },
                ))
                .unwrap(),
            b"\x1b[1;2A"
        );

        assert_eq!(
            terminal
                .key_input(&key(
                    TerminalKey::Function { number: 5 },
                    "",
                    KeyModifiers {
                        ctrl: true,
                        ..KeyModifiers::default()
                    },
                ))
                .unwrap(),
            b"\x1b[15;5~"
        );
        assert_eq!(
            terminal
                .key_input(&key(TerminalKey::Character, "界", KeyModifiers::default()))
                .unwrap(),
            "界".as_bytes()
        );
        assert_eq!(
            terminal
                .key_input(&key(
                    TerminalKey::Character,
                    "å",
                    KeyModifiers {
                        alt: true,
                        ..KeyModifiers::default()
                    },
                ))
                .unwrap(),
            "å".as_bytes(),
            "macOS Option-generated Unicode must not gain an ESC prefix"
        );
    }

    #[test]
    fn ctrl_v_reaches_pi_in_legacy_and_kitty_modes() {
        for (mode, expected) in [
            (b"".as_slice(), b"\x16".as_slice()),
            (b"\x1b[>4;2m".as_slice(), b"\x1b[27;5;118~".as_slice()),
            (b"\x1b[>7u".as_slice(), b"\x1b[118;5u".as_slice()),
        ] {
            for (physical, unshifted) in [
                (None, None),
                (PhysicalKey::new(41), Some('v')),
                // Layout identity, not the US character at this physical location.
                (PhysicalKey::new(47), Some('v')),
            ] {
                let mut terminal = Terminal::new(80, 24).unwrap();
                terminal.write(mode);
                let mut event = key(
                    TerminalKey::Character,
                    "v",
                    KeyModifiers {
                        ctrl: true,
                        ..KeyModifiers::default()
                    },
                );
                event.physical = physical;
                event.unshifted_codepoint = unshifted;
                let expected = if physical == PhysicalKey::new(47) && mode == b"\x1b[>7u" {
                    b"\x1b[118::46;5u".as_slice()
                } else {
                    expected
                };
                assert_eq!(
                    terminal.key_input(&event).unwrap(),
                    expected,
                    "physical={physical:?}, mode={mode:?}"
                );
            }
        }
    }

    #[test]
    fn kitty_character_modifiers_repeats_and_reused_named_events() {
        let mut terminal = Terminal::new(80, 24).unwrap();
        terminal.write(b"\x1b[>7u");
        for character in ['a', 'c', 'v', 'w'] {
            let mut event = key(
                TerminalKey::Character,
                &character.to_string(),
                KeyModifiers {
                    ctrl: true,
                    ..KeyModifiers::default()
                },
            );
            event.unshifted_codepoint = Some(character);
            assert_eq!(
                terminal.key_input(&event).unwrap(),
                format!("\x1b[{};5u", u32::from(character)).as_bytes()
            );
            event.action = KeyAction::Repeat;
            assert_eq!(
                terminal.key_input(&event).unwrap(),
                format!("\x1b[{};5:2u", u32::from(character)).as_bytes()
            );
        }
        let mut shifted = key(
            TerminalKey::Character,
            "V",
            KeyModifiers {
                shift: true,
                ctrl: true,
                ..KeyModifiers::default()
            },
        );
        shifted.unshifted_codepoint = Some('v');
        assert_eq!(terminal.key_input(&shifted).unwrap(), b"\x1b[118:86;6u");
        assert_eq!(
            terminal
                .key_input(&key(TerminalKey::Up, "", KeyModifiers::default()))
                .unwrap(),
            b"\x1b[1;1:1A"
        );
        assert_eq!(
            terminal
                .key_input(&key(TerminalKey::Character, "v", KeyModifiers::default()))
                .unwrap(),
            b"v"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn option_unicode_with_native_base_remains_text() {
        for mode in [b"".as_slice(), b"\x1b[>7u".as_slice()] {
            let mut terminal = Terminal::new(80, 24).unwrap();
            terminal.write(mode);
            let mut event = key(
                TerminalKey::Character,
                "å",
                KeyModifiers {
                    alt: true,
                    ..KeyModifiers::default()
                },
            );
            event.physical = PhysicalKey::new(20);
            event.unshifted_codepoint = Some('a');
            assert_eq!(terminal.key_input(&event).unwrap(), "å".as_bytes());
        }
    }

    #[test]
    fn key_encoder_honors_backarrow_keypad_modify_other_keys_and_shift_enter_fallback() {
        let mut terminal = Terminal::new(80, 24).unwrap();
        assert_eq!(
            terminal
                .key_input(&key(TerminalKey::Backspace, "", KeyModifiers::default()))
                .unwrap(),
            [0x7f]
        );
        terminal.write(b"\x1b[?67h");
        assert_eq!(
            terminal
                .key_input(&key(TerminalKey::Backspace, "", KeyModifiers::default()))
                .unwrap(),
            [0x08]
        );

        let mut keypad = key(TerminalKey::Character, "1", KeyModifiers::default());
        keypad.physical = PhysicalKey::new(81);
        assert_eq!(terminal.key_input(&keypad).unwrap(), b"1");

        let shifted_enter = key(
            TerminalKey::Enter,
            "",
            KeyModifiers {
                shift: true,
                ..KeyModifiers::default()
            },
        );
        assert_eq!(terminal.key_input(&shifted_enter).unwrap(), b"\x1b[13;2u");
        let mut keypad_enter = shifted_enter.clone();
        keypad_enter.physical = PhysicalKey::new(97);
        assert_ne!(terminal.key_input(&keypad_enter).unwrap(), b"\x1b[13;2u");

        let ctrl_a = key(
            TerminalKey::Character,
            "a",
            KeyModifiers {
                ctrl: true,
                ..KeyModifiers::default()
            },
        );
        assert_eq!(terminal.key_input(&ctrl_a).unwrap(), [1]);
        terminal.write(b"\x1b[>4;2m");
        assert_ne!(terminal.key_input(&ctrl_a).unwrap(), [1]);
        assert_eq!(
            terminal.key_input(&shifted_enter).unwrap(),
            b"\x1b[27;2;13~"
        );

        terminal.write(b"\x1b[>27u");
        let mut negotiated_enter = shifted_enter;
        negotiated_enter.action = KeyAction::Repeat;
        let negotiated = terminal.key_input(&negotiated_enter).unwrap();
        assert!(String::from_utf8(negotiated).unwrap().contains(":2"));
    }

    #[test]
    fn kitty_lone_shift_text_remains_text_for_nvim() {
        for (text, physical, unshifted) in [(":", 49, ';'), ("A", 20, 'a')] {
            let mut terminal = Terminal::new(80, 24).unwrap();
            // Neovim requests disambiguation plus event types, but not report-all.
            terminal.write(b"\x1b[>3u");
            let mut event = key(
                TerminalKey::Character,
                text,
                KeyModifiers {
                    shift: true,
                    ..KeyModifiers::default()
                },
            );
            event.physical = PhysicalKey::new(physical);
            event.unshifted_codepoint = Some(unshifted);
            assert_eq!(terminal.key_input(&event).unwrap(), text.as_bytes());
        }
    }

    #[test]
    fn kitty_keyboard_flags_encode_repeat_action() {
        let mut terminal = Terminal::new(80, 24).unwrap();
        terminal.write(b"\x1b[>27u");
        let mut event = key(TerminalKey::Up, "", KeyModifiers::default());
        event.physical = PhysicalKey::new(78);
        let press = terminal.key_input(&event).unwrap();
        event.action = KeyAction::Repeat;
        let repeat = terminal.key_input(&event).unwrap();
        assert_ne!(press, repeat);
        assert!(String::from_utf8(repeat).unwrap().contains(":2"));
    }

    #[test]
    fn ansi_styles_colors_cursor_and_crlf() {
        let mut terminal = Terminal::new(8, 3).unwrap();
        terminal.write(b"\x1b[1;3;4;7;38;2;1;2;3;48;2;4;5;6mA\x1b[0m\r\nB\x1b[3;4H");

        let screen = terminal.screen().unwrap();
        let styled = &screen.cells[0][0];
        assert_eq!(styled.text, "A");
        assert_eq!(styled.fg, [4, 5, 6]);
        assert_eq!(styled.bg, [1, 2, 3]);
        assert!(styled.bold);
        assert!(styled.italic);
        assert_eq!(styled.underline, UnderlineStyle::Single);
        assert_eq!(screen.cells[1][0].text, "B");
        assert_eq!((screen.cursor_x, screen.cursor_y), (3, 2));
        assert!(screen.cursor_visible);
    }

    #[test]
    fn sgr_styles_variants_colors_and_resets_are_exported() {
        let mut terminal = Terminal::new(20, 2).unwrap();
        terminal.write(b"\x1b[2;5;8;9;53;4:1;58;2;1;2;3mA\x1b[4:2mB\x1b[4:3mC\x1b[4:4mD\x1b[4:5mE");
        terminal.write(b"\x1b[0;4;38;5;1;58;5;1mF\x1b[59mG\x1b[24mH");

        let screen = terminal.screen().unwrap();
        let a = &screen.cells[0][0];
        assert_eq!(a.text, "A");
        assert!(
            a.faint && a.blink && a.concealed && a.strikethrough && a.overline,
            "all Ghostty style flags must cross the shim"
        );
        assert_eq!(a.underline, UnderlineStyle::Single);
        assert_eq!(a.underline_color, [1, 2, 3]);
        for (x, style) in [
            (1, UnderlineStyle::Double),
            (2, UnderlineStyle::Curly),
            (3, UnderlineStyle::Dotted),
            (4, UnderlineStyle::Dashed),
        ] {
            assert_eq!(screen.cells[0][x].underline, style);
            assert_eq!(screen.cells[0][x].underline_color, [1, 2, 3]);
        }
        let palette = &screen.cells[0][5];
        assert_eq!(palette.underline, UnderlineStyle::Single);
        assert_eq!(palette.underline_color, screen.cells[0][5].fg);
        assert_ne!(palette.underline_color, a.underline_color);
        let reset_color = &screen.cells[0][6];
        assert_eq!(reset_color.underline_color, reset_color.fg);
        assert_eq!(screen.cells[0][7].underline, UnderlineStyle::None);

        terminal.write(b"\r\n\x1b[2;5;8;9;53;4mX\x1b[22;25;28;29;55;24mY");
        let reset = terminal.screen().unwrap();
        let y = &reset.cells[1][1];
        assert!(!y.faint && !y.blink && !y.concealed && !y.strikethrough && !y.overline);
        assert_eq!(y.underline, UnderlineStyle::None);
    }

    #[test]
    fn inverse_resolves_colors_before_default_underline_color() {
        let mut terminal = Terminal::new(4, 1).unwrap();
        terminal.write(b"\x1b[7;4;38;2;1;2;3;48;2;4;5;6mA");
        let screen = terminal.screen().unwrap();
        let cell = &screen.cells[0][0];
        assert_eq!(cell.fg, [4, 5, 6]);
        assert_eq!(cell.bg, [1, 2, 3]);
        assert_eq!(cell.underline_color, cell.fg);
    }

    #[test]
    fn decscusr_cursor_shape_and_blinking_follow_application_requests() {
        use griz_protocol::CursorShape::{Bar, Block, Underline};
        let mut terminal = Terminal::new(8, 2).unwrap();
        let default = terminal.screen().unwrap();
        assert_eq!(default.cursor_shape, Block);
        for (parameter, shape, blinking) in [
            (1, Block, true),
            (2, Block, false),
            (3, Underline, true),
            (4, Underline, false),
            (5, Bar, true),
            (6, Bar, false),
        ] {
            terminal.write(format!("\x1b[{parameter} q").as_bytes());
            let screen = terminal.screen().unwrap();
            assert_eq!(screen.cursor_shape, shape, "DECSCUSR {parameter}");
            assert_eq!(screen.cursor_blinking, blinking, "DECSCUSR {parameter}");
        }
        // Neovim normal -> insert -> normal switches need no printable output.
        for (sequence, shape) in [
            (b"\x1b[2 q", Block),
            (b"\x1b[6 q", Bar),
            (b"\x1b[2 q", Block),
        ] {
            terminal.write(sequence);
            assert_eq!(terminal.screen().unwrap().cursor_shape, shape);
        }
        terminal.write(b"\x1b[0 q");
        let reset = terminal.screen().unwrap();
        assert_eq!(reset.cursor_shape, default.cursor_shape);
        assert_eq!(reset.cursor_blinking, default.cursor_blinking);
        terminal.write(b"\x1b[?25l");
        assert!(!terminal.screen().unwrap().cursor_visible);
        terminal.write(b"\x1b[?25h");
        assert!(terminal.screen().unwrap().cursor_visible);
    }

    #[test]
    fn osc_cursor_color_and_reset_are_exported() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        let default = terminal.screen().unwrap().cursor_color;
        terminal.write(b"\x1b]12;#123456\x07");
        assert_eq!(
            terminal.screen().unwrap().cursor_color,
            Some([0x12, 0x34, 0x56])
        );
        terminal.write(b"\x1b]112\x07");
        assert_eq!(terminal.screen().unwrap().cursor_color, default);
    }

    #[test]
    fn indexed_ansi_colors_are_resolved_to_rgb() {
        let mut terminal = Terminal::new(4, 1).unwrap();
        terminal.write(b"\x1b[31mA\x1b[38;5;1mB\x1b[0mC");

        let screen = terminal.screen().unwrap();
        assert_eq!(screen.cells[0][0].fg, screen.cells[0][1].fg);
        assert_ne!(screen.cells[0][0].fg, screen.cells[0][2].fg);
    }

    #[test]
    fn unicode_graphemes_and_wide_continuations() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.write("A界e\u{301}".as_bytes());

        let screen = terminal.screen().unwrap();
        assert_eq!(screen.cells[0][0].text, "A");
        assert_eq!(screen.cells[0][0].width, 1);
        assert_eq!(screen.cells[0][1].text, "界");
        assert_eq!(screen.cells[0][1].width, 2);
        assert_eq!(screen.cells[0][2].text, "");
        assert_eq!(screen.cells[0][2].width, 0);
        assert_eq!(screen.cells[0][3].text, "e\u{301}");
    }

    #[test]
    fn row_metadata_distinguishes_chunked_soft_wraps_from_hard_breaks() {
        let mut terminal = Terminal::new(4, 3).unwrap();
        terminal.write(b"ab");
        terminal.write(b"cd");
        assert!(!terminal.screen().unwrap().is_row_soft_wrapped(0));

        terminal.write(b"e");
        let wrapped = terminal.screen().unwrap();
        assert_eq!(wrapped.row_soft_wrapped.as_slice(), &[true, false, false]);
        assert_eq!(wrapped.cells[1][0].text, "e");

        terminal.write(b"\r\nz");
        let hard = terminal.screen().unwrap();
        assert!(hard.is_row_soft_wrapped(0));
        assert!(!hard.is_row_soft_wrapped(1));
        assert_eq!(hard.cells[2][0].text, "z");
    }

    #[test]
    fn wide_glyph_at_the_right_edge_exports_a_non_text_spacer_head() {
        let mut terminal = Terminal::new(4, 2).unwrap();
        terminal.write("abc界Z".as_bytes());

        let screen = terminal.screen().unwrap();
        assert!(screen.is_row_soft_wrapped(0));
        assert!(screen.cells[0][3].wide_spacer_head);
        assert!(screen.cells[0][3].text.is_empty());
        assert_eq!(screen.cells[1][0].text, "界");
        assert_eq!(screen.cells[1][0].width, 2);
        assert_eq!(screen.cells[1][1].width, 0);
        assert_eq!(screen.cells[1][2].text, "Z");
    }

    #[test]
    fn resize_reflow_recomputes_soft_wrap_rows() {
        let mut terminal = Terminal::new(6, 4).unwrap();
        terminal.write(b"abcdefghij\r\nXY");
        assert_eq!(
            terminal.screen().unwrap().row_soft_wrapped.as_slice(),
            &[true, false, false, false]
        );

        terminal.resize(4, 4).unwrap();
        let screen = terminal.screen().unwrap();
        assert_eq!(
            screen.row_soft_wrapped.as_slice(),
            &[true, true, false, false]
        );
        let text: String = screen
            .cells
            .iter()
            .flat_map(|row| row.iter())
            .filter(|cell| cell.width != 0 && !cell.wide_spacer_head)
            .map(|cell| cell.text.as_str())
            .collect();
        assert!(text.contains("abcdefghijXY"));
    }

    #[test]
    fn resize_updates_dimensions_and_preserves_content() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.write(b"hello");
        terminal.resize(3, 3).unwrap();

        let screen = terminal.screen().unwrap();
        assert_eq!((screen.cols, screen.rows), (3, 3));
        assert_eq!(screen.cells.len(), 3);
        assert!(
            screen
                .cells
                .iter()
                .flatten()
                .map(|cell| cell.text.as_str())
                .collect::<String>()
                .contains("hello")
        );
    }

    #[test]
    fn alternate_screen_restores_primary_screen() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.write(b"primary");
        terminal.write(b"\x1b[?1049h\x1b[Halt");
        assert_eq!(terminal.screen().unwrap().cells[0][0].text, "a");

        terminal.write(b"\x1b[?1049l");
        let screen = terminal.screen().unwrap();
        assert_eq!(screen.cells[0][0].text, "p");
        assert_eq!(screen.cells[0][1].text, "r");
    }

    fn mouse(action: MouseAction, button: Option<MouseButton>, x: f32, y: f32) -> MouseInput {
        MouseInput {
            action,
            button,
            x,
            y,
            cell_width: 8,
            cell_height: 16,
            shift: false,
            alt: false,
            ctrl: false,
        }
    }

    #[test]
    fn mouse_tracking_state_and_disabled_encoding() {
        let mut terminal = Terminal::new(80, 24).unwrap();
        let event = mouse(MouseAction::Press, Some(MouseButton::Left), 8.0, 16.0);
        assert!(!terminal.screen().unwrap().mouse_tracking);
        assert!(terminal.mouse_input(&event).unwrap().is_empty());

        for mode in [9, 1000, 1002, 1003] {
            terminal.write(format!("\x1b[?{mode}h").as_bytes());
            assert!(terminal.screen().unwrap().mouse_tracking);
            terminal.write(format!("\x1b[?{mode}l").as_bytes());
            assert!(!terminal.screen().unwrap().mouse_tracking);
        }
    }

    #[test]
    fn mouse_x10_and_sgr_press_release_motion_wheel_and_modifiers() {
        let mut terminal = Terminal::new(80, 24).unwrap();
        let press = mouse(MouseAction::Press, Some(MouseButton::Left), 8.0, 16.0);

        terminal.write(b"\x1b[?9h");
        assert_eq!(terminal.mouse_input(&press).unwrap(), b"\x1b[M \"\"");
        assert!(
            terminal
                .mouse_input(&mouse(
                    MouseAction::Release,
                    Some(MouseButton::Left),
                    8.0,
                    16.0,
                ))
                .unwrap()
                .is_empty()
        );

        terminal.write(b"\x1b[?9l\x1b[?1000h\x1b[?1006h");
        assert_eq!(terminal.mouse_input(&press).unwrap(), b"\x1b[<0;2;2M");
        assert_eq!(
            terminal
                .mouse_input(&mouse(
                    MouseAction::Release,
                    Some(MouseButton::Left),
                    8.0,
                    16.0,
                ))
                .unwrap(),
            b"\x1b[<0;2;2m"
        );
        assert!(
            terminal
                .mouse_input(&mouse(MouseAction::Motion, None, 8.0, 16.0))
                .unwrap()
                .is_empty()
        );

        terminal.write(b"\x1b[?1002h");
        assert_eq!(
            terminal
                .mouse_input(&mouse(
                    MouseAction::Motion,
                    Some(MouseButton::Left),
                    8.0,
                    16.0,
                ))
                .unwrap(),
            b"\x1b[<32;2;2M"
        );
        assert!(
            terminal
                .mouse_input(&mouse(MouseAction::Motion, None, 8.0, 16.0))
                .unwrap()
                .is_empty()
        );

        terminal.write(b"\x1b[?1003h");
        assert_eq!(
            terminal
                .mouse_input(&mouse(MouseAction::Motion, None, 8.0, 16.0))
                .unwrap(),
            b"\x1b[<35;2;2M"
        );
        assert_eq!(
            terminal
                .mouse_input(&mouse(
                    MouseAction::Press,
                    Some(MouseButton::WheelUp),
                    8.0,
                    16.0,
                ))
                .unwrap(),
            b"\x1b[<64;2;2M"
        );
        assert_eq!(
            terminal
                .mouse_input(&mouse(
                    MouseAction::Press,
                    Some(MouseButton::WheelDown),
                    8.0,
                    16.0,
                ))
                .unwrap(),
            b"\x1b[<65;2;2M"
        );
        let mut modified = press;
        modified.shift = true;
        modified.alt = true;
        modified.ctrl = true;
        assert_eq!(terminal.mouse_input(&modified).unwrap(), b"\x1b[<28;2;2M");
    }

    #[test]
    fn sgr_click_ignores_pointer_jitter_within_the_pressed_cell() {
        let mut terminal = Terminal::new(80, 24).unwrap();
        terminal.write(b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h");
        let press = mouse(MouseAction::Press, Some(MouseButton::Left), 20.0, 24.0);
        assert_eq!(terminal.mouse_input(&press).unwrap(), b"\x1b[<0;3;2M");
        // Pi re-renders on press. Output must not reset the encoder's last reported cell.
        terminal.write(b"\x1b[Hredraw");
        let jitter = mouse(MouseAction::Motion, Some(MouseButton::Left), 20.5, 24.5);
        assert!(
            terminal.mouse_input(&jitter).unwrap().is_empty(),
            "same-cell motion turns Pi's logo click into a drag"
        );
        let release = mouse(MouseAction::Release, Some(MouseButton::Left), 20.5, 24.5);
        assert_eq!(terminal.mouse_input(&release).unwrap(), b"\x1b[<0;3;2m");
    }

    #[test]
    fn mouse_motion_dedup_preserves_real_drags_and_pixel_reporting() {
        let mut terminal = Terminal::new(80, 24).unwrap();
        terminal.write(b"\x1b[?1003h\x1b[?1006h");
        let hover = mouse(MouseAction::Motion, None, 20.0, 24.0);
        assert_eq!(terminal.mouse_input(&hover).unwrap(), b"\x1b[<35;3;2M");
        assert!(terminal.mouse_input(&hover).unwrap().is_empty());
        let press = mouse(MouseAction::Press, Some(MouseButton::Left), 20.0, 24.0);
        assert_eq!(terminal.mouse_input(&press).unwrap(), b"\x1b[<0;3;2M");
        let drag = mouse(MouseAction::Motion, Some(MouseButton::Left), 28.0, 24.0);
        assert_eq!(terminal.mouse_input(&drag).unwrap(), b"\x1b[<32;4;2M");
        assert!(terminal.mouse_input(&drag).unwrap().is_empty());
        let release = mouse(MouseAction::Release, Some(MouseButton::Left), 28.0, 24.0);
        assert_eq!(terminal.mouse_input(&release).unwrap(), b"\x1b[<0;4;2m");
        // A second press/release in the same cell must not be deduplicated.
        let press = mouse(MouseAction::Press, Some(MouseButton::Left), 28.0, 24.0);
        assert_eq!(terminal.mouse_input(&press).unwrap(), b"\x1b[<0;4;2M");
        assert_eq!(terminal.mouse_input(&release).unwrap(), b"\x1b[<0;4;2m");

        terminal.write(b"\x1b[?1006l\x1b[?1016h");
        assert_eq!(terminal.mouse_input(&press).unwrap(), b"\x1b[<0;28;24M");
        // Pixel mode must still report movement inside the same character cell.
        let pixel_drag = mouse(MouseAction::Motion, Some(MouseButton::Left), 29.0, 24.0);
        assert_eq!(
            terminal.mouse_input(&pixel_drag).unwrap(),
            b"\x1b[<32;29;24M"
        );
    }

    #[test]
    fn mouse_geometry_changes_invalidate_motion_dedup() {
        let mut terminal = Terminal::new(80, 24).unwrap();
        terminal.write(b"\x1b[?1003h\x1b[?1006h");
        let mut motion = mouse(MouseAction::Motion, None, 20.0, 24.0);
        assert_eq!(terminal.mouse_input(&motion).unwrap(), b"\x1b[<35;3;2M");
        motion.cell_width = 10;
        assert_eq!(terminal.mouse_input(&motion).unwrap(), b"\x1b[<35;3;2M");
        terminal.resize(100, 40).unwrap();
        assert_eq!(terminal.mouse_input(&motion).unwrap(), b"\x1b[<35;3;2M");
    }

    #[test]
    fn mouse_utf8_urxvt_and_sgr_pixels_follow_terminal_modes() {
        let mut terminal = Terminal::new(400, 100).unwrap();
        let event = mouse(MouseAction::Press, Some(MouseButton::Left), 2392.0, 32.0);

        terminal.write(b"\x1b[?1000h\x1b[?1005h");
        assert_eq!(
            terminal.mouse_input(&event).unwrap(),
            [b"\x1b[M ".as_slice(), "Ō".as_bytes(), b"#"].concat()
        );

        terminal.write(b"\x1b[?1005l\x1b[?1015h");
        assert_eq!(terminal.mouse_input(&event).unwrap(), b"\x1b[32;300;3M");

        terminal.write(b"\x1b[?1015l\x1b[?1016h");
        assert_eq!(terminal.mouse_input(&event).unwrap(), b"\x1b[<0;2392;32M");
    }

    #[test]
    fn invalid_mouse_geometry_is_rejected() {
        let mut terminal = Terminal::new(80, 24).unwrap();
        terminal.write(b"\x1b[?1000h");
        let mut event = mouse(MouseAction::Press, Some(MouseButton::Left), 0.0, 0.0);
        event.cell_width = 0;
        let error = terminal.mouse_input(&event).unwrap_err();
        assert_eq!(error.operation(), "encode mouse input");
    }

    #[test]
    fn title_modes_and_terminal_responses_are_exposed() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.write(b"\x1b]2;griz title\x07\x1b[?1h\x1b[?2004h\x1b[6n");

        let screen = terminal.screen().unwrap();
        assert_eq!(screen.title, "griz title");
        assert!(screen.application_cursor);
        assert!(screen.bracketed_paste);
        assert_eq!(terminal.take_responses().unwrap(), b"\x1b[1;1R");
        assert!(terminal.take_responses().unwrap().is_empty());
    }

    #[test]
    fn synchronized_output_hold_tracks_chunked_start_end_reset_and_resize() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        assert!(!terminal.render_held());

        terminal.write(b"\x1b[?20");
        assert!(!terminal.render_held());
        terminal.write(b"26h");
        assert!(terminal.render_held());
        terminal.write(b"\x1b[?2026h");
        assert!(
            terminal.render_held(),
            "repeated begin is not a new transition"
        );
        terminal.write(b"\x1b[?202");
        assert!(terminal.render_held());
        terminal.write(b"6l");
        assert!(!terminal.render_held());

        terminal.write(b"\x1b[?2026hheld\x1b");
        assert!(terminal.render_held());
        terminal.write(b"c");
        assert!(!terminal.render_held(), "RIS ends the hold");

        terminal.write(b"\x1b[?2026h");
        assert!(terminal.render_held());
        terminal.resize(9, 3).unwrap();
        assert!(!terminal.render_held(), "Ghostty resize ends the hold");
    }

    #[test]
    fn synchronized_output_keeps_parsing_replies_and_input_modes_live() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.write(b"\x1b[?2026hintermediate\x1b[6n\x1b[?2004h");
        assert!(terminal.render_held());
        assert_eq!(terminal.take_responses().unwrap(), b"\x1b[2;5R");
        assert_eq!(
            terminal.paste("one\ntwo", false).unwrap(),
            PasteResult::Written(b"\x1b[200~one\ntwo\x1b[201~".to_vec())
        );
        assert!(terminal.render_held());
        assert!(
            terminal
                .screen()
                .unwrap()
                .cells
                .iter()
                .flatten()
                .any(|cell| cell.text == "i"),
            "the VT continues updating internally while publication is held"
        );
    }

    #[test]
    fn stalled_hold_can_be_ended_and_a_future_hold_can_start() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.write(b"\x1b[?2026hfirst");
        assert!(terminal.render_held());
        terminal.end_render_hold().unwrap();
        assert!(!terminal.render_held());
        terminal.write(b"\x1b[?2026hsecond");
        assert!(terminal.render_held());
        terminal.write(b"\x1b[?2026l");
        assert!(!terminal.render_held());
    }

    #[test]
    fn paste_uses_live_bracketed_mode_and_rejects_only_unsafe_text() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        assert_eq!(
            terminal.paste("hello", false).unwrap(),
            PasteResult::Written(b"hello".to_vec())
        );
        assert_eq!(
            terminal.paste("one\ntwo", false).unwrap(),
            PasteResult::Rejected
        );
        assert_eq!(
            terminal.paste("one\ntwo", true).unwrap(),
            PasteResult::Written(b"one\rtwo".to_vec())
        );

        terminal.write(b"\x1b[?2004h");
        assert_eq!(
            terminal.paste("one\ntwo", false).unwrap(),
            PasteResult::Written(b"\x1b[200~one\ntwo\x1b[201~".to_vec())
        );
        assert_eq!(
            terminal.paste("x\x1b[201~y", false).unwrap(),
            PasteResult::Rejected
        );
    }

    #[test]
    fn paste_sanitizes_controls_preserves_tabs_unicode_and_handles_empty_text() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        assert_eq!(terminal.paste("", false).unwrap(), PasteResult::Empty);
        assert_eq!(
            terminal.paste("a\0\tb\x7fc界", false).unwrap(),
            PasteResult::Written("a \tb c界".as_bytes().to_vec())
        );
    }

    #[test]
    fn kitty_paste_mode_without_clipboard_read_falls_back_to_text() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.write(b"\x1b[?5522h");
        assert_eq!(
            terminal.paste("fallback", false).unwrap(),
            PasteResult::Written(b"fallback".to_vec())
        );
        terminal.write(b"\x1b[?2004h");
        assert_eq!(
            terminal.paste("fallback", false).unwrap(),
            PasteResult::Written(b"\x1b[200~fallback\x1b[201~".to_vec())
        );
    }

    #[test]
    fn capture_allocation_failure_returns_no_partial_paste() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.fail_next_capture_allocation();
        let error = terminal.paste("complete or nothing", false).unwrap_err();
        assert_eq!(error.code(), OUT_OF_MEMORY);
        assert_eq!(error.operation(), "encode paste");
        assert!(terminal.take_responses().unwrap().is_empty());
    }
}
