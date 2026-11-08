//! Native Rune client. Workspaces, PTYs and terminal emulators live in griz-core.
mod client;
mod input;
mod settings;
mod settings_ui;
mod terminal_view;
mod ui;

use griz_protocol::{Command, Event, Screen, TerminalId, ViewState, WorkspaceId, WorkspaceInfo};
use rune::{App, AppSpec, Ime, KeyEvent, TextEdit, Titlebar, Tx, Waker};
use settings::{Action, Settings};
use ui::view;

const TITLEBAR_HEIGHT: f32 = 44.0;

#[derive(Clone)]
enum Dialog {
    NewWorkspace,
    RenameWorkspace(WorkspaceId),
    CloseWorkspace(WorkspaceId, String),
    NewTerminal(WorkspaceId),
    CloseTerminal(TerminalId, String),
}

struct State {
    sidebar_open: bool,
    snapshot: Option<ViewState>,
    client: Option<client::Client>,
    wake: Option<Waker>,
    grid: (u16, u16),
    status: String,
    notice: Option<String>,
    preedit: String,
    offline: bool,
    online: bool,
    /// Ignore older queued pictures and suppress typing until this selection is acknowledged.
    pending: Option<u64>,
    restore: Option<(u32, WorkspaceId, Option<TerminalId>)>,
    dialog: Option<Dialog>,
    draft: TextEdit,
    settings: Settings,
    prefs: rune::prefs::Prefs,
    settings_open: bool,
    settings_page: settings_ui::Page,
    recording: Option<Action>,
    settings_error: Option<String>,
    shortcut_search: TextEdit,
    viewport_size: Option<(f32, f32)>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            sidebar_open: true,
            snapshot: None,
            client: None,
            wake: None,
            grid: (80, 24),
            status: "Connecting to local service…".into(),
            notice: None,
            preedit: String::new(),
            offline: false,
            online: false,
            pending: None,
            restore: None,
            dialog: None,
            draft: TextEdit::new(""),
            settings: Settings::default(),
            prefs: rune::prefs::Prefs::memory(),
            settings_open: false,
            settings_page: settings_ui::Page::Appearance,
            recording: None,
            settings_error: None,
            shortcut_search: TextEdit::new(""),
            viewport_size: None,
        }
    }
}

impl State {
    fn open_settings(&mut self, tx: &mut Tx) {
        self.dialog = None;
        self.preedit.clear();
        self.recording = None;
        self.settings_open = true;
        tx.blur();
    }

    fn close_settings(&mut self, tx: &mut Tx) {
        self.settings_open = false;
        self.recording = None;
        tx.blur();
    }

    fn settings_changed(&mut self) {
        self.settings_error = self.settings.save(&mut self.prefs).err();
        if let Some((width, height)) = self.viewport_size {
            let grid = terminal_view::grid_for(width, height, &self.settings);
            if grid != self.grid {
                self.grid = grid;
                self.resize();
            }
        }
    }

    fn change_font_size(&mut self, delta: f32) {
        let size = (self.settings.font_size + delta).clamp(10.0, 28.0);
        if size != self.settings.font_size {
            self.settings.font_size = size;
            self.settings_changed();
        }
    }

    fn bind_shortcut(&mut self, action: Action, chord: Option<rune::Chord>) {
        match self.settings.bind(action, chord) {
            Ok(()) => {
                self.recording = None;
                self.settings_changed();
            }
            Err(error) => self.settings_error = Some(error),
        }
    }

    fn act(&mut self, action: Action, tx: &mut Tx) {
        let font_action = matches!(action, Action::IncreaseFontSize | Action::DecreaseFontSize);
        if !font_action && (self.settings_open || self.dialog.is_some()) {
            return;
        }
        match action {
            Action::IncreaseFontSize => self.change_font_size(1.0),
            Action::DecreaseFontSize => self.change_font_size(-1.0),
            Action::ToggleSidebar => self.sidebar_open = !self.sidebar_open,
            Action::NewWorkspace => self.open_dialog(Dialog::NewWorkspace, tx),
            Action::NewTerminal => {
                if let Some(workspace) = self.workspace() {
                    self.change(Command::CreateTerminal {
                        workspace_id: workspace.id,
                        cwd: None,
                    });
                }
            }
            Action::CloseTerminal => {
                if let Some(id) = self.active_terminal() {
                    let number = self
                        .workspace()
                        .and_then(|w| w.terminals.iter().position(|t| t.id == id))
                        .map(|i| i + 1)
                        .unwrap_or(1);
                    // Capture the active terminal's ID, never a tab/container ID. When splits
                    // arrive, active_terminal() will follow the focused pane instead.
                    self.open_dialog(Dialog::CloseTerminal(id, format!("terminal {number}")), tx);
                }
            }
            Action::NextTab | Action::PreviousTab => {
                if let Some(workspace) = self.workspace() {
                    let len = workspace.terminals.len();
                    if len > 0 {
                        let current = workspace
                            .terminals
                            .iter()
                            .position(|t| Some(t.id) == self.active_terminal())
                            .unwrap_or(0);
                        let next = if action == Action::NextTab {
                            (current + 1) % len
                        } else {
                            (current + len - 1) % len
                        };
                        self.change(Command::SelectTerminal {
                            terminal_id: workspace.terminals[next].id,
                        });
                    }
                }
            }
        }
        if !font_action {
            tx.blur();
        }
    }

    fn screen(&self) -> Option<&Screen> {
        self.snapshot.as_ref()?.terminal.as_ref().map(|t| &t.screen)
    }

    fn active_terminal(&self) -> Option<TerminalId> {
        self.snapshot.as_ref()?.active_terminal
    }

    fn workspace(&self) -> Option<&WorkspaceInfo> {
        let snapshot = self.snapshot.as_ref()?;
        snapshot
            .workspaces
            .iter()
            .find(|w| w.id == snapshot.active_workspace)
    }

    fn shell_exited(&self) -> bool {
        self.snapshot
            .as_ref()
            .and_then(|s| s.terminal.as_ref())
            .is_some_and(|t| t.exited.is_some())
    }

    fn ready(&self) -> bool {
        self.online && !self.offline && self.pending.is_none()
    }

    fn connect(&mut self) {
        if self.offline {
            return;
        }
        if let Some(wake) = &self.wake {
            self.restore = self
                .snapshot
                .as_ref()
                .map(|s| (s.core_pid, s.active_workspace, s.active_terminal));
            self.client = Some(client::Client::start(self.grid, wake.clone()));
            self.pending = None;
            self.online = false;
            self.preedit.clear();
            self.notice = None;
            self.status = "Connecting to local service…".into();
        }
    }

    fn send(&mut self, command: Command) -> Option<u64> {
        if self.offline || !self.online {
            return None;
        }
        let client = self.client.as_mut()?;
        match client.send(command) {
            Ok(serial) => Some(serial),
            Err(error) => {
                self.notice = Some(error);
                None
            }
        }
    }

    fn change(&mut self, command: Command) {
        if !self.ready() {
            return;
        }
        self.notice = None;
        if let Some(serial) = self.send(command) {
            self.pending = Some(serial);
            self.preedit.clear();
        }
    }

    fn resize(&mut self) {
        if self.ready()
            && let Some(terminal_id) = self.active_terminal()
        {
            self.send(Command::Resize {
                terminal_id,
                cols: self.grid.0,
                rows: self.grid.1,
            });
        }
    }

    fn input(&mut self, bytes: Vec<u8>) {
        // A click may race a previously queued screen. Never type into the old tab.
        if self.settings_open || self.dialog.is_some() || self.pending.is_some() || self.offline {
            return;
        }
        if !self.online {
            self.notice = Some("Not connected. Click Reconnect.".into());
            return;
        }
        if self.shell_exited() {
            self.notice = Some("Shell has exited. Click Start new shell above.".into());
            return;
        }
        if bytes.len() > griz_protocol::MAX_INPUT {
            self.notice = Some("Input rejected: paste limit is 256 KiB".into());
            return;
        }
        if let Some(terminal_id) = self.active_terminal() {
            self.send(Command::Input {
                terminal_id,
                data: bytes,
            });
        }
    }

    fn apply(&mut self, snapshot: ViewState) {
        if self.pending.is_some_and(|serial| snapshot.ack < serial) {
            return;
        }
        let selection_changed = self.pending.is_some()
            || !self.online
            || self.active_terminal() != snapshot.active_terminal;
        self.pending = None;
        self.online = !self.offline;
        let old_terminal = self.active_terminal();
        self.status = match &snapshot.terminal {
            Some(t) => match &t.exited {
                Some(status) => format!("Shell exited: {status} · use Start new shell"),
                None => format!(
                    "Connected · core {} · shell {} · {} × {}",
                    snapshot.core_pid, t.shell_pid, t.screen.cols, t.screen.rows
                ),
            },
            None if snapshot.workspaces.is_empty() => {
                "No workspaces. Create a workspace with + in the sidebar to get started.".into()
            }
            None => "No terminals in this workspace. Open a terminal to get started.".into(),
        };
        self.snapshot = Some(snapshot);
        if self.shell_exited() || old_terminal != self.active_terminal() {
            self.preedit.clear();
        }
        if let Some((core_pid, workspace_id, terminal_id)) = self.restore.take()
            && self.snapshot.as_ref().is_some_and(|s| {
                s.core_pid == core_pid && s.workspaces.iter().any(|w| w.id == workspace_id)
            })
        {
            self.change(Command::SelectWorkspace { workspace_id });
            if let Some(terminal_id) = terminal_id
                && self.snapshot.as_ref().is_some_and(|s| {
                    s.workspaces.iter().any(|w| {
                        w.id == workspace_id && w.terminals.iter().any(|t| t.id == terminal_id)
                    })
                })
                && let Some(serial) = self.send(Command::SelectTerminal { terminal_id })
            {
                self.pending = Some(serial);
            }
        }
        // Reconcile on selection/attachment, not every picture: differently-sized
        // clients viewing one terminal must not fight over its size indefinitely.
        if selection_changed
            && self.ready()
            && self.screen().is_some_and(|s| (s.cols, s.rows) != self.grid)
        {
            self.resize();
        }
    }

    fn open_dialog(&mut self, dialog: Dialog, tx: &mut Tx) {
        if self.settings_open || !self.ready() {
            return;
        }
        let value = match &dialog {
            Dialog::RenameWorkspace(id) => self
                .snapshot
                .as_ref()
                .and_then(|s| s.workspaces.iter().find(|w| w.id == *id))
                .map(|w| w.name.as_str())
                .unwrap_or(""),
            // Blank means the service default; a workspace has no directory of its own.
            _ => "",
        };
        self.draft = TextEdit::new(value);
        self.draft.focused = !matches!(
            dialog,
            Dialog::CloseTerminal(..) | Dialog::CloseWorkspace(..)
        );
        self.dialog = Some(dialog);
        tx.blur();
    }

    fn submit_dialog(&mut self, tx: &mut Tx) {
        if !self.ready() {
            return;
        }
        let value = self.draft.text.trim().to_owned();
        let command = match self.dialog.clone() {
            Some(Dialog::NewWorkspace) => Command::CreateWorkspace { name: value },
            Some(Dialog::RenameWorkspace(workspace_id)) => Command::RenameWorkspace {
                workspace_id,
                name: value,
            },
            Some(Dialog::NewTerminal(workspace_id)) => Command::CreateTerminal {
                workspace_id,
                cwd: (!value.is_empty()).then_some(value),
            },
            Some(Dialog::CloseWorkspace(workspace_id, _)) => {
                Command::CloseWorkspace { workspace_id }
            }
            Some(Dialog::CloseTerminal(terminal_id, _)) => Command::CloseTerminal { terminal_id },
            None => return,
        };
        self.dialog = None;
        self.change(command);
        tx.blur();
    }
}

fn poll(state: &mut State, _: f64, tx: &mut Tx) -> bool {
    let Some(client) = &state.client else {
        return false;
    };
    let (snapshot, error, disconnected) = {
        let mut inbox = client.inbox.lock().unwrap();
        (
            inbox.view.take(),
            inbox.error.take(),
            std::mem::take(&mut inbox.disconnected),
        )
    };
    let changed = snapshot.is_some() || error.is_some() || disconnected;
    if let Some(snapshot) = snapshot {
        state.apply(snapshot);
    }
    if let Some(error) = error {
        state.notice = Some(error);
    }
    if disconnected {
        state.online = false;
        state.pending = None;
        state.preedit.clear();
        state.status = "Disconnected. Click Reconnect.".into();
    }
    if changed {
        tx.instant();
    }
    changed
}

fn app_shortcut(state: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    if state.recording.is_some() {
        return settings_ui::record_key(state, event, tx);
    }
    if rune::Chord::parse("mod+comma")
        .unwrap()
        .matches_tx(event, tx)
    {
        if !tx.key_repeat() {
            state.open_settings(tx);
        }
        return true;
    }
    // App-wide commands also work in Settings and text fields. Consuming Close while
    // a modal is open prevents AppKit's standard Cmd-W fallback from closing the window.
    for action in [
        Action::IncreaseFontSize,
        Action::DecreaseFontSize,
        Action::CloseTerminal,
    ] {
        if state.settings.matches(action, event, tx) {
            if !tx.key_repeat() || action != Action::CloseTerminal {
                state.act(action, tx);
            }
            return true;
        }
    }
    if state.settings_open || state.dialog.is_some() {
        return false;
    }
    if tx.focused().is_none_or(|key| key == "terminal") {
        for action in Action::ALL {
            if state.settings.matches(action, event, tx) {
                if !tx.key_repeat() {
                    state.act(action, tx);
                }
                return true;
            }
        }
    }
    false
}

fn terminal_key(state: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    if app_shortcut(state, event, tx) {
        return true;
    }
    if state.settings_open || state.dialog.is_some() {
        return false;
    }
    let application_cursor = state.screen().is_some_and(|s| s.application_cursor);
    if let Some(bytes) = input::encode(event, application_cursor) {
        state.input(bytes);
        true
    } else {
        false
    }
}

fn on_key(state: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    if tx.focused().is_none() {
        terminal_key(state, event, tx)
    } else {
        app_shortcut(state, event, tx)
    }
}

fn on_ime(state: &mut State, event: &Ime, tx: &mut Tx) -> bool {
    if state.settings_open
        || state.dialog.is_some()
        || tx.focused().is_some_and(|key| key != "terminal")
    {
        return false;
    }
    match event {
        Ime::Preedit(text, _) if state.ready() => state.preedit = text.clone(),
        Ime::Commit(text) => {
            state.preedit.clear();
            state.input(text.as_bytes().to_vec());
        }
        Ime::Disabled => state.preedit.clear(),
        _ => {}
    }
    true
}

fn main() {
    rune::prewarm_gpu();
    let mut args = std::env::args().skip(1);
    let mut rune_args = Vec::new();
    let mut initial = State::default();
    while let Some(arg) = args.next() {
        if arg == "--frame" {
            let result = args
                .next()
                .ok_or("--frame needs a JSON view event file".to_string())
                .and_then(|path| std::fs::read(path).map_err(|e| e.to_string()))
                .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string()));
            match result {
                Ok(Event::View { state }) => {
                    initial.offline = true;
                    initial.apply(state);
                }
                Ok(_) => {
                    eprintln!("griz: --frame requires a protocol v3 view event");
                    std::process::exit(1);
                }
                Err(error) => {
                    eprintln!("griz: {error}");
                    std::process::exit(1);
                }
            }
        } else {
            rune_args.push(arg);
        }
    }
    // Tests and offline captures use in-memory defaults and never touch user preferences.
    if !initial.offline {
        initial.prefs = rune::prefs::Prefs::open("griz");
        (initial.settings, initial.settings_error) = Settings::from_store(&initial.prefs);
    }
    let spec: AppSpec<State> = AppSpec {
        scenes: &[
            ("settings", |s| s.settings_open = true),
            ("shortcuts", |s| {
                s.settings_open = true;
                s.settings_page = settings_ui::Page::Shortcuts;
            }),
        ],
        setup: Some(|state, wake| state.wake = Some(wake)),
        poll: Some(poll),
        on_key: Some(on_key),
        ..AppSpec::new("Griz", (1100.0, 720.0), State::default, view)
    };
    let app = App::new(spec)
        .init(move || initial)
        .args(rune_args)
        .on_ime(on_ime)
        .menu_item(
            rune::MenuItem::new("Settings…", State::open_settings)
                .id("settings")
                .key_equivalent("mod+comma".parse().expect("settings shortcut"))
                .keys_first(),
        )
        .titlebar(Titlebar::new(TITLEBAR_HEIGHT));
    let handle = app.handle();
    app.on_ready(move |_| {
        handle.update(|state, tx| {
            state.connect();
            tx.instant();
        });
    })
    .run();
}

#[cfg(test)]
mod tests;
