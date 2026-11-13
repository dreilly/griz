# libghostty integration gaps

Static audit of Griz against the bundled libghostty-vt 1.0.0 public headers.
Prepared with a read-only subagent audit; priorities below are recommendations,
not a commitment to implement every API.

## What this audit means

Ghostty is already the authoritative VT parser. Griz feeds PTY output to
`ghostty_terminal_vt_write` and returns generated replies to the PTY
(`crates/griz-vt/src/ghostty_shim.c`, `crates/griz-core/src/service.rs`).
Many missing features are therefore **parsed correctly but dropped at the
snapshot, IPC, input, rendering, or host-effects boundary**. Adding another
escape-sequence parser is generally the wrong fix.

Implementation references below use files/symbols rather than fixed line numbers
because the cursor changes shift those lines. Header paths are relative to
`vendor/ghostty-vt-1.0.0.xcframework/ghostty-vt.xcframework/macos-arm64_x86_64/Headers/ghostty/vt/`.
This is primarily static evidence, not a live interoperability test matrix.

## Addressed with the cursor fix

- Cursor shape, blinking flag, and explicit cursor color are carried from Ghostty's
  render state through the C/Rust snapshot and `Screen` to the renderer.
- Block, bar, underline, and hollow-block geometry are supported. Block cursors
  redraw contrasting glyphs, including wide graphemes and cell graphics.
- Blink is a client-side presentation cycle, not core/IPC traffic. Reduce Motion
  holds the cursor visible. Blink uses a shared wall-clock phase; it does not yet
  restart its visible interval after every keystroke or cursor movement.

## Highest-priority follow-ups

### 1. Use terminal-aware paste encoding and unsafe-paste confirmation

**Confirmed:** `src/input.rs::paste` manually removes ESC, normalizes newlines,
and optionally adds bracketed-paste delimiters. `src/ui.rs` sends the resulting
bytes as generic input. Unbracketed multiline paste can immediately execute
multiple shell commands without confirmation.

**Ghostty support:** `ghostty_terminal_paste`, `GhosttyPaste`, `GHOSTTY_REJECTED`,
Kitty paste mode 5522, and `ghostty_paste_encode` (`paste.h`).

**Suggested scope:** A distinct paste command, encoded against the live terminal
in core/shim, with a rejection/confirmation round trip. Clipboard-read callbacks
for Kitty paste events need an explicit permission/ownership policy.

### 2. Stop dropping SGR styles, especially invisible text

**Confirmed:** `ghostty_shim.c::griz_fill_cell` and protocol `Cell` retain only
bold, italic, and a boolean underline. `src/terminal_view.rs` consequently omits
faint, invisible/conceal, strikethrough, overline, cell blink, and underline
style/color. In particular, concealed text is not concealed by Griz's renderer.

**Ghostty support:** Style flags, underline variants, and underline color in
`style.h`. This is cell blink, separate from the cursor work.

**Suggested scope:** Expand the cell style schema and implement conceal first,
then dim/strike/overline and curly/double/colored underline. Keep animation
renderer-side. Conceal is a display behavior, not protection of terminal data.

### 3. Honor synchronized-output holds

**Confirmed:** `crates/griz-core/src/service.rs` schedules output publication
without consulting Ghostty's render-hold state. Applications using mode 2026 can
still expose intermediate redraws.

**Ghostty support:** `GHOSTTY_MODE_SYNCHRONIZED_OUTPUT` and
`GHOSTTY_TERMINAL_OPT_RENDER_HOLD` (`modes.h`, `terminal.h`).

**Suggested scope:** Surface hold transitions and coalesce output publications
until release; include a bounded timeout/recovery strategy for misbehaving apps.

### 4. Use Ghostty's keyboard encoder

**Confirmed:** `src/input.rs::encode` hand-encodes a limited subset;
`src/main.rs::terminal_key` supplies only `Screen.application_cursor`. Negotiated
Kitty keyboard flags, modifyOtherKeys, keypad/backarrow modes, release events,
and some modified/function keys are not represented end to end.

**Ghostty support:** `ghostty_key_encoder_set_from_terminal` and
`ghostty_key_encoder_encode` (`key/encoder.h`).

**Suggested scope:** Preserve physical key, action, modifiers, and text over IPC;
encode in core against live terminal state. Verify Rune's physical-key/release
metadata before promising every negotiated mode.

### 5. Preserve logical lines when copying

**Confirmed:** `src/terminal_mouse.rs` copies a visible `Screen`, implements its
own word/line selection, and joins physical rows with newlines. Protocol `Screen`
has no wrap metadata. Soft-wrapped text acquires false newlines; selection cannot
extend through history beyond the captured viewport.

**Ghostty support:** Row wrap state (`screen.h`), native selection/gesture APIs
(`selection.h`), and formatter unwrapping/trimming (`formatter.h`).

**Suggested scope:** Prefer core-owned selection endpoints and Ghostty formatter
output, or first export wrap metadata to fix copying. Later add history
selection/autoscroll and semantic command-output selection.

### 6. Configure a deliberate scrollback budget

**Confirmed:** `griz_vt_new` does not set a scrollback limit. The bundled
`terminal.h` documents a default 10,000-byte budget rounded up to one page
(approximately 400 KB), so retained history is shallow and workload-dependent.

**Ghostty support:** `GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_BYTES`,
`GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_LINES`, and idle compression APIs.

**Suggested scope:** Choose an explicit memory/line budget, expose a user setting
if useful, and consider incremental compression only after measuring workloads.

## Major compatibility and capability gaps

### 7. Kitty graphics are not rendered

**Confirmed:** Griz snapshots and IPC contain only cells; the renderer has no
image-placement path. Ghostty exposes image storage and render placements, and
its documented default Kitty storage budget is 10 MB per screen. The subagent
also verified image/Kitty symbols in the bundled static library. The exact live
capability-query behavior remains untested.

**Ghostty support:** `GHOSTTY_RENDER_STATE_DATA_IMAGE_ITERATOR`, image pixel/data
APIs (`render.h`), and `GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_STORAGE_LIMIT`.

**Suggested scope:** Until implementing image transport/cache/layers/animation,
consider setting storage to zero so unsupported graphics are not accepted and
stored invisibly. Implementing rendering is substantial cross-layer work.

### 8. Pixel geometry and size reports are missing

**Confirmed:** Resize IPC carries only columns/rows; Ghostty resize and PTY
`winsize` receive zero pixel dimensions (`ghostty_shim.c`,
`crates/griz-core/src/session.rs`).

**Ghostty support:** `GHOSTTY_TERMINAL_OPT_SIZE`, `GhosttyTerminalSizeFn`,
pixel-aware resize, and mode 2048 (`terminal.h`, `size_report.h`, `modes.h`).

**Impact/scope:** XTWINOPS/cell-size/in-band reports and image geometry need
actual cell pixels over resize IPC. Multiple clients with different font sizes
need an explicit geometry owner.

### 9. Focus reporting has no input path

**Confirmed:** No protocol/core path sends terminal focus-in/out events.

**Ghostty support:** `GHOSTTY_MODE_FOCUS_EVENT` and `ghostty_focus_encode`
(`modes.h`, `focus.h`).

**Impact/scope:** TUIs using mode 1004 cannot react to focus changes. Add focus
commands, but define the report owner for shared terminals/multiple windows.

### 10. Alternate-screen wheel navigation is ignored

**Confirmed:** `src/terminal_mouse.rs::wheel` routes non-mouse-reporting wheel
input to local history, without consulting alternate-screen state and mode 1007.

**Ghostty support:** `GHOSTTY_MODE_MOUSE_ALTERNATE_SCROLL` and active-screen state.

**Impact/scope:** Wheel/trackpad navigation may do nothing in alternate-screen
apps. Query screen/mode state and translate wheel steps using terminal-aware
key encoding.

### 11. Default versus explicit color information is lost

**Confirmed:** The shim immediately resolves missing cell colors to RGB;
`src/terminal_view.rs` treats all black backgrounds as transparent. Explicit
black backgrounds therefore become the client's theme background. Client theme
and white-point changes are not synchronized with Ghostty defaults/query replies.

**Ghostty support:** Cell color queries distinguish `GHOSTTY_NO_VALUE`; terminal
foreground/background/palette options and color-scheme APIs are available
(`render.h`, `terminal.h`, `color_scheme.h`).

**Suggested scope:** Preserve default/explicit provenance and define a terminal
color/scheme reporting policy. Different per-client themes complicate reporting.

## User-facing integrations

### 12. Terminal clipboard operations

OSC 52/1337/Kitty clipboard effects have no host callback integration.
`GHOSTTY_TERMINAL_OPT_CLIPBOARD_WRITE` / `_READ` in `terminal.h` provide the hooks.
Remote tools cannot copy to or request the local clipboard. Add bounded effect
records plus consent and client ownership; never silently enable arbitrary reads.

### 13. OSC 8 hyperlinks

Cell hyperlink flags and URI lookup exist in `screen.h` / `grid_ref.h`, but
protocol `Cell` and the renderer discard them. Export link spans/IDs, add
hover/click behavior, and validate URL schemes before launching anything.

### 14. OSC-reported working directory

Core polls the local process CWD (`service.rs`), ignoring
`GHOSTTY_TERMINAL_OPT_PWD_CHANGED` / `GHOSTTY_TERMINAL_DATA_PWD`. SSH/container
logical directories consequently cannot drive labels correctly. Parse OSC 7
URIs/hosts safely and define precedence over process polling; do not treat remote
paths as trusted local paths.

### 15. Bell, desktop notification, and standard progress

The shim integrates custom OSC 7501 status but not Ghostty's bell, desktop
notification, or OSC 9;4 progress callbacks (`terminal.h`). Add bounded effects,
background-tab policy, and notification permissions rather than interpreting
sequences independently in the GUI.

### 16. More semantic prompt events

`griz_semantic_prompt` consumes only `PROMPT_START`. Ghostty also exposes input
start, output start, command line, and command completion events, plus prompt
viewport movement and output selection. These enable prompt jumping, selecting
command output, command metadata, and exit-state presentation (`terminal.h`,
`selection.h`).

### 17. Search terminal content and history

Ghostty's standalone search API covers active screen/scrollback, selected
matches, and highlights (`search.h`); Griz has no terminal-content search IPC/UI.
Implement per-client search state in core, match counts, viewport navigation,
and highlight ranges. This is distinct from the existing navigation palette.

## Lower-priority completeness and diagnostics

### 18. Consistent terminal identity

`session.rs` sets `TERM=xterm-256color` and `TERM_PROGRAM=griz`, but Ghostty's
terminfo name is unset and XTVERSION defaults to `libghostty` per `terminal.h`.
Set `GHOSTTY_TERMINAL_OPT_TERMINFO_NAME` and a Griz XTVERSION callback to make
capability probes consistent with the capabilities actually supported.

### 19. Application window-resize requests

`GhosttyTerminalResizeWindowFn` / `GHOSTTY_TERMINAL_OPT_RESIZE_WINDOW` are not
integrated. Supporting `CSI 8;rows;cols t` is a product/security decision, not
necessarily a bug: terminal-controlled resizing is awkward in shared split layouts.

### 20. Extended mouse buttons

Ghostty supports buttons four through eleven (`mouse/event.h`), while Griz's
input model only covers the primary buttons and vertical wheel directions.
Only expand it if Rune/platform input exposes those events distinctly.

### 21. VT failure/memory telemetry

`GHOSTTY_TERMINAL_DATA_VT_PROCESSING_ERROR` and memory usage APIs (`terminal.h`)
are not read. Add diagnostic logging/counters so rare processing failures and
history/image memory use are observable without expanding routine frame IPC.

## Boundaries and remaining verification

- Header/API availability and absence of the Griz integration paths were
  statically inspected. This is not proof of every application's observed behavior.
- Keyboard metadata, extra mouse buttons, live graphics capability replies, and
  actual history depth should be verified with focused runtime tests.
- Clipboard, focus, geometry, theme, and terminal-controlled resize all require
  policies for multiple clients observing the same terminal.
- Standalone utilities (allocator control, WebAssembly, raw OSC/SGR parsers,
  snapshot serialization, unknown-sequence interception) are intentionally not
  counted as missing features without a concrete Griz use case.
