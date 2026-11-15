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

### 1. Terminal-aware paste encoding and unsafe-paste confirmation — addressed

Paste is now a distinct GUI → core operation. Core calls bundled
`ghostty_terminal_paste` against the target terminal's live modes, so mode 2004
bracketed multiline flows without a prompt, while Ghostty's `GHOSTTY_REJECTED`
result opens a modal for unsafe unbracketed line terminators or a bracketed-paste
terminator. Ghostty also performs control-byte sanitization and newline framing.

The confirmation event contains only fixed-size counts and identifiers, never the
clipboard text. Core holds at most one paste (maximum 256 KiB of raw UTF-8) per
client behind a one-use token bound to the original terminal and shell generation.
Selection changes, replacement pastes, shell replacement/exit/closure, and client
disconnect invalidate it. Confirmation rechecks the selected target and generation,
then encodes again against the terminal's current modes; it never reroutes.

`GHOSTTY_TERMINAL_OPT_CLIPBOARD_READ` remains unset. Consequently Kitty mode 5522
honestly falls back to text paste rather than advertising clipboard ownership or
permission that Griz does not implement. Clipboard reads remain tracked separately
under terminal clipboard operations below.

### 2. Stop dropping SGR styles, especially invisible text — addressed

The Ghostty snapshot and IPC now carry faint, conceal, strikethrough, overline,
cell blink, every `GhosttySgrUnderline` variant, and resolved underline RGB.
Rendering uses cell-relative decoration geometry for text, wide graphemes, block
graphics, previews, and block-cursor redraws. Blink remains renderer-owned and
Reduce Motion holds ink visible; backgrounds stay steady.

Conceal suppresses all ink and decorations, including graphics and cursor redraws,
while retaining terminal text for selection/copy. It is display behavior, not a
security boundary.

### 3. Honor synchronized-output holds — addressed

The VT shim registers `GHOSTTY_TERMINAL_OPT_RENDER_HOLD` and exposes only its
callback-derived boolean to Rust; Griz does not duplicate the escape-sequence
parser. PTY bytes continue through Ghostty while held, including query replies,
program-status/catalog effects, paste/input mode changes, resets, and other VT
state. Only screen publication is coalesced.

Each terminal retains one last completed live `Screen`, initially the blank new
terminal, and each client retains one completed `Screen` per currently visible
real pane for its independent scroll viewport. Client caches are pruned when
switching tabs (at most eight panes), and terminal caches are bounded by the
64-terminal limit. A newly revisited held pane falls back to its completed live
frame until release. Caches are removed with terminals/clients. Attach, commands,
directory/catalog changes, output from another terminal, split panes, previews,
and scroll publications use those completed frames while held; a fresh attach
therefore receives the last completed live frame, or the initial blank frame if
mode 2026 began before any later frame completed.

Ghostty's paired callback semantics are authoritative: DECRST 2026 and RIS release
a hold, and `ghostty_terminal_resize()` also releases it as documented so the
resized result can appear immediately. Release schedules the latest screen without
polling. A missing release is recovered after one second by disabling mode 2026
through Ghostty's host mode API (whose documented behavior does not invoke the
callback) and synchronizing the shim boolean. Repeated begin sequences do not move
the deadline; shell exit also ends a hold, and later holds can start normally.

### 4. Use Ghostty's keyboard encoder — addressed within Rune metadata limits

Ordinary native keys now cross GUI → core as bounded structured events: logical
key, optional Rune `Tx::physical_key`, optional layout character from `Tx::key_base`,
press/repeat action, modifiers, and generated UTF-8 text. The shim sets (and resets)
Ghostty's unshifted codepoint on each event; omitting this made Ctrl+V arrive as a
plain `v` under Pi's Kitty flags (7), disabling Pi's own clipboard-image attachment
shortcut. Older/logical-only events fall back to a single logical character rather
than a guessed US physical layout. Core requires the addressed terminal to be that
client's selected live shell, then the terminal-owned Ghostty encoder calls
`ghostty_key_encoder_set_from_terminal` immediately before every encoding. Cursor,
keypad, DECBKM, alt-prefix, modifyOtherKeys, and Kitty flags therefore come from the
live terminal rather than a stale `Screen` or a second escape-sequence parser.
Rune's full logical named set and F1–F24 are represented; supported native physical
codes include keypad and media keys.

Griz keeps its historical Shift+Enter CSI-u sequence only while neither Kitty flags
nor modifyOtherKeys is negotiated. Once either keyboard mode is active, Ghostty's
result is authoritative, including Kitty repeat event types. Native Command/platform shortcuts remain outside terminal input,
and macOS Option-generated Unicode remains text rather than gaining an artificial
Meta escape prefix. For macOS Option-generated Unicode, Griz marks Alt consumed
so adding the layout character does not turn composed text into a Kitty shortcut.

IME commits still use bounded raw UTF-8 `Input`. An IME commit has composed text but
no trustworthy native key identity/action, so Griz does not fabricate a Kitty
report-all/associated-text event for it. No additional keyboard capability callback
is registered or advertised.

**Remaining metadata limits:** Rune's logical `on_key` path reports presses and
`Tx::key_repeat`, but not matching releases, so Griz sends press/repeat only and does
not invent release events. Native dispatch usually supplies a physical code; scripted
`Runtime::key` events and some platform keys do not, and remain logical-only. Rune's
`Mods` also omits Caps Lock, Num Lock, modifier-side, consumed-modifier, and composing
state, so those Ghostty event fields stay unset. Applications requesting Kitty release
or modifier-only streams will not receive events Griz cannot observe.

### 5. Preserve logical lines when copying — addressed for the visible grid

Ghostty row wrap state now crosses the C/Rust snapshot and protocol `Screen` for
both live and scrolled captures. Client-local copying omits separators across
soft-wrapped physical rows, retains hard breaks and explicitly selected spaces,
and excludes Ghostty's wide-glyph spacer head. Unicode graphemes, wide
continuations, reverse/partial selections, reflowed rows, and frozen selection
snapshots retain their existing visible-grid behavior. Older saved frames without
row metadata safely treat every row boundary as hard.

**Remaining backlog:** Selection still covers only the captured visible grid and
uses Griz's existing click/word/triple-line gestures. Moving endpoints and gesture
semantics to Ghostty, selecting across history with autoscroll, and semantic
command-output selection remain separate work using `selection.h` and
`formatter.h`.

### 6. Configure a deliberate scrollback budget — addressed

Every `griz-vt` terminal now sets both Ghostty limits during initialization:
10,000 physical history lines and an estimated 8 MiB page allocation per terminal,
whichever is reached first. The line limit provides useful ordinary shell history;
the byte limit bounds narrow, wide-glyph, and style-heavy workloads. The core owns
one emulator and budget per terminal, and all initial, workspace/tab, split, and
replacement-shell creation paths use the same constructor.

The bundled `terminal.h` defines both option values as `size_t*` and documents
whole-page pruning. Limits are therefore approximate: byte allocation can exceed the
configured value by about one page (currently roughly 400 KiB), and retained physical
lines can exceed their value by the remaining rows in a page. With Griz's 64-terminal
limit, configured scrollback byte budgets total 512 MiB, plus page-granularity and
active-screen/emulator overhead; this is a planning bound, not a hard allocator or RSS
guarantee. There is no user setting yet. Idle compression remains deferred until
workload and latency measurements justify its scheduling complexity.

## Major compatibility and capability gaps

### 7. Kitty graphics — addressed for static images

The shim reads Ghostty's resolved render-state image iterator and borrows decoded
pixel views only until Rust copies a new generation into an immutable RGBA asset.
Per-emulator namespaces isolate terminal IDs, shell replacements, and image IDs.
Completed/frozen frames own their assets, so synchronized holds cannot leak newly
replaced pixels. PNG decoding is registered once through `GHOSTTY_SYS_OPT_DECODE_PNG`,
with bounded dimensions/allocation, Ghostty allocator ownership, and no FFI unwinding.

The replaceable outbox still queues complete views, never separately droppable image
events. Its writer selects a view first and then streams newly referenced resources
in 64 KiB decoded/base64 chunks before that view. The GUI receiver validates and
hydrates the complete view before publication; caches retain only current-view keys.
The Rune renderer caches cropped pixels, paints Ghostty's three layers in z-order,
and clips to the grid. Fixed, virtual-placeholder, and relative placements use the
same resolved metadata; U+10EEEE placeholder graphemes are not painted as text.

Budgets: 16 MiB Kitty storage per screen, 16 MiB RGBA per exported resource/visible
screen, 8192 pixels per source axis, 256 placements per screen, 256 MiB unique
resources per live+preview view, and 32 MiB/256 cropped textures per GUI rendering
thread (plus active GPU/Rune/frozen-frame allocations). Raw images exceeding source
limits can remain in bounded Ghostty storage but are not exported. File/temp-file/
shared-memory image media are explicitly disabled. Animation scheduling, animated
PNG, iTerm2/Sixel, and JSON-only offline image snapshots remain outside this scope.

Validation includes Pi-style chunked PNG, source crops, replacement/delete/reset,
virtual and relative placement resolution, independent reconnect hydration, previews,
and held image replacement through the real service/PTY. An isolated native Griz
window visibly rendered a fixture emitted by the installed Pi Kitty encoder;
`artifacts/kitty-smoke/native-pi-kitty.png` records that check. A manual native
Control+V check also confirmed that Pi received `ESC[118;5u`, read a 46,834-byte
clipboard image, and inserted its temporary path. Separately, an offline replay of
an image-bearing `read` result through the installed Pi CLI's real tool-result UI
visibly rendered in Griz with the explicit Kitty override. The native A/B captures
are `artifacts/kitty-smoke/pi-image-output-{auto,kitty}.png`. No model request was
made; the complete interactive submit/model/tool cycle remains untested.

Capability caveat: the generic Kitty query is handled by Ghostty, but Pi 1.5.2
auto-detects by environment and does not recognize Griz. The clean-environment A/B
replay sent no image resources without an override, versus 98 resource chunks and
one visible placement with `PI_IMAGE_PROTOCOL=kitty pi`. Use that explicit user
override for image output in clean Pi launches; Griz does not spoof another
terminal or inject a Pi-specific variable. Earlier sessions inherited a Ghostty
fingerprint, explaining why Pi attempted image previews even before rendering existed.

### 8. Pixel geometry and size reports — addressed

Live panes report physical cell dimensions through bounded `SetCellSize` IPC.
Ghostty resizes retain those dimensions, its `GHOSTTY_TERMINAL_OPT_SIZE` callback
answers cell/window/grid size queries, and PTY `winsize` includes pixel dimensions
(saturating the OS's 16-bit fields where necessary). Default startup geometry is
8×16 pixels until a live client supplies its layout.

The last accepted live layout owns shared terminal geometry, as with grid resizing.
Preview-only clients cannot change it. Replacement shells inherit it, and a DPI-only
change updates physical cells even when point-space pane rectangles stay unchanged.
Geometry changes end a synchronized hold consistently with grid resize. Focus/theme
ownership and application-requested host-window resizing remain separate work.

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
- Keyboard press/repeat encoding is covered across live modes, but release,
  modifier-only, lock-state, consumed-modifier, and always-present physical metadata
  remain unavailable from Griz's Rune logical-key path. Extra mouse buttons and the
  complete Pi clipboard-submit/model/tool cycle remain unverified. Native clipboard
  attachment and real Pi tool-result image replay were checked separately; static
  image rendering and explicit scrollback retention have focused runtime tests.
- Clipboard, focus, theme, and terminal-controlled resize still require policies
  for multiple clients observing the same terminal. Grid/pixel geometry explicitly
  follows the last accepted live layout.
- Standalone utilities (allocator control, WebAssembly, raw OSC/SGR parsers,
  snapshot serialization, unknown-sequence interception) are intentionally not
  counted as missing features without a concrete Griz use case.
