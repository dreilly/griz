# Griz

A native terminal workspace app built with [Rune](https://github.com/anomalyco/rune)
and [libghostty-vt 1.0](https://lib.ghostty.org/). Inspired by Herdr Desktop and Rex.

## Run (macOS)

With Rust/rustup and Xcode command-line tools installed:

```sh
./scripts/dev.sh
```

The script downloads the official, checksum-pinned libghostty-vt 1.0.0 XCFramework
if needed, builds both processes, and opens the GUI. No Zig installation is needed.
Downloads stay in the ignored `vendor/` directory. macOS Apple Silicon and Intel
are supported; other platforms are not wired up yet.

Or run the steps individually:

```sh
source "$HOME/.cargo/env" # if Cargo is not already on PATH
./scripts/setup-ghostty.sh
cargo build --workspace
cargo run -p griz
```

Rune is the sibling checkout at `../rune`, tested at dev revision
`7256c7c2afb605862ccfe600a469df7330509efb`. Rust is pinned to its toolchain, 1.99.1.
For a standalone checkout later, replace the Rune path dependency with a pinned git revision.

### Development

Griz is pre-release. The GUI and core evolve together; breaking changes are expected,
and IPC compatibility is not maintained. Rebuild both processes after changes. Dev builds use optimization level 1 so
whole-screen IPC and GUI layout stay responsive while retaining debug checks.

Rebuilding does not replace a running core, and stopping the core ends its terminals.
To test changes **without disturbing existing sessions**, use a separate endpoint:

```sh
preview_dir="$(mktemp -d /tmp/griz-preview.XXXXXX)"
chmod 700 "$preview_dir"
GRIZ_SOCKET="$preview_dir/core.sock" ./scripts/dev.sh
# Reopen that same preview later with:
GRIZ_SOCKET="$preview_dir/core.sock" ./target/debug/griz
# Explicitly end its terminals when finished:
GRIZ_SOCKET="$preview_dir/core.sock" ./target/debug/griz-core --stop
```

## Sharing an early test build (Apple Silicon)

Build a self-contained macOS app and tester zip with:

```sh
./scripts/package-macos.sh
```

Requires an Apple Silicon Mac, the same Rust/Xcode/Rune build setup as development,
and Python 3. The script builds **both** release executables for
`aarch64-apple-darwin`, checks for non-system runtime libraries, bundles the app
icon, records version/commit/build information, and includes dependency notices
and locked third-party sources. Output goes to ignored `artifacts/releases/`:

- `Griz-<version>-<commit>[-dirty]-macos-arm64.zip`
- A matching `.zip.sha256` checksum file.

The zip contains `Griz.app`, `BUILD-INFO.txt`, and a short tester guide. It needs
no Rust, Xcode, Ghostty installation, or sibling Rune checkout on the tester's
Mac. Send the zip/checksum directly or attach them to a manual GitHub prerelease.
There is no update server or automatic update mechanism.

By default, the bundle is **ad-hoc signed, not notarized**. This verifies bundle
integrity locally but does not make it trusted by Gatekeeper. Private testers may
need **System Settings → Privacy & Security → Open Anyway**. Do not ask them to
disable Gatekeeper globally. If you have a Developer ID certificate and a saved
`notarytool` keychain profile, the same packager can sign and notarize:

```sh
./scripts/package-macos.sh \
  --sign-identity 'Developer ID Application: Your Name (TEAMID)' \
  --notary-profile griz-notary
```

Signing happens inside-out, followed by signature verification. With a notary
profile, the script submits the app, staples and validates the ticket, checks
Gatekeeper acceptance, and only then creates the final zip. Apple credentials
are not stored in the repository. These signing/notarization steps require your
own credentials; the default ad-hoc path does not use Apple's notary service.

The default deployment target is macOS 13.0; override with
`MACOSX_DEPLOYMENT_TARGET` if needed. This is a build target, **not a tested OS
compatibility matrix**. Smoke-test the extracted app on the tester's macOS version.
Both Griz and Rune revisions/dirty states are recorded; prefer committed, clean
checkouts for reproducible tester builds. Source distributions include the locked
Cargo sources and license materials; review third-party obligations when changing
dependencies or widening distribution.

Finder/bundled launches start a new core in the **login account's home directory**
and use its configured shell, independent of Finder's current directory or missing
`SHELL`. Terminal sessions run interactive login shells so login PATH setup is
available before prompt plugins run. Direct development launches still inherit cwd and `$SHELL`. Existing
cores/sessions are not restarted or moved. Building/packaging never stops a core.

**Manual updates require saving work, quitting all GUIs, and stopping the old
core before replacing the app.** This ends all its sessions; a new GUI must not
attach to a mismatched old core. See [the tester guide](docs/EARLY-TESTING.md) for
installation, updating, feedback, diagnostics, and removal instructions.

## Workspaces and terminals

**Workspace → tabs → terminal panes.** Each tab starts with one terminal and can
contain nested horizontal and vertical splits.

A workspace is a **collection of terminals**, not a project or directory. For example,
“Daily” can contain a frontend shell, backend logs, a dotfiles editor, an SSH
connection, and a scratch shell, all in unrelated directories.

- The service starts with one workspace and terminal. Its automatic label shows
  the current directory's basename (for example, `griz` in `~/code/griz`).
- Click **+** beside Workspaces to create a workspace with its first terminal.
  The name is optional: blank or whitespace follows the selected terminal's
  **current** directory automatically. The first terminal starts in the current
  shell directory. With no selected terminal, the service’s starting directory is used.
- Automatic labels update as you `cd` or switch tabs. The home directory is `~`
  and the filesystem root is `/`. Hover a workspace to see its full directory path:
  `~/…` within your home directory, absolute elsewhere. Custom names stay fixed,
  but their hover path still updates. Clear a workspace's name through **Rename**
  to restore automatic tracking. Each window follows its own selected tab,
  including its remembered selection in inactive workspaces.
- The service checks live shell directories about every 250 ms while clients are
  attached; prompt hooks and OSC 7 are not required. Exited shells retain their last
  known directory. No directory polling runs without clients or workspaces.
- Click a workspace in the sidebar to switch. Your selected tab is remembered for
  each workspace on that connection.
- Click **+ Terminal** to open another independent shell. Its optional absolute
  starting directory applies only to that terminal. Blank uses the service's
  launch directory. You can also `cd` independently in any shell.
- Click tabs to switch layouts; click a pane's header or terminal area to focus it.
  Each pane's header has always-visible **↔**, **↕**, and **×** controls. Use **↔**
  to split that pane vertically (left/right), or **↕** to split it horizontally
  (top/bottom). **⌘D** and **⇧⌘D** split the focused pane. Splits divide the target
  pane equally and can be nested, up to eight panes per tab. Each pane has its own
  rounded frame, separated by a 10-point gap. The new shell inherits the target
  shell's current directory. All visible panes update independently, including unfocused ones.
- **Resize panes:** drag the gap between any two split branches, horizontally or
  vertically. Nested dividers resize only their branch; each visible shell reflows
  immediately. Double-click a divider to restore an even split. Proportions are local
  to this window and survive pane focus changes and zoom; new/rearranged splits start
  evenly sized. Minimum sizes keep headers and terminal content usable where space permits.
- **Rearrange panes:** drag the centered six-dot grip in a split pane's header.
  Drop in another pane's center to swap positions, or near its left/right/top/bottom
  edge to place the pane there. A tinted preview shows the destination before release.
  Escape cancels a move or restores a resize; dropping outside the panes does nothing.
  Moving preserves the existing shells and tab identity, and syncs the layout to other
  windows. Rearranging is within the current tab; restore a zoomed layout before dragging.
- **Zoom a pane:** click **⤢** in a split pane's header or press **⌘Enter** to
  make the focused pane fill the terminal content area without making the window
  fullscreen or hiding the sidebar/tabs. Other panes keep running; the zoom control
  gains a border in the pane's accent color while zoomed. Click **⤡** or press
  **⌘Enter** again to restore the original split layout. Pane-focus shortcuts still
  cycle through panes while zoomed. Zoom is remembered per tab in this window only;
  creating a split reveals the layout, and closing down to one pane clears zoom.
  **⌥⌘→ / ⌥⌘←** cycle pane focus within the current tab.
- Right-click any workspace for **Rename workspace…**,
  **Close workspace…**, and an **Accent color** swatch row: blue, purple, pink, orange,
  green, or graphite, using Rune’s accent palette. Green is the default. The workspace’s
  selected sidebar entry, active terminal tab, and pane outline/focus highlight share
  its accent; terminal ANSI colors are unchanged. Color changes sync to every connected
  window and survive GUI reconnects for the lifetime of the workspace in the service.
  Picking a color on an inactive workspace does not select it. Escape or clicking outside
  dismisses the menu. Rename changes only that workspace's name. Clearing the name uses
  that workspace’s selected terminal directory, even for an inactive workspace.
- **Close workspace** asks for confirmation, then removes the workspace and ends all
  of its terminals and foreground jobs, for every connected window. Affected windows
  switch to a neighboring workspace, keeping their remembered tab there. Closing the
  last workspace hides the terminal pane and shows only the **Create a workspace to
  get started** button, centered in the empty area. It uses normal UI font scaling
  and immediately creates a workspace with its directory-based default name, even
  with the sidebar hidden.
- The tab's **×** closes its focused terminal (or its first pane in an inactive tab).
  Every pane also has its own **×** in the header. Closing immediately ends only
  that terminal's shell and foreground job, without a confirmation dialog. The remaining
  panes expand into the freed space. Closing a tab's final pane removes the tab.
  **Only closing the final terminal in a workspace asks for confirmation**, using a
  simple **Close workspace?** prompt with the workspace name. It uses the same
  neighboring-workspace fallback as **Close workspace**. If no workspaces remain,
  use **+** to create one.
- A shell that exits stays visibly exited. **Start new shell** explicitly replaces
  only that ended shell, retaining the tab and its original starting directory.
  The service refuses to replace a live shell.

**Switching tabs/workspaces or closing the GUI never stops terminals.** Closing a
tab, closing a workspace, and explicitly stopping the service are deliberately
separate operations. Tab renaming/reordering are not implemented yet.

## Agents

The **Agents** section below Workspaces lists programs reporting status across
**all** workspaces, tabs, and split panes. It uses the
[Program Status Protocol (OSC 7501)](https://www.superlogical.com/rex/docs/build/program-status),
not screen scraping, process-name detection, or harness-specific extensions.

### No per-agent setup

Run a program that natively supports OSC 7501, such as current Pi:

```sh
pi
```

No `-e`, extension installation, wrapper command, or special environment variable
is required. The program queries the terminal, Griz answers, and the program
reports its state. Programs may also send reports without querying. Agents that
do not implement the protocol do not appear merely because their process is
running; Griz does not inject hooks or guess whether they are finished.

Rebuild both Griz processes and use a **new core** for this feature. See the
isolated preview instructions above to keep existing terminals undisturbed.
The earlier experimental Pi bridge and custom agent-reporting IPC are no longer
used or shipped.

### Harness logos

Agent cards and numbered tabs show bundled logos for **Pi**, **Claude Code**,
**Codex**, **Gemini CLI**, and **OpenCode**, based on the report's `app` identity.
Unknown programs get a generic terminal mark; ordinary shells show a terminal
icon beside their tab number. Quick Open uses the same terminal fallback when
no harness is reporting. Split tabs show each distinct harness once across all their panes,
including background panes and child records. Icons update when reports change
or clear, without changing selection, status, or notification behavior.
No logo font, downloads, or harness-specific setup is required. These icons do
not add agent detection: a program still needs to report OSC 7501 to appear.
Asset sources and notices are in [`assets/harnesses/NOTICE.md`](assets/harnesses/NOTICE.md).

### Status and attention

Reports retain **Working**, **Waiting** (protocol `blocked`), **Error**,
**Done**, or **Idle** status on individual cards without aggregate status headings.
Working cards have a thin, gently pulsing pale-blue gradient outline; Done cards
have a steady blue outline. A newly observed completion sends one soft blue wave
left to right across that card, fading out in 1.1 seconds. Already-completed records
on attachment/reconnection do not replay it. Reduce Motion skips the wave and
keeps the working outline still. Status remains available in each card's accessibility label. Record titles, messages, progress, and waiting reasons
come from the program. Click a record to jump to its exact terminal pane and
acknowledge its notification. Filled dots and a count mark unacknowledged
completion, input requests, or failures. Focusing a terminal through its tab,
workspace, split pane, or pane header also acknowledges that pane's unread
records, including when it is already selected. Other panes' notifications
remain unread. Background status updates alone do not acknowledge notifications.
Acknowledgements sync across windows and cannot dismiss a newer revision or a
replacement shell's result.

Each card's location uses **its own terminal pane's current directory**, not the
workspace's selection-dependent automatic name. Switching tabs, split focus, or
workspaces does not change other agents' locations. Custom workspace names remain
as additional context, and hovering shows the pane's full (home-abbreviated) path.
Child records share their owning pane's location; live directory changes update
only cards belonging to that pane.

The core consumes reports even when a pane is hidden or every GUI is closed.
Reopening shows the current records and unacknowledged results. Updates are
coalesced with normal frame delivery, not published on every status byte.
These are **in-app indicators**, not macOS notification-center alerts.

OSC 7501 supports a root record plus hierarchical child records in the same
pane. Griz keeps up to **64 records per terminal** (the protocol's required
minimum), evicting the least recently updated record at the cap. Each report
replaces that record completely; absent fields are not sticky. Child records
inherit `app` from the nearest ancestor that specifies one. Children reported
by a program are supported; independent headless process discovery is not.

A `clear` report removes its record and descendants; clearing the root or a full
terminal reset removes everything. A new shell prompt (OSC 133 `A`) or PTY
process exit drops Working/Waiting/Idle records, but **Done/Error results survive**
until the program clears/replaces them or the terminal is closed/restarted.
There are no heartbeats, idle timeouts, or PID-liveness guesses. Applications
can explicitly clear records on shutdown, so not every exit leaves a result.

libghostty-vt validates syntax, base64, control characters, and protocol limits.
Messages and titles are untrusted plain text; Griz disarms invisible formatting
controls outside the grid and always identifies the source workspace/pane.
Full protocol-sized messages stay in the terminal's record store; sidebar
message previews are capped at 512 UTF-8 bytes to bound catalog IPC.

### Try the protocol from a shell

```sh
printf '\033]7501;state=working:app=example:progress=40\033\\'
printf '\033]7501;state=done:app=example:msg=VGVzdHMgcGFzc2Vk\033\\'
printf '\033]7501;state=clear\033\\'
```

The second report's message decodes to “Tests passed”. Any application can use
the same protocol; it is not specific to coding agents.

## Two processes, independent lifetimes

- **`griz-core`** owns the workspace catalog and each terminal's PTY, interactive
  shell, and libghostty-vt emulator. It consumes output even when a terminal is
  unselected or no GUI is attached.
- **`griz`** is the disposable Rune client. It draws the active tab's panes and sends
  addressed commands. It owns no PTY and does not link libghostty in production.
- **`griz-protocol`** defines length-prefixed JSON over a private local
  Unix socket. It has no Rune or terminal-library dependencies.
- **`griz-vt`** wraps the official C API through a small C shim compiled against the
  release headers. Styled cells come from RenderState, not plain-text formatting.

A single service actor owns and mutates all terminal emulators. Blocking socket and
PTY I/O runs on worker threads, with bounded channel handoffs. Terminal IDs remain
stable and are not reused while the service lives. Shell replacement also changes
a generation number, so late output from the old shell cannot corrupt the new one.

Each GUI connection owns **its own selection**. Switching one window does not
switch another. Every terminal operation includes a terminal ID; it never means
“whichever shell is currently selected.” Selection commands have serial numbers:
the GUI suppresses typing until it receives the acknowledged new selection, so
queued old pictures cannot redirect keystrokes into the wrong tab.

The service sends a consistent catalog plus every visible pane's complete
styled screen. PTY-output publications are coalesced to roughly 60 per second;
command acknowledgements and catalog changes can publish immediately. Only visible
terminals need screen extraction; every background terminal still parses all its
output. A slow client retains only its newest pending view, with a separate bounded
error queue. The service blocks when idle; the GUI does not continuously poll frames.

This is intentionally simpler than Rex's per-client raw-byte streaming design:
terminal emulation and split layouts remain authoritative in the service. More
efficient frame encoding can be added behind the same ownership boundary.

## Settings and keyboard shortcuts

Open **Settings…** with **⌘,**, **Open Settings…** in the command palette, or the macOS app
menu. Settings opens as an in-app modal panel, including while disconnected.

- **Appearance:** choose Menlo, Monaco, or the system monospace terminal font;
  adjust the app-wide font size from 10–28 pt; choose Dark or Midnight. Font size
  scales sidebar text, tabs, buttons, dialogs, Settings, and terminals together,
  preserving the UI's type hierarchy. Changes apply immediately, including terminal
  cell geometry and PTY dimensions. **⌘+ / ⌘−** increase/decrease size anywhere in
  the app, including Settings and text fields (**⌘=** also increases size).
  Themes change the interface and terminal base background. **Terminal whites**
  defaults to **Soft** (`#D8D8D8` white); choose **Softer** (`#C8C8C8`) or **Original**
  (unmodified colors). Soft modes compress bright grayscale foregrounds, including
  explicit ANSI/RGB whites from applications, while preserving dim grays (up to
  `#A0A0A0`), chromatic colors, and all backgrounds. The local, live adjustment also
  applies to terminal previews; it never changes terminal data or copied text.
  **Default workspace accent** starts at green; choose blue, purple, pink, orange,
  green, or graphite for workspaces created from this GUI. Existing workspaces keep
  their own colors, which can still be changed from their context menu. The default
  is saved locally with the other appearance preferences; the chosen color becomes
  shared workspace state when a workspace is created. A fresh core's initial workspace
  uses the first attaching GUI's saved default too, before its first view is shown.
  Reconnecting to an existing core never overrides workspace colors.
  Regular action buttons use a fixed **Soft** style with a subtle neutral fill,
  rounded corners, and consistent hover, press, and keyboard focus states. There
  is no button-style picker; older style preferences are ignored.
- **Shortcuts:** search actions, click a binding, then press a new combination.
  Conflicts are rejected; **Clear** disables a binding and **Reset** restores its
  default. Escape cancels recording before dismissing Settings. Shortcuts require
  Command, Control, or Option; Settings and native editing/system shortcuts are reserved.
  Font sizing, closing the active terminal, and opening either palette are app-wide
  commands; other shortcuts only run with terminal focus. Closing is suppressed
  while a modal is open.
- **Prefix bindings:** a second, independently editable binding sits alongside each
  shortcut. The **Prefix key** row defaults to **Ctrl+B**; press and release it,
  then press the action key. The status bar shows when Griz is waiting. Defaults:
  **p / n** select the previous/next workspace, **1–9** select a workspace by its
  sidebar position, **< / >** select the previous/next tab, **" / %** split
  horizontally/vertically, **h / j / k / l** focus the pane left/down/up/right,
  and **z** toggles pane zoom. Directional focus does not wrap at the edge.
  Prefix sequences only run with terminal focus, not in Settings, dialogs, or
  palettes. **Escape** or an unknown key cancels without typing into the shell;
  press the prefix twice to send the prefix itself through to a nested TUI.
  Clear/Reset work independently for each binding and for the prefix key.
- Defaults: **⌘T** opens a terminal using the service default directory,
  **⌘W** closes the **active terminal** (confirming only for a workspace's final
  terminal), **⇧⌘N** opens New Workspace,
  **⌘B** toggles the sidebar, and **⇧⌘→ / ⇧⌘←** cycle through tabs.
  **⌘D / ⇧⌘D** split vertically/horizontally, **⌥⌘→ / ⌥⌘←** cycle pane focus,
  and **⌘Enter** toggles pane zoom. **Toggle pane zoom** is also available in the
  command palette and can be rebound or disabled in Settings (`pane.toggle_zoom`).
  Closing targets the focused pane's captured terminal ID, not a tab or window.
  The Settings shortcut **⌘,** stays fixed so Settings is always reachable.

### Terminal mouse and copying

- Mouse wheel and trackpad scrolling browse the terminal's retained history in either
  direction. Each window/pane has its own scroll position; incoming output keeps your
  place, and typing returns to the live terminal. Hold **Shift** while scrolling to
  browse terminal history instead of sending wheel events to a mouse-aware program.
  Alternate-screen programs have no terminal history of their own.
- Drag across terminal text to highlight it. Releasing the mouse automatically copies
  the selection and shows a **Copied to clipboard** toast near the bottom for two seconds.
  Double-click selects a word; triple-click selects a visible line. Wide Unicode and
  combining characters are copied intact. **⌘C** or **Ctrl+Shift+C** copies it again;
  **Escape** clears the highlight without sending Escape to the shell.
- Programs that enable terminal mouse reporting receive primary/middle clicks and drags,
  pointer motion, secondary clicks, and wheel/trackpad scrolling. Encoding follows the
  program's current X10, UTF-8, SGR, URXVT, or SGR-pixel mode through libghostty.
  Cell-based modes suppress repeated motion within the same cell, so pointer jitter does
  not cancel TUI clicks (including Pi's logo animation). Pixel mode retains pixel motion.
  Hold **Shift** while dragging to select/copy text instead of sending mouse events.
- Selection is local to this window and pane; it never types the selected text into the
  shell. Changing the screen contents/grid or navigating away clears stale highlights.
  Cached text remains selectable after disconnect or shell exit.

### Quick navigation and command palette

- Click the wide **search control** in the titlebar or press **⌘P** to open
  **Jump To** across every workspace and terminal tab. The control shows the current
  shortcut, follows rebindings, and remains clickable if the shortcut is disabled.
  The sidebar icon beside the workspace **+** button collapses the sidebar to a thin
  left-hand column containing just the toggle; click it again to reopen.
  Search workspace
  names, terminal numbers, and each terminal’s **current** shell directory, including
  inactive tabs. Terminal rows use their own directory, not the workspace’s selected
  tab’s automatic name or the terminal’s starting directory. Directory changes update
  live, and exited shells keep their last known path. Matches are case-insensitive and allow abbreviations and multiple search words.
  Workspace results restore that workspace’s remembered tab; terminal results select
  that exact terminal, including when it belongs to another workspace.
  The highlighted result shows a **live, read-only preview** of its tab, including
  split panes. Workspace previews use your remembered tab and focused pane;
  terminal previews highlight the exact pane. Browsing never switches workspaces,
  resizes terminals, or sends input. The preview sits beside results in wide windows
  and below them in narrower windows; very short windows keep the results-only layout.
- **⇧⌘P** opens **Commands**: create a workspace here (using the selected shell’s
  current directory), open terminals, choose a terminal starting directory, rename
  or close the current workspace, split panes, cycle pane focus, close the active terminal, cycle tabs, toggle the
  sidebar, adjust font size, and open Settings. Reconnect is available while
  disconnected; Start new shell appears for an exited shell. Destructive commands
  follow the same close rules: workspace closes and final-terminal closes ask for
  confirmation; other terminal closes are immediate.
- In both modes, type to filter, use **Ctrl-N / Ctrl-P** or **↓ / ↑** to cycle
  through results (wrapping at either end), and **Enter** to select. **Escape** or
  a click outside dismisses without running anything. Typing, paste, and search
  navigation never go to the shell while the palette is open.
- Opening the other mode resets the query. Both opener shortcuts can be changed or
  disabled in Settings → Shortcuts. On non-macOS platforms the primary modifier is
  Control instead of Command. Within an open palette, Ctrl-P navigates results.
- Session actions are disabled while disconnected or waiting for a selection
  acknowledgement, but Settings and local appearance commands remain available.
  Jump To includes individual terminals inside split layouts; selecting one opens
  its containing tab and focuses that exact pane.

Preferences save automatically through Rune's atomic JSON store. On macOS the file
is `~/Library/Application Support/griz/prefs.json`; on Linux it is
`$XDG_CONFIG_HOME/griz/prefs.json` (falling back to `~/.config/griz/prefs.json`).
There is no need to edit it. A save failure is shown in the panel while the change
remains active for this run. A malformed JSON file is preserved as `prefs.json.bad`;
invalid individual preferences use safe fallbacks and show a warning in Settings.
Preferences are loaded when a GUI starts; already-open GUI processes do not live-sync
settings with each other. Offline `--frame` captures and GUI tests use in-memory
preferences and never write the user's file.

These are GUI preferences, not workspace/session persistence. Shell defaults are
still configured through `griz-core`'s launch options below; changing appearance or
shortcuts never restarts running shells.

## Session lifetime and configuration

The GUI attaches to an existing core or starts the `griz-core` executable beside it
in a detached process session. Direct development launches use the current directory
and `$SHELL` (falling back to `/bin/sh`). The bundled `.app` starts a new core in the
login account's home directory with its configured shell. Each shell runs as an
**interactive login shell** (`-l -i`), so login configuration runs before prompt
configuration. For zsh this includes `.zprofile` before `.zshrc`; for bash it
includes the normal login profile (`.bash_profile`, `.bash_login`, or `.profile`).
This lets Homebrew and other PATH setup work from Finder, without hard-coded
package-manager paths. Bash users who keep interactive settings in `.bashrc`
should source it from their login profile, as with other login-shell terminals. `GRIZ_SESSION` contains its terminal ID.

**Quitting the GUI only detaches.** Reopening returns to the same workspaces and
shells, including their variables, directories, running jobs, and terminal state.
A new GUI connection initially selects the first workspace and its first tab;
**Reconnect** in an existing window restores its selected tab if the same core and
tab still exist. Selection history is not saved to disk.

The endpoint, instance lock, and service log live under `~/.local/state/griz/`.
The directory is private (0700), and the socket is 0600. A file lock prevents two
services from replacing one another. `GRIZ_SOCKET` overrides the endpoint; its
parent must be a private, user-owned directory.

To **explicitly stop the service and end all its terminal shells**:

```sh
./target/debug/griz-core --stop
```

To choose the default shell/directory, start the service before the GUI:

```sh
./target/debug/griz-core --shell /bin/zsh --cwd "$HOME/code/griz"
```

This directory is a default for new terminals, **not a workspace directory**.
Persistence is in memory: stopping/crashing the core or rebooting ends its sessions
and loses the catalog. No launchd integration or reboot restoration is installed.

## What works

- Optionally named, renameable workspaces and independently running terminal tabs.
- Real local shells with input, cursor, ANSI/truecolor backgrounds and text,
  bold/italic/faint, conceal, blink, strike/overline, colored underline variants,
  Unicode cells, and alternate-screen state. Conceal hides rendering, not copied text.
- Cell-fitted block elements and solid light/heavy box-drawing strokes (including
  mixed-weight junctions and half-line caps), so TUI logos and borders join without
  font-leading gaps. The same geometry scales into navigation previews.
- Window/sidebar resize updates libghostty and every visible pane's PTY (SIGWINCH).
- Enter, Shift+Enter (CSI-u, for multiline input in Pi and compatible TUIs), Backspace,
  Tab, arrows (including application cursor mode), common navigation and function keys,
  Ctrl-C/Ctrl-D, terminal-aware clipboard paste, and basic IME preedit/commit. Paste uses
  live Ghostty modes; unsafe unbracketed multiline requires a count-only confirmation.
- Terminal mouse reporting, visible-grid selection, automatic clipboard copy on release,
  and a transient bottom copy confirmation.
- Detach/reattach, background output, workspace/final-terminal close confirmation,
  ended-shell recovery.

## Current limits

- No tab reordering/renaming, pane dragging across tabs/windows, or remote transport yet.
- No draggable scrollback scrollbar, images, hyperlinks, advanced terminal keyboard protocols,
  or full font/glyph shaping. Selection covers the visible grid; copied multi-row text
  uses newlines between visual rows (soft-wrapped logical lines are not yet distinguished).
  Secondary clicks are reported as press/release pairs; right-button dragging is not yet exposed by Rune.
- Fixed-cell monospace rendering, not Ghostty's full renderer; basic shell/TUI use is the target.
  SGR decorations use cell geometry; dashed, double-line, rounded, and diagonal box-drawing
  glyphs still use the selected font.
  Shade blocks use uniform 25/50/75% foreground coverage rather than stippled patterns.
- Full snapshots, not dirty-cell deltas. Limits: 16 workspaces, 64 terminals total,
  16 clients, eight panes per tab, 300 × 120 cells per grid, 256 KiB per input or raw
  UTF-8 paste, one pending paste confirmation per client, and 16 MiB per IPC message.
- Starting directories must be absolute existing paths; no file picker or `~` expansion.
- Multiple clients may view one terminal but share its dimensions; the last
  selection/resize wins. Different terminals have independent dimensions.
- Closing a tab terminates its shell and foreground process group; this is not a
  general process-tree supervisor for deliberately detached/nohup jobs.

## Check

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Integration tests use isolated real services, PTYs, and shells. They cover terminal
state/directory isolation, output while unselected/detached, independent client
selections, remembering tabs across workspace switches, creation/rename validation,
resize, stale IDs, continuous output, explicit close/foreground termination,
ended-shell replacement, request validation, and single-instance ownership.
GUI tests cover navigation, dialogs, paste confirmation/cancellation, toolbar focus,
input routing while selection acknowledgements are pending, empty workspaces, and
ended-shell recovery.
Split tests cover nested layouts, both directions, rounded pane spacing, per-pane
resizing, focus/input acknowledgement safety, cwd inheritance, sibling output,
multi-client selection, layout collapse, stable tab IDs, and pane limits. Pane interaction
tests cover both resize axes, nested dividers, minimum sizes, centered drag grips,
swap/edge drop previews, cancellation, topology changes, and input gating. Core move
tests verify process preservation, directional reparenting, and invalid/stale targets.
Mouse tests cover mode-specific encoding, real PTY routing, split-pane targeting,
Unicode/reverse/multiline selection, copy-on-release and toast expiry, Shift override,
copy shortcuts, cancellation, scrolling, and stale-screen/disconnected handling.
Navigation preview tests cover live hidden-pane output, subscription isolation,
unchanged selection/dimensions, stale targets, dismissal, and fitted split miniatures.
Agent tests cover native OSC 7501 negotiation/DA ordering, arbitrary PTY chunking,
metadata validation, bounded hierarchical records, field replacement/inheritance,
clear/reset/prompt/exit lifetimes, hidden/multi-client/detached reporting,
revision/generation-safe acknowledgements, and exact-pane navigation/input gating.
Other tests exercise libghostty's colors/cursor/Unicode/alternate screen/query
responses and Rune's cell placement.

### Input latency measurements

Measure local PTY echo through a temporary, isolated core (existing sessions are
never attached to or stopped):

```sh
cargo build --workspace
python3 scripts/bench_input.py --binary target/debug/griz-core \
  --size 80x24 --size 160x48 --samples 60 \
  --output artifacts/input-latency.json
# Verify typing after subscribing/unsubscribing a palette preview:
python3 scripts/bench_input.py --binary target/debug/griz-core --preview-cycle
# CPU-side GUI dispatch/apply/frame timings, including a densely filled terminal:
cargo test -p griz input_latency_gui_cpu -- --ignored --nocapture --test-threads=1
```

Repeat `--binary` to compare saved builds. The script waits for an actual cursor
change, not just an acknowledgement, and reports complete socket-frame arrival
separately from Python JSON decoding. GUI timings exclude GPU submission and
physical display scanout; neither benchmark is a key-to-photon measurement.

For an offline GUI capture, save a current **`view` event** (including its catalog
and selected terminal) into a JSON file:

```sh
cargo run -p griz -- --frame artifacts/demo-view.json \
  --snap artifacts/workspaces.png --tree artifacts/workspaces.json
```

Offline captures never attach, send input, or reflow. Live resize happens in the
core. Generated captures stay in ignored `artifacts/`.

## References

- [Rune hello example](https://github.com/anomalyco/rune/tree/dev/examples/hello)
- [Rex public testing](https://www.superlogical.com/updates/public-testing-beginning)
- [libghostty-vt API and RenderState](https://lib.ghostty.org/vt/first-terminal.html)
