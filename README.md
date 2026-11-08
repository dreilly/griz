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
and IPC compatibility is not maintained. Rebuild both processes after changes.

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

## Workspaces and terminals

**Workspace → tabs → terminals.** For now, each tab contains exactly one terminal.

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
- Click tabs to switch terminals. Right-click any workspace for **Rename workspace…**,
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
- **Close tab** asks for confirmation, then ends that terminal's shell and
  foreground job. Other terminals are untouched. Closing the last tab also closes
  its workspace for every connected window, using the same neighboring-workspace
  fallback as **Close workspace**. If no workspaces remain, use **+** to create one.
- A shell that exits stays visibly exited. **Start new shell** explicitly replaces
  only that ended shell, retaining the tab and its original starting directory.
  The service refuses to replace a live shell.

**Switching tabs/workspaces or closing the GUI never stops terminals.** Closing a
tab, closing a workspace, and explicitly stopping the service are deliberately
separate operations. Tab renaming/reordering and split panes are not implemented yet.

## Two processes, independent lifetimes

- **`griz-core`** owns the workspace catalog and each terminal's PTY, interactive
  shell, and libghostty-vt emulator. It consumes output even when a terminal is
  unselected or no GUI is attached.
- **`griz`** is the disposable Rune client. It draws the selected terminal and sends
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

The service sends a consistent catalog plus the selected terminal's complete
styled screen. PTY-output publications are coalesced to roughly 60 per second;
command acknowledgements and catalog changes can publish immediately. Only visible
terminals need screen extraction; every background terminal still parses all its
output. A slow client retains only its newest pending view, with a separate bounded
error queue. The service blocks when idle; the GUI does not continuously poll frames.

This is intentionally simpler than Rex's per-client raw-byte streaming design:
terminal emulation remains authoritative in the service. Splits and more efficient
frame encoding can be added behind the same ownership boundary.

## Settings and keyboard shortcuts

Open **Settings…** with **⌘,**, the titlebar Settings button, or the macOS app
menu. Settings opens as an in-app modal panel, including while disconnected.

- **Appearance:** choose Menlo, Monaco, or the system monospace terminal font;
  adjust the app-wide font size from 10–28 pt; choose Dark or Midnight. Font size
  scales sidebar text, tabs, buttons, dialogs, Settings, and terminals together,
  preserving the UI's type hierarchy. Changes apply immediately, including terminal
  cell geometry and PTY dimensions. **⌘+ / ⌘−** increase/decrease size anywhere in
  the app, including Settings and text fields (**⌘=** also increases size).
  Themes change the interface
  and terminal base background; applications' explicit ANSI colors are preserved.
- **Buttons:** preview the empty-state workspace button in Soft, Outline, Solid,
  Ghost, Pill, or Accent styles. Click a sample to select and save it without creating
  a workspace. Soft is the neutral default; only Accent uses the green tint.
  Samples include the real hover/press/focus behavior and match the actual button.
- **Shortcuts:** search actions, click a binding, then press a new combination.
  Conflicts are rejected; **Clear** disables a binding and **Reset** restores its
  default. Escape cancels recording before dismissing Settings. Shortcuts require
  Command, Control, or Option; Settings and native editing/system shortcuts are reserved.
  Font sizing and closing the active terminal are app-wide commands; other actions
  only run with terminal focus. Closing is suppressed while a modal is open.
- Defaults: **⌘T** opens a terminal using the service default directory,
  **⌘W** confirms closing the **active terminal**, **⇧⌘N** opens New Workspace,
  **⌘B** toggles the sidebar, and **⇧⌘→ / ⇧⌘←** cycle through tabs.
  Closing targets a captured terminal ID, not a tab or window. When split panes are
  added, this action will target the focused terminal within the tab.
  The Settings shortcut **⌘,** stays fixed so Settings is always reachable.

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
in a detached process session. The first service launch uses the current directory
and `$SHELL` (falling back to `/bin/sh`). Each shell runs interactively and reads
its normal startup configuration. `GRIZ_SESSION` contains its terminal ID.

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
  bold/italic/underline, Unicode cells, and alternate-screen state.
- Window/sidebar resize updates libghostty and the selected PTY (SIGWINCH).
- Enter, Backspace, Tab, arrows (including application cursor mode), common navigation
  and function keys, Ctrl-C/Ctrl-D, clipboard paste, and basic IME preedit/commit.
- Detach/reattach, background output, explicit close confirmation, ended-shell recovery.

## Current limits

- No tab reordering/renaming, splits, or remote transport yet.
- No scrollback UI, selection/copy, mouse reporting, images, hyperlinks,
  advanced terminal keyboard protocols, or full font/glyph shaping.
- Fixed-cell monospace rendering, not Ghostty's full renderer; basic shell/TUI use is the target.
- Full snapshots, not dirty-cell deltas. Limits: 16 workspaces, 64 terminals total,
  16 clients, 300 × 120 cells per grid, 256 KiB per input/paste, and 16 MiB per IPC message.
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
GUI tests cover navigation, dialogs, paste, toolbar focus, input routing while
selection acknowledgements are pending, empty workspaces, and ended-shell recovery.
Other tests exercise libghostty's colors/cursor/Unicode/alternate screen/query
responses and Rune's cell placement.

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
