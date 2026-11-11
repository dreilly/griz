# Griz — early tester guide

Thanks for trying Griz! This is an experimental native terminal workspace app
for **Apple Silicon Macs**. Use it for disposable work initially, not as the only
terminal for an important job. You do not need Rust, Xcode, Ghostty, or the source
checkout to run the supplied app. Your shells, tools, and agent subscriptions
are still your own.

## Install and open

1. Unzip the download and drag **Griz.app** into **Applications**.
2. Open Griz from Finder. A newly started service uses your account's home
   directory (`~`) and configured shell. Sessions are interactive login shells:
   for zsh, `.zprofile` runs before `.zshrc`, including your normal Homebrew/PATH
   setup. An already-running service keeps its existing terminals and defaults.
3. For a private, ad-hoc-signed build, macOS may say it cannot verify the developer.
   If you trust this particular download, try opening it, then go to **System
   Settings → Privacy & Security → Open Anyway** and confirm. Do not disable
   Gatekeeper globally. If approval is unavailable or macOS reports malware or
   a damaged app, contact the sender rather than removing security protections.

The release targets macOS 13 or newer, but the deployment target is not a promise
that every version has been tested. Include your exact macOS version in feedback.
`BUILD-INFO.txt` records the build machine's OS and deployment target.

## Things to try

- Create and switch workspaces; open and close terminal tabs.
- Split panes with **⌘D** / **⇧⌘D** and drag the gaps to resize them; double-click
  a divider to return to an even split. Try nested splits and resizing the window.
- Drag a pane's centered six-dot header grip onto another pane: drop in the center
  to swap, or near an edge to place it left/right/above/below. Check the preview;
  **Escape** cancels a move or restores a resize. Existing shells should keep running.
- Zoom a pane with **⌘Enter**, then restore it and check that its sizes are remembered.
  Run your normal shell tools and try changing pane focus while they produce output.
- Use **⌘P** to search, **⇧⌘P** for commands, and **⌘,** for settings.
- Select text, copy and paste, scroll through history, and use a terminal editor.
- Quit and reopen Griz: the same terminal sessions should still be there.
- Run a current Pi release to try agent status, notifications, and harness logos.
  Programs must report OSC 7501 to appear in Agents; a logo does not add detection.

## Important session behavior

**Quitting the app does not stop its terminals.** A background `griz-core` service
keeps them alive. Closing a pane/workspace does end the affected jobs.

Session state lives in memory, not on disk. A reboot, core crash, or explicit
service shutdown ends the sessions and loses the workspace catalog. There is no
reboot restoration, auto-updater, or installed login/background service.

To explicitly end **all** Griz sessions, first finish/save your work, then quit
Griz and run this from another terminal:

```sh
/Applications/Griz.app/Contents/MacOS/griz-core --stop
```

If Griz is installed elsewhere, adjust that path. A "No such file" or
"Connection refused" socket error may mean the service is already stopped.

## Updating manually

There is no update server. You will receive another zip when a new build is ready.

1. Finish/save work in every Griz pane. **Updating requires ending all sessions.**
2. Quit every Griz window/app instance.
3. Run the `griz-core --stop` command above using the **old app**, before replacing it.
4. Replace Griz.app in Applications with the new copy, then reopen it.

Do not mix a new GUI with a still-running old core: the protocol is pre-release
and versions need to match. GUI preferences normally survive replacement.

## Tools missing compared with another terminal

If Griz says `command not found` for tools installed on this Mac, compare these
commands in Griz and in your working terminal:

```sh
printf 'shell=%s\nPATH=%s\n' "$SHELL" "$PATH"
command -v brew starship fzf nvim
```

For zsh, `[[ -o login ]] && echo login-shell` should print `login-shell`. In an
older build that skipped login startup, `exec /bin/zsh -l` can confirm the issue
by starting a login shell in that pane. Prefer the corrected build rather than
adding duplicate or hard-coded PATH entries. Save work and stop the old core
before replacing the app; an existing core continues launching its old behavior.
If `nvim` is found but fails after starting, include the exact error separately:
that may be a terminal compatibility problem rather than command lookup.

## Reporting a problem

Send the sender:

- `BUILD-INFO.txt` from the download (version, commit, and build details).
  It is also inside Griz.app → **Show Package Contents** → Contents → Resources.
- macOS version and Mac model/chip.
- Steps to reproduce, what you expected, and what actually happened.
- A screenshot or short recording, if useful and safe to share.
- Relevant recent lines from `~/.local/state/griz/core.log` if the service failed.
  For a crash, a matching Griz report may be in Console or
  `~/Library/Logs/DiagnosticReports/`.

Review logs/screenshots before sharing: terminal contents, commands, paths, and
error messages can contain credentials, customer data, or other private material.
The app does not upload diagnostics automatically.

## Removing Griz

Finish/save your work, quit Griz, stop the core with the command above, then delete
Griz.app. Optional leftover local data:

- `~/.local/state/griz/` — socket, lock, and service log (remove only after stopping).
- `~/Library/Application Support/griz/` — GUI preferences.

Third-party license notices and locked dependency sources are included inside the
app at Contents → Resources → ThirdPartyNotices / ThirdPartySources.zip.
