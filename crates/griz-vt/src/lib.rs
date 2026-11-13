//! Safe, single-threaded wrapper around libghostty-vt 1.0.

mod program_status;
#[cfg(test)]
mod program_status_tests;
#[cfg(test)]
mod scrollback_tests;

use griz_protocol::{Cell, MouseAction, MouseButton, MouseInput, ProgramStatusRecord, Screen};
use std::error;
use std::ffi::{c_int, c_void};
use std::fmt;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;
use std::slice;

const CELL_BOLD: u8 = 1 << 0;
const CELL_ITALIC: u8 = 1 << 1;
const CELL_UNDERLINE: u8 = 1 << 2;
#[cfg(test)]
const OUT_OF_MEMORY: c_int = -1;
const INVALID_VALUE: c_int = -2;
const REJECTED: c_int = -7;

#[repr(C)]
struct GrizVt {
    _private: [u8; 0],
}

#[repr(C)]
struct GrizVtCell {
    text_offset: usize,
    text_len: usize,
    width: u8,
    fg: [u8; 3],
    bg: [u8; 3],
    flags: u8,
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
    cells: *mut GrizVtCell,
    cells_len: usize,
    text: *mut u8,
    text_len: usize,
    _text_cap: usize,
    title: *mut u8,
    title_len: usize,
}

unsafe extern "C" {
    fn griz_vt_new(
        cols: u16,
        rows: u16,
        out: *mut *mut GrizVt,
        report: extern "C" fn(*mut c_void, *const program_status::RawReport),
        prompt: extern "C" fn(*mut c_void),
        userdata: *mut c_void,
    ) -> c_int;
    fn griz_vt_free(vt: *mut GrizVt);
    fn griz_vt_write(vt: *mut GrizVt, data: *const u8, len: usize);
    fn griz_vt_resize(vt: *mut GrizVt, cols: u16, rows: u16) -> c_int;
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
    _single_threaded: PhantomData<Rc<()>>,
}

impl Terminal {
    /// Creates a terminal with the given cell dimensions.
    pub fn new(cols: u16, rows: u16) -> Result<Self, Error> {
        let mut raw = std::ptr::null_mut();
        let mut statuses = Box::<program_status::Records>::default();
        // SAFETY: out is valid; userdata is a stable Box kept alive until after
        // the terminal is freed. Callbacks run synchronously during VT writes.
        check("create", unsafe {
            griz_vt_new(
                cols,
                rows,
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
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Error> {
        // SAFETY: the terminal handle is live and the scalar arguments have the
        // exact types expected by the shim.
        check("resize", unsafe {
            griz_vt_resize(self.raw.as_ptr(), cols, rows)
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
        // SAFETY: Snapshot owns a live C allocation until this function exits.
        unsafe { snapshot.to_screen() }
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
    unsafe fn to_screen(&self) -> Result<Screen, Error> {
        // SAFETY: guaranteed by the caller and the lifetime of `self`.
        let snapshot = unsafe { self.0.as_ref() };
        let expected_cells = usize::from(snapshot.cols) * usize::from(snapshot.rows);
        if snapshot.cells_len != expected_cells
            || (snapshot.cells_len != 0 && snapshot.cells.is_null())
            || (snapshot.text_len != 0 && snapshot.text.is_null())
            || (snapshot.title_len != 0 && snapshot.title.is_null())
        {
            return Err(Error {
                operation: "decode render snapshot",
                code: INVALID_VALUE,
            });
        }

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
                    fg: cell.fg,
                    bg: cell.bg,
                    bold: cell.flags & CELL_BOLD != 0,
                    italic: cell.flags & CELL_ITALIC != 0,
                    underline: cell.flags & CELL_UNDERLINE != 0,
                });
            }
            cells.push(decoded_row);
        }

        Ok(Screen {
            cols: snapshot.cols,
            rows: snapshot.rows,
            cells,
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
        })
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        // SAFETY: this is the unique owned snapshot and it is freed exactly once.
        unsafe { griz_vt_snapshot_free(self.0.as_ptr()) }
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
        assert!(styled.underline);
        assert_eq!(screen.cells[1][0].text, "B");
        assert_eq!((screen.cursor_x, screen.cursor_y), (3, 2));
        assert!(screen.cursor_visible);
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
