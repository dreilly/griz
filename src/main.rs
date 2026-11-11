//! Native Rune client. Workspaces, PTYs and terminal emulators live in griz-core.
mod agents_ui;
mod client;
mod command_palette;
mod harness_icons;
mod input;
mod navigation_preview;
mod panes;
mod settings;
mod settings_ui;
mod terminal_mouse;
mod terminal_view;
mod ui;
mod workspace_ui;

use griz_protocol::{
    Command, Event, Screen, SplitDirection, TabInfo, TerminalId, ViewState, WorkspaceId,
    WorkspaceInfo,
};
use rune::{App, AppSpec, Ime, KeyEvent, TextEdit, Titlebar, Tx, Waker};
use settings::{Action, Settings};
use std::collections::HashMap;
use ui::view;

const TITLEBAR_HEIGHT: f32 = 44.0;

#[derive(Clone)]
enum Dialog {
    NewWorkspace,
    RenameWorkspace(WorkspaceId),
    WorkspaceMenu(WorkspaceId, (f32, f32)),
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
    palette: command_palette::PaletteState,
    completion_sweeps: agents_ui::CompletionSweeps,
    terminal_mouse: terminal_mouse::Interaction,
    pane_sizes: HashMap<TerminalId, (f32, f32)>,
    pane_grids: HashMap<TerminalId, (u16, u16)>,
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
            palette: command_palette::PaletteState::default(),
            completion_sweeps: Default::default(),
            terminal_mouse: Default::default(),
            pane_sizes: HashMap::new(),
            pane_grids: HashMap::new(),
        }
    }
}

impl State {
    fn open_settings(&mut self, tx: &mut Tx) {
        terminal_mouse::cancel(self);
        command_palette::close(self);
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
        let sizes = self.pane_sizes.clone();
        for (id, (width, height)) in sizes {
            self.layout_pane(id, width, height);
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
        if !font_action && (self.settings_open || self.dialog.is_some() || self.palette.is_open()) {
            return;
        }
        match action {
            Action::QuickOpen | Action::CommandPalette => {
                command_palette::open(self, action, tx);
                return;
            }
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
                    self.request_close_terminal(id, tx);
                }
            }
            Action::SplitHorizontal | Action::SplitVertical => {
                if self.can_split()
                    && let Some(terminal_id) = self.active_terminal()
                {
                    self.split_terminal(
                        terminal_id,
                        if action == Action::SplitHorizontal {
                            SplitDirection::Horizontal
                        } else {
                            SplitDirection::Vertical
                        },
                    );
                }
            }
            Action::NextPane | Action::PreviousPane => {
                if let Some(tab) = self.active_tab() {
                    let ids = tab.layout.terminal_ids();
                    let current = ids
                        .iter()
                        .position(|id| Some(*id) == self.active_terminal())
                        .unwrap_or(0);
                    let next = if action == Action::NextPane {
                        (current + 1) % ids.len()
                    } else {
                        (current + ids.len() - 1) % ids.len()
                    };
                    self.change(Command::SelectTerminal {
                        terminal_id: ids[next],
                    });
                }
            }
            Action::NextTab | Action::PreviousTab => {
                if let Some(workspace) = self.workspace() {
                    let len = workspace.tabs.len();
                    if len > 0 {
                        let current = workspace
                            .tabs
                            .iter()
                            .position(|t| {
                                self.active_terminal()
                                    .is_some_and(|id| t.layout.contains(id))
                            })
                            .unwrap_or(0);
                        let next = if action == Action::NextTab {
                            (current + 1) % len
                        } else {
                            (current + len - 1) % len
                        };
                        if let Some(terminal_id) =
                            workspace.tabs[next].layout.terminal_ids().first().copied()
                        {
                            self.change(Command::SelectTerminal { terminal_id });
                        }
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

    fn active_tab(&self) -> Option<&TabInfo> {
        let id = self.active_terminal()?;
        self.workspace()?
            .tabs
            .iter()
            .find(|t| t.layout.contains(id))
    }

    fn can_split(&self) -> bool {
        self.ready()
            && self
                .active_tab()
                .is_some_and(|t| t.layout.terminal_ids().len() < griz_protocol::MAX_PANES_PER_TAB)
    }

    fn split_terminal(&mut self, terminal_id: TerminalId, direction: SplitDirection) {
        if !self.can_split()
            || !self
                .active_tab()
                .is_some_and(|tab| tab.layout.contains(terminal_id))
        {
            return;
        }
        let split = Command::SplitTerminal {
            terminal_id,
            direction,
        };
        if self.active_terminal() == Some(terminal_id) {
            self.change(split);
            return;
        }

        // The core requires the split target to be selected. Queue both commands
        // on the same ordered connection, and gate input until the split's ack:
        // the intermediate selection snapshot must not re-enable typing.
        self.change(Command::SelectTerminal { terminal_id });
        if self.pending.is_some()
            && let Some(serial) = self.send(split)
        {
            self.pending = Some(serial);
        }
    }

    fn pane_frame(&self, id: TerminalId) -> Option<&griz_protocol::TerminalFrame> {
        let snapshot = self.snapshot.as_ref()?;
        snapshot
            .terminals
            .iter()
            .find(|t| t.id == id)
            .or_else(|| snapshot.terminal.as_ref().filter(|t| t.id == id))
    }

    fn layout_pane(&mut self, id: TerminalId, width: f32, height: f32) {
        // Layout callbacks are queued; a pane may have closed or its tab changed
        // before this callback runs. Do not resurrect it or resize a hidden shell.
        if !self.active_tab().is_some_and(|t| t.layout.contains(id)) {
            return;
        }
        self.pane_sizes.insert(id, (width, height));
        let grid = terminal_view::grid_for(width, height, &self.settings);
        if Some(id) == self.active_terminal() {
            self.grid = grid;
        }
        if self.pane_grids.insert(id, grid) != Some(grid) && self.ready() {
            self.send(Command::Resize {
                terminal_id: id,
                cols: grid.0,
                rows: grid.1,
            });
        }
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
            self.palette.reset_preview();
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

    /// Focusing a terminal acknowledges only the notifications actually observed
    /// in that pane. Keep generation/revision guards so a newer result survives.
    fn acknowledge_focused_agents(&mut self) {
        if !self.ready() || self.settings_open || self.dialog.is_some() || self.palette.is_open() {
            return;
        }
        let Some(id) = self.active_terminal() else {
            return;
        };
        let agents: Vec<_> = self
            .workspace()
            .into_iter()
            .flat_map(|w| &w.terminals)
            .filter(|t| t.id == id)
            .flat_map(|t| &t.agents)
            .filter(|a| a.unread)
            .map(|a| Command::AcknowledgeAgent {
                terminal_id: id,
                generation: a.generation,
                id: a.record.id.clone(),
                revision: a.record.revision,
            })
            .collect();
        for command in agents {
            self.send(command);
        }
    }

    fn change(&mut self, command: Command) {
        if !self.ready() {
            return;
        }
        if matches!(&command, Command::SelectTerminal { terminal_id } if Some(*terminal_id) == self.active_terminal())
        {
            self.acknowledge_focused_agents();
        }
        terminal_mouse::cancel(self);
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

    /// Return whether input changed view-visible state (normally only an error).
    /// Sending bytes alone needs no GUI rebuild; the echoed screen arrives via poll.
    fn input(&mut self, bytes: Vec<u8>) -> bool {
        // A click may race a previously queued screen. Never type into the old tab.
        if self.settings_open
            || self.dialog.is_some()
            || self.palette.is_open()
            || self.pending.is_some()
            || self.offline
        {
            return false;
        }
        if !self.online {
            self.notice = Some("Not connected. Click Reconnect.".into());
            return true;
        }
        if self.shell_exited() {
            self.notice = Some("Shell has exited. Click Start new shell above.".into());
            return true;
        }
        if bytes.len() > griz_protocol::MAX_INPUT {
            self.notice = Some("Input rejected: paste limit is 256 KiB".into());
            return true;
        }
        let had_selection = self.terminal_mouse.selection.take().is_some();
        if let Some(terminal_id) = self.active_terminal() {
            return self
                .send(Command::Input {
                    terminal_id,
                    data: bytes,
                })
                .is_none()
                || had_selection;
        }
        false
    }

    fn apply(&mut self, snapshot: ViewState) {
        if self.pending.is_some_and(|serial| snapshot.ack < serial) {
            return;
        }
        agents_ui::update_completions(self, &snapshot);
        let selection_changed = self.pending.is_some()
            || !self.online
            || self.active_terminal() != snapshot.active_terminal;
        self.pending = None;
        self.online = !self.offline;
        let old_terminal = self.active_terminal();
        let old_tab = self.active_tab().cloned();
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
        if let Some(Dialog::WorkspaceMenu(id, _)) = &self.dialog
            && !snapshot
                .workspaces
                .iter()
                .any(|workspace| workspace.id == *id)
        {
            self.dialog = None;
        }
        if self
            .snapshot
            .as_ref()
            .is_some_and(|old| old.core_pid != snapshot.core_pid)
            || self.terminal_mouse.selection.as_ref().is_some_and(|s| {
                self.pane_frame(s.id).is_some_and(|old| {
                    snapshot
                        .terminals
                        .iter()
                        .chain(snapshot.terminal.iter())
                        .any(|new| new.id == s.id && new.shell_pid != old.shell_pid)
                })
            })
        {
            terminal_mouse::discard(self);
        }
        self.snapshot = Some(snapshot);
        terminal_mouse::reconcile(self);
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
        // Reconcile once on attachment/selection/layout changes, not on every frame:
        // differently-sized windows must not fight over PTY sizes indefinitely.
        let layout_changed = old_tab.as_ref() != self.active_tab();
        if let Some(snapshot) = &self.snapshot {
            let visible: Vec<_> = snapshot.terminals.iter().map(|t| t.id).collect();
            self.pane_sizes.retain(|id, _| visible.contains(id));
            self.pane_grids.retain(|id, _| visible.contains(id));
        }
        if (selection_changed || layout_changed) && self.ready() {
            for (id, (width, height)) in self.pane_sizes.clone() {
                let grid = terminal_view::grid_for(width, height, &self.settings);
                self.pane_grids.insert(id, grid);
                if self
                    .pane_frame(id)
                    .is_some_and(|t| (t.screen.cols, t.screen.rows) != grid)
                {
                    self.send(Command::Resize {
                        terminal_id: id,
                        cols: grid.0,
                        rows: grid.1,
                    });
                }
            }
            if self
                .active_tab()
                .is_some_and(|t| t.layout.terminal_ids().len() == 1)
                && self
                    .active_terminal()
                    .is_some_and(|id| !self.pane_sizes.contains_key(&id))
                && self.screen().is_some_and(|s| (s.cols, s.rows) != self.grid)
            {
                self.resize();
            }
        }
        command_palette::sync_preview(self);
        if old_terminal != self.active_terminal() && self.pending.is_none() {
            self.acknowledge_focused_agents();
        }
    }

    fn request_close_terminal(&mut self, id: TerminalId, tx: &mut Tx) {
        if !self.ready() || self.settings_open || self.dialog.is_some() || self.palette.is_open() {
            return;
        }
        let Some(workspace) = self.snapshot.as_ref().and_then(|s| {
            s.workspaces
                .iter()
                .find(|w| w.terminals.iter().any(|t| t.id == id))
        }) else {
            return;
        };
        if workspace.terminals.len() == 1 {
            // Capture this terminal, not the current selection: confirming later must
            // never close a different shell after a catalog or selection update.
            self.open_dialog(Dialog::CloseTerminal(id, workspace.name.clone()), tx);
        } else {
            self.change(Command::CloseTerminal { terminal_id: id });
            tx.blur();
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
                .and_then(|w| w.custom_name.as_deref())
                .unwrap_or(""),
            // Other dialogs start blank; the core resolves their directory defaults.
            _ => "",
        };
        let value = value.to_owned();
        terminal_mouse::cancel(self);
        command_palette::close(self);
        self.draft = TextEdit::new(value);
        self.draft.focused = !matches!(
            dialog,
            Dialog::CloseTerminal(..) | Dialog::CloseWorkspace(..) | Dialog::WorkspaceMenu(..)
        );
        self.preedit.clear();
        self.dialog = Some(dialog);
        tx.blur();
    }

    fn submit_dialog(&mut self, tx: &mut Tx) {
        if !self.ready() {
            return;
        }
        let value = self.draft.text.trim().to_owned();
        let command = match self.dialog.clone() {
            Some(Dialog::NewWorkspace) => Command::CreateWorkspace {
                name: value,
                accent: self.settings.default_workspace_accent,
            },
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
            Some(Dialog::WorkspaceMenu(..)) | None => return,
        };
        self.dialog = None;
        self.change(command);
        tx.blur();
    }
}

fn poll(state: &mut State, _: f64, tx: &mut Tx) -> bool {
    let toast_changed = terminal_mouse::poll(state, tx);
    let Some(client) = &state.client else {
        return toast_changed;
    };
    let (snapshot, error, disconnected) = {
        let mut inbox = client.inbox.lock().unwrap();
        (
            inbox.view.take(),
            inbox.error.take(),
            std::mem::take(&mut inbox.disconnected),
        )
    };
    let changed = toast_changed || snapshot.is_some() || error.is_some() || disconnected;
    if let Some(snapshot) = snapshot {
        state.apply(snapshot);
    }
    if let Some(error) = error {
        state.notice = Some(error);
    }
    if disconnected {
        terminal_mouse::cancel(state);
        if matches!(state.dialog, Some(Dialog::WorkspaceMenu(..))) {
            state.dialog = None;
        }
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

fn terminal_focus(state: &State, tx: &Tx) -> bool {
    tx.focused().is_none_or(|key| {
        key == "terminal"
            || state
                .active_terminal()
                .is_some_and(|id| key == format!("terminal-viewport-{id}"))
    })
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
    if state.palette.is_open() && command_palette::key(state, event, tx) {
        return true;
    }
    for action in [Action::QuickOpen, Action::CommandPalette] {
        if state.settings.matches(action, event, tx) {
            if !tx.key_repeat() && !state.settings_open && state.dialog.is_none() {
                command_palette::open(state, action, tx);
            }
            return true;
        }
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
    if state.settings_open || state.dialog.is_some() || state.palette.is_open() {
        return false;
    }
    if terminal_focus(state, tx) {
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
    if state.settings_open || state.dialog.is_some() || state.palette.is_open() {
        return false;
    }
    if (rune::Chord::parse("mod+c").unwrap().matches_tx(event, tx)
        || rune::Chord::parse("ctrl+shift+c")
            .unwrap()
            .matches_tx(event, tx))
        && terminal_mouse::copy_selection(state, tx)
    {
        return true;
    }
    if event.key == rune::Key::Escape && state.terminal_mouse.selection.is_some() {
        terminal_mouse::cancel(state);
        tx.instant();
        return true;
    }
    let application_cursor = state.screen().is_some_and(|s| s.application_cursor);
    if let Some(bytes) = input::encode(event, application_cursor) {
        if !state.input(bytes) {
            tx.live_only();
        }
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
        || state.palette.is_open()
        || !terminal_focus(state, tx)
    {
        return false;
    }
    match event {
        Ime::Preedit(text, _) if state.ready() => state.preedit = text.clone(),
        Ime::Commit(text) => {
            let had_preedit = !state.preedit.is_empty();
            state.preedit.clear();
            if !state.input(text.as_bytes().to_vec()) && !had_preedit {
                tx.live_only();
            }
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
                    eprintln!("griz: --frame requires a protocol view event");
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
