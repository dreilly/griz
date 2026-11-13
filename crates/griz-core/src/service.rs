use crate::clients::Outbox;
use crate::session::Session;
use crate::validation::{directory_label, directory_path, validate_cwd, workspace_name};
use crate::{ClientId, Message};
use griz_protocol::{
    Command, Event, MAX_INPUT, MAX_PANES_PER_TAB, MAX_TERMINALS, MAX_WORKSPACES, PaneLayout,
    PanePlacement, PreviewFrame, PreviewTarget, Screen, SplitDirection, TabInfo, TerminalFrame,
    TerminalId, TerminalInfo, ViewState, WorkspaceAccent, WorkspaceId, WorkspaceInfo, dimensions,
};
use griz_vt::{PasteResult, Terminal, Viewport};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc::Receiver, mpsc::RecvTimeoutError, mpsc::SyncSender};
use std::time::{Duration, Instant};

const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const DIRECTORY_INTERVAL: Duration = Duration::from_millis(250);
const DEFAULT_SIZE: (u16, u16) = (80, 24);

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn output_frame_deadline(last: Option<Instant>, now: Instant) -> Instant {
    last.map_or(now, |last| (last + FRAME_INTERVAL).max(now))
}

fn paste_line_count(text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }
    let mut lines = 1u32;
    let mut bytes = text.as_bytes().iter().copied().peekable();
    while let Some(byte) = bytes.next() {
        match byte {
            b'\r' => {
                lines += 1;
                if bytes.peek() == Some(&b'\n') {
                    bytes.next();
                }
            }
            b'\n' => lines += 1,
            _ => {}
        }
    }
    lines
}

#[cfg(test)]
mod pacing_tests {
    use super::*;

    #[test]
    fn first_and_idle_output_do_not_wait_for_a_frame_interval() {
        let now = Instant::now();
        assert_eq!(output_frame_deadline(None, now), now);
        assert_eq!(
            output_frame_deadline(Some(now - FRAME_INTERVAL * 2), now),
            now
        );
    }

    #[test]
    fn sustained_output_keeps_the_existing_frame_boundary() {
        let last = Instant::now();
        for elapsed in [1, 4, 8, 15] {
            assert_eq!(
                output_frame_deadline(Some(last), last + Duration::from_millis(elapsed)),
                last + FRAME_INTERVAL,
            );
        }
    }
}

struct TerminalRecord {
    id: TerminalId,
    label: String,
    initial_cwd: PathBuf,
    current_cwd: PathBuf,
    generation: u64,
    grid: (u16, u16),
    vt: Terminal,
    session: Session,
    exited: Option<String>,
    agent_notifications: crate::agents::Notifications,
}

impl TerminalRecord {
    fn spawn(
        id: TerminalId,
        label: String,
        cwd: PathBuf,
        generation: u64,
        grid: (u16, u16),
        shell: &Path,
        tx: SyncSender<Message>,
    ) -> Result<Self> {
        let vt = Terminal::new(grid.0, grid.1)?;
        let session = Session::spawn(shell, &cwd, id, generation, grid.0, grid.1, tx)?;
        Ok(Self {
            id,
            label,
            current_cwd: cwd.clone(),
            initial_cwd: cwd,
            generation,
            grid,
            vt,
            session,
            exited: None,
            agent_notifications: crate::agents::Notifications::default(),
        })
    }

    fn info(&self) -> TerminalInfo {
        TerminalInfo {
            id: self.id,
            label: self.label.clone(),
            initial_cwd: self.initial_cwd.to_string_lossy().into_owned(),
            directory: directory_path(&self.current_cwd),
            shell_pid: self.session.pid,
            exited: self.exited.clone(),
            agents: self
                .agent_notifications
                .info(self.vt.program_status(), self.generation),
        }
    }
}

struct Workspace {
    id: WorkspaceId,
    custom_name: Option<String>,
    accent: WorkspaceAccent,
    terminals: Vec<TerminalRecord>,
    tabs: Vec<TabInfo>,
    next_terminal_number: usize,
}

struct PendingPaste {
    id: u64,
    terminal_id: TerminalId,
    generation: u64,
    text: String,
}

struct Client {
    active_workspace: WorkspaceId,
    selected: HashMap<WorkspaceId, Option<TerminalId>>,
    dimensions: Option<(u16, u16)>,
    preview: Option<PreviewTarget>,
    viewports: HashMap<TerminalId, Viewport>,
    pending_paste: Option<PendingPaste>,
    next_paste_id: u64,
    ack: u64,
    outbox: Arc<Outbox>,
}

#[derive(Clone, Copy)]
enum Outcome {
    Requester,
    Catalog,
    Input,
}

pub struct Service {
    shell: PathBuf,
    default_cwd: PathBuf,
    tx: SyncSender<Message>,
    workspaces: Vec<Workspace>,
    /// Only the first attachment may set the bootstrap workspace's accent.
    initial_workspace_unclaimed: bool,
    clients: BTreeMap<ClientId, Client>,
    next_workspace_id: WorkspaceId,
    next_terminal_id: TerminalId,
    dirty: HashSet<TerminalId>,
    catalog_dirty: bool,
    frame_deadline: Option<Instant>,
    last_output_frame: Option<Instant>,
    pending_input_acks: HashSet<ClientId>,
}

impl Service {
    pub fn new(shell: PathBuf, default_cwd: PathBuf, tx: SyncSender<Message>) -> Result<Self> {
        let terminal = TerminalRecord::spawn(
            1,
            "Terminal 1".into(),
            default_cwd.clone(),
            1,
            DEFAULT_SIZE,
            &shell,
            tx.clone(),
        )?;
        Ok(Self {
            shell,
            default_cwd,
            tx,
            workspaces: vec![Workspace {
                id: 1,
                custom_name: None,
                accent: WorkspaceAccent::default(),
                terminals: vec![terminal],
                tabs: vec![TabInfo {
                    id: 1,
                    layout: PaneLayout::Leaf { terminal_id: 1 },
                }],
                next_terminal_number: 2,
            }],
            initial_workspace_unclaimed: true,
            clients: BTreeMap::new(),
            next_workspace_id: 2,
            next_terminal_id: 2,
            dirty: HashSet::new(),
            catalog_dirty: false,
            frame_deadline: None,
            last_output_frame: None,
            pending_input_acks: HashSet::new(),
        })
    }

    pub fn run(&mut self, rx: Receiver<Message>) -> Result<()> {
        let mut directory_deadline = Instant::now();
        let result = (|| {
            loop {
                // A continuously full PTY queue must not starve the publication timer.
                if self
                    .frame_deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
                {
                    self.flush_dirty()?;
                }
                // Shells need not print a prompt (or use OSC 7) after changing directory.
                // Poll independently of PTY output, but sleep indefinitely without clients
                // or workspaces. Check here so continuous output cannot starve updates.
                let watch_directories = !self.clients.is_empty() && !self.workspaces.is_empty();
                if watch_directories && Instant::now() >= directory_deadline {
                    directory_deadline = Instant::now() + DIRECTORY_INTERVAL;
                    if self.refresh_directories() {
                        self.publish_all()?;
                    }
                }
                let deadline = self
                    .frame_deadline
                    .into_iter()
                    .chain(watch_directories.then_some(directory_deadline))
                    .min();
                let message = if let Some(deadline) = deadline {
                    match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                        Ok(message) => message,
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                } else {
                    match rx.recv() {
                        Ok(message) => message,
                        Err(_) => break,
                    }
                };

                match message {
                    Message::Attach {
                        client_id,
                        cols,
                        rows,
                        default_workspace_accent,
                        outbox,
                    } => self.attach(client_id, cols, rows, default_workspace_accent, outbox)?,
                    Message::Detach { client_id } => self.detach(client_id),
                    Message::Command {
                        client_id,
                        serial,
                        command,
                    } => self.process_command(client_id, serial, command)?,
                    Message::Output {
                        terminal_id,
                        generation,
                        bytes,
                    } => self.output(terminal_id, generation, bytes),
                    Message::Exited {
                        terminal_id,
                        generation,
                        status,
                    } => self.exited(terminal_id, generation, status)?,
                    Message::Shutdown => {
                        for client in self.clients.values() {
                            client.outbox.stopping();
                        }
                        break;
                    }
                }
            }
            Ok(())
        })();
        self.shutdown();
        result
    }

    fn attach(
        &mut self,
        client_id: ClientId,
        cols: u16,
        rows: u16,
        default_workspace_accent: WorkspaceAccent,
        outbox: Arc<Outbox>,
    ) -> Result<()> {
        // Initialize before publishing any view; never let a reconnect or another
        // window's preferences overwrite a workspace's shared accent.
        if self.initial_workspace_unclaimed {
            self.initial_workspace_unclaimed = false;
            if let Some(workspace) = self.workspaces.first_mut() {
                workspace.accent = default_workspace_accent;
            }
        }
        // Directory polling sleeps without windows; refresh before reconnect.
        self.refresh_directories();
        let active_workspace = self.workspaces.first().map_or(0, |workspace| workspace.id);
        let selected = self
            .workspaces
            .iter()
            .map(|workspace| {
                (
                    workspace.id,
                    workspace
                        .tabs
                        .first()
                        .and_then(|tab| tab.layout.terminal_ids().first().copied()),
                )
            })
            .collect();
        let client_dimensions = if cols == 0 || rows == 0 {
            None
        } else {
            Some(dimensions(cols, rows))
        };
        self.clients.insert(
            client_id,
            Client {
                active_workspace,
                selected,
                dimensions: client_dimensions,
                preview: None,
                viewports: HashMap::new(),
                pending_paste: None,
                next_paste_id: 1,
                ack: 0,
                outbox,
            },
        );
        if let (Some(size), Some(terminal_id)) =
            (client_dimensions, self.active_terminal(client_id))
            && !self.terminal_tab_is_split(terminal_id)
        {
            self.resize_terminal(terminal_id, size);
        }
        self.publish_views(&[client_id])
    }

    fn detach(&mut self, client_id: ClientId) {
        self.invalidate_pending_paste(client_id);
        self.clients.remove(&client_id);
        self.pending_input_acks.remove(&client_id);
        if self.clients.is_empty() {
            self.dirty.clear();
            self.frame_deadline = None;
        }
    }

    fn process_command(
        &mut self,
        client_id: ClientId,
        serial: u64,
        command: Command,
    ) -> Result<()> {
        let Some(client) = self.clients.get_mut(&client_id) else {
            return Ok(());
        };
        if serial <= client.ack {
            client.outbox.error(
                Some(serial),
                format!(
                    "command serial {serial} is not greater than acknowledged serial {}",
                    client.ack
                ),
            );
            return self.publish_views(&[client_id]);
        }
        client.ack = serial;
        let outbox = client.outbox.clone();
        if matches!(&command, Command::Paste { .. }) {
            // Every new paste replaces any older confirmation, even if the new request is bad.
            self.invalidate_pending_paste(client_id);
        }

        // Selection/name changes should use a fresh cwd immediately, not wait for
        // the next poll. Do not perform process queries on every keystroke or resize.
        let refresh = !matches!(
            &command,
            Command::Input { .. }
                | Command::Mouse { .. }
                | Command::Scroll { .. }
                | Command::Resize { .. }
                | Command::SetPreview { .. }
                | Command::Paste { .. }
                | Command::ConfirmPaste { .. }
                | Command::CancelPaste { .. }
        );
        let outcome = self.apply_command(client_id, serial, command);
        self.invalidate_stale_pastes();
        let directories_changed = refresh && self.refresh_directories();
        match outcome {
            Ok(Outcome::Input) => {
                // The PTY has not echoed the key yet. Sending a full old screen here
                // delays the useful picture and makes the GUI render twice per key.
                // Output normally supplies the acknowledgement; a bounded fallback
                // still acknowledges silent input (e.g. a password prompt).
                self.pending_input_acks.insert(client_id);
                self.frame_deadline
                    .get_or_insert_with(|| Instant::now() + FRAME_INTERVAL);
                Ok(())
            }
            Ok(Outcome::Requester) if !directories_changed => self.publish_views(&[client_id]),
            Ok(_) => {
                self.dirty.clear();
                self.frame_deadline = None;
                self.publish_all()
            }
            Err(message) => {
                outbox.error(Some(serial), message);
                if directories_changed {
                    self.publish_all()
                } else {
                    self.publish_views(&[client_id])
                }
            }
        }
    }

    fn apply_command(
        &mut self,
        client_id: ClientId,
        serial: u64,
        command: Command,
    ) -> std::result::Result<Outcome, String> {
        match command {
            Command::AcknowledgeAgent {
                terminal_id,
                generation,
                id,
                revision,
            } => {
                let (workspace, terminal) = self
                    .terminal_location(terminal_id)
                    .ok_or_else(|| format!("unknown terminal {terminal_id}"))?;
                let terminal = &mut self.workspaces[workspace].terminals[terminal];
                if terminal.generation == generation {
                    terminal.agent_notifications.acknowledge(
                        terminal.vt.program_status(),
                        &id,
                        revision,
                    );
                }
                Ok(Outcome::Catalog)
            }
            Command::SetPreview { target } => {
                // Invalidated targets simply produce no preview; catalog changes can race
                // navigation. Never change remembered selections or terminal dimensions.
                self.clients.get_mut(&client_id).unwrap().preview = target;
                Ok(Outcome::Requester)
            }
            Command::CreateWorkspace { name, accent } => {
                self.create_workspace(client_id, name, accent)
            }
            Command::RenameWorkspace { workspace_id, name } => {
                let index = self
                    .workspaces
                    .iter()
                    .position(|workspace| workspace.id == workspace_id)
                    .ok_or_else(|| format!("unknown workspace {workspace_id}"))?;
                self.workspaces[index].custom_name = workspace_name(&name)?;
                Ok(Outcome::Catalog)
            }
            Command::SetWorkspaceAccent {
                workspace_id,
                accent,
            } => {
                let workspace = self
                    .workspaces
                    .iter_mut()
                    .find(|workspace| workspace.id == workspace_id)
                    .ok_or_else(|| format!("unknown workspace {workspace_id}"))?;
                workspace.accent = accent;
                Ok(Outcome::Catalog)
            }
            Command::CloseWorkspace { workspace_id } => self.close_workspace(workspace_id),
            Command::SelectWorkspace { workspace_id } => {
                let workspace = self
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.id == workspace_id)
                    .ok_or_else(|| format!("unknown workspace {workspace_id}"))?;
                let fallback = workspace
                    .tabs
                    .first()
                    .and_then(|tab| tab.layout.terminal_ids().first().copied());
                let client = self.clients.get_mut(&client_id).unwrap();
                let selected = *client.selected.entry(workspace_id).or_insert(fallback);
                let size = client.dimensions;
                if let (Some(terminal_id), Some(size)) = (selected, size)
                    && !self.terminal_tab_is_split(terminal_id)
                {
                    self.resize_terminal(terminal_id, size);
                }
                self.clients.get_mut(&client_id).unwrap().active_workspace = workspace_id;
                Ok(Outcome::Requester)
            }
            Command::CreateTerminal { workspace_id, cwd } => {
                self.create_terminal(client_id, workspace_id, cwd)
            }
            Command::SelectTerminal { terminal_id } => {
                let (workspace_index, _) = self
                    .terminal_location(terminal_id)
                    .ok_or_else(|| format!("unknown terminal {terminal_id}"))?;
                let workspace_id = self.workspaces[workspace_index].id;
                if self.clients[&client_id].active_workspace != workspace_id {
                    return Err(format!(
                        "terminal {terminal_id} is not in the active workspace"
                    ));
                }
                if let Some(size) = self.clients[&client_id].dimensions
                    && !self.terminal_tab_is_split(terminal_id)
                {
                    self.resize_terminal(terminal_id, size);
                }
                self.clients
                    .get_mut(&client_id)
                    .unwrap()
                    .selected
                    .insert(workspace_id, Some(terminal_id));
                Ok(Outcome::Requester)
            }
            Command::SplitTerminal {
                terminal_id,
                direction,
            } => self.split_terminal(client_id, terminal_id, direction),
            Command::MovePane {
                terminal_id,
                target_terminal_id,
                placement,
            } => self.move_pane(client_id, terminal_id, target_terminal_id, placement),
            Command::CloseTerminal { terminal_id } => self.close_terminal(terminal_id),
            Command::Input { terminal_id, data } => {
                if data.len() > MAX_INPUT {
                    return Err(format!("terminal input exceeds {MAX_INPUT} bytes"));
                }
                let (workspace_index, terminal_index) = self
                    .terminal_location(terminal_id)
                    .ok_or_else(|| format!("unknown terminal {terminal_id}"))?;
                let terminal = &self.workspaces[workspace_index].terminals[terminal_index];
                if terminal.exited.is_some() {
                    return Err(format!("terminal {terminal_id} has exited"));
                }
                terminal
                    .session
                    .input()
                    .ok_or_else(|| format!("terminal {terminal_id} input is closed"))?
                    .try_send(data)
                    .map_err(|error| {
                        format!("terminal {terminal_id} input unavailable: {error}")
                    })?;
                self.clients
                    .get_mut(&client_id)
                    .unwrap()
                    .viewports
                    .remove(&terminal_id);
                Ok(Outcome::Input)
            }
            Command::Paste { terminal_id, text } => {
                self.begin_paste(client_id, serial, terminal_id, text)
            }
            Command::ConfirmPaste { paste_id } => self.confirm_paste(client_id, paste_id),
            Command::CancelPaste { paste_id } => self.cancel_paste(client_id, paste_id),
            Command::Mouse { terminal_id, event } => {
                let (workspace_index, terminal_index) = self
                    .terminal_location(terminal_id)
                    .ok_or_else(|| format!("unknown terminal {terminal_id}"))?;
                let terminal = &mut self.workspaces[workspace_index].terminals[terminal_index];
                if terminal.exited.is_some() {
                    return Err(format!("terminal {terminal_id} has exited"));
                }
                let input = terminal
                    .session
                    .input()
                    .cloned()
                    .ok_or_else(|| format!("terminal {terminal_id} input is closed"))?;
                let data = terminal
                    .vt
                    .mouse_input(&event)
                    .map_err(|error| format!("terminal {terminal_id} mouse input: {error}"))?;
                if !data.is_empty() {
                    input.try_send(data).map_err(|error| {
                        format!("terminal {terminal_id} input unavailable: {error}")
                    })?;
                }
                Ok(Outcome::Input)
            }
            Command::Scroll { terminal_id, rows } => {
                if !self.visible_terminal_ids(client_id).contains(&terminal_id) {
                    return Err(format!("terminal {terminal_id} is not visible"));
                }
                let (workspace, terminal) = self
                    .terminal_location(terminal_id)
                    .ok_or_else(|| format!("unknown terminal {terminal_id}"))?;
                let viewport = self
                    .clients
                    .get_mut(&client_id)
                    .unwrap()
                    .viewports
                    .entry(terminal_id)
                    .or_default();
                self.workspaces[workspace].terminals[terminal]
                    .vt
                    .scroll(viewport, rows)
                    .map_err(|error| format!("terminal {terminal_id} scroll: {error}"))?;
                if viewport.is_live() {
                    self.clients
                        .get_mut(&client_id)
                        .unwrap()
                        .viewports
                        .remove(&terminal_id);
                }
                Ok(Outcome::Requester)
            }
            Command::Resize {
                terminal_id,
                cols,
                rows,
            } => {
                if self.terminal_location(terminal_id).is_none() {
                    return Err(format!("unknown terminal {terminal_id}"));
                }
                let size = dimensions(cols, rows);
                self.resize_terminal(terminal_id, size);
                self.clients.get_mut(&client_id).unwrap().dimensions = Some(size);
                Ok(Outcome::Requester)
            }
            Command::StartShell { terminal_id } => self.start_shell(terminal_id),
        }
    }

    fn begin_paste(
        &mut self,
        client_id: ClientId,
        serial: u64,
        terminal_id: TerminalId,
        text: String,
    ) -> std::result::Result<Outcome, String> {
        if text.len() > MAX_INPUT {
            return Err(format!("terminal paste exceeds {MAX_INPUT} UTF-8 bytes"));
        }
        if self.active_terminal(client_id) != Some(terminal_id) {
            return Err(format!("terminal {terminal_id} is not selected"));
        }
        let (workspace_index, terminal_index) = self
            .terminal_location(terminal_id)
            .ok_or_else(|| format!("unknown terminal {terminal_id}"))?;
        let terminal = &mut self.workspaces[workspace_index].terminals[terminal_index];
        if terminal.exited.is_some() {
            return Err(format!("terminal {terminal_id} has exited"));
        }
        let input = terminal
            .session
            .input()
            .cloned()
            .ok_or_else(|| format!("terminal {terminal_id} input is closed"))?;
        let generation = terminal.generation;
        let shell_pid = terminal.session.pid;
        match terminal
            .vt
            .paste(&text, false)
            .map_err(|error| format!("terminal {terminal_id} paste: {error}"))?
        {
            PasteResult::Empty => Ok(Outcome::Requester),
            PasteResult::Written(data) => {
                input.try_send(data).map_err(|error| {
                    format!("terminal {terminal_id} input unavailable: {error}")
                })?;
                self.clients
                    .get_mut(&client_id)
                    .unwrap()
                    .viewports
                    .remove(&terminal_id);
                Ok(Outcome::Input)
            }
            PasteResult::Rejected => {
                let byte_count = u32::try_from(text.len()).unwrap();
                let line_count = paste_line_count(&text);
                let client = self.clients.get_mut(&client_id).unwrap();
                let paste_id = client.next_paste_id;
                client.next_paste_id = client
                    .next_paste_id
                    .checked_add(1)
                    .ok_or_else(|| "paste confirmation token space exhausted".to_string())?;
                client.pending_paste = Some(PendingPaste {
                    id: paste_id,
                    terminal_id,
                    generation,
                    text,
                });
                client
                    .outbox
                    .paste_confirmation(Event::PasteConfirmationRequired {
                        serial,
                        paste_id,
                        terminal_id,
                        shell_pid,
                        byte_count,
                        line_count,
                    });
                Ok(Outcome::Requester)
            }
        }
    }

    fn confirm_paste(
        &mut self,
        client_id: ClientId,
        paste_id: u64,
    ) -> std::result::Result<Outcome, String> {
        let pending = {
            let client = self.clients.get_mut(&client_id).unwrap();
            if client.pending_paste.as_ref().map(|paste| paste.id) != Some(paste_id) {
                return Err(format!("stale paste confirmation {paste_id}"));
            }
            let pending = client.pending_paste.take().unwrap();
            client.outbox.cancel_paste_confirmation(paste_id);
            pending
        };
        if self.active_terminal(client_id) != Some(pending.terminal_id) {
            return Err(format!(
                "terminal {} is no longer selected",
                pending.terminal_id
            ));
        }
        let (workspace_index, terminal_index) = self
            .terminal_location(pending.terminal_id)
            .ok_or_else(|| format!("unknown terminal {}", pending.terminal_id))?;
        let terminal = &mut self.workspaces[workspace_index].terminals[terminal_index];
        if terminal.generation != pending.generation {
            return Err(format!(
                "terminal {} shell was replaced",
                pending.terminal_id
            ));
        }
        if terminal.exited.is_some() {
            return Err(format!("terminal {} has exited", pending.terminal_id));
        }
        let input = terminal
            .session
            .input()
            .cloned()
            .ok_or_else(|| format!("terminal {} input is closed", pending.terminal_id))?;
        match terminal
            .vt
            .paste(&pending.text, true)
            .map_err(|error| format!("terminal {} paste: {error}", pending.terminal_id))?
        {
            PasteResult::Empty => Ok(Outcome::Requester),
            PasteResult::Written(data) => {
                input.try_send(data).map_err(|error| {
                    format!(
                        "terminal {} input unavailable: {error}",
                        pending.terminal_id
                    )
                })?;
                self.clients
                    .get_mut(&client_id)
                    .unwrap()
                    .viewports
                    .remove(&pending.terminal_id);
                Ok(Outcome::Input)
            }
            PasteResult::Rejected => Err(format!(
                "terminal {} rejected an approved paste",
                pending.terminal_id
            )),
        }
    }

    fn cancel_paste(
        &mut self,
        client_id: ClientId,
        paste_id: u64,
    ) -> std::result::Result<Outcome, String> {
        let client = self.clients.get_mut(&client_id).unwrap();
        if client.pending_paste.as_ref().map(|paste| paste.id) != Some(paste_id) {
            return Err(format!("stale paste cancellation {paste_id}"));
        }
        client.pending_paste = None;
        client.outbox.cancel_paste_confirmation(paste_id);
        Ok(Outcome::Requester)
    }

    fn invalidate_pending_paste(&mut self, client_id: ClientId) {
        let Some(client) = self.clients.get_mut(&client_id) else {
            return;
        };
        if let Some(paste) = client.pending_paste.take() {
            client.outbox.cancel_paste_confirmation(paste.id);
        }
    }

    fn invalidate_stale_pastes(&mut self) {
        let invalid = self
            .clients
            .iter()
            .filter_map(|(client_id, client)| {
                let paste = client.pending_paste.as_ref()?;
                if self.active_terminal(*client_id) != Some(paste.terminal_id) {
                    return Some(*client_id);
                }
                let Some((workspace, terminal)) = self.terminal_location(paste.terminal_id) else {
                    return Some(*client_id);
                };
                let terminal = &self.workspaces[workspace].terminals[terminal];
                (terminal.generation != paste.generation
                    || terminal.exited.is_some()
                    || terminal.session.input().is_none())
                .then_some(*client_id)
            })
            .collect::<Vec<_>>();
        for client_id in invalid {
            self.invalidate_pending_paste(client_id);
        }
    }

    fn create_workspace(
        &mut self,
        client_id: ClientId,
        name: String,
        accent: WorkspaceAccent,
    ) -> std::result::Result<Outcome, String> {
        if self.workspaces.len() >= MAX_WORKSPACES {
            return Err(format!("workspace limit ({MAX_WORKSPACES}) reached"));
        }
        if self.terminal_count() >= MAX_TERMINALS {
            return Err(format!("terminal limit ({MAX_TERMINALS}) reached"));
        }
        let cwd = self.workspace_cwd(client_id, self.clients[&client_id].active_workspace);
        let custom_name = workspace_name(&name)?;
        let workspace_id = self.take_workspace_id();
        let terminal_id = self.take_terminal_id();
        let grid = self.clients[&client_id].dimensions.unwrap_or(DEFAULT_SIZE);
        let terminal = TerminalRecord::spawn(
            terminal_id,
            "Terminal 1".into(),
            cwd,
            1,
            grid,
            &self.shell,
            self.tx.clone(),
        )
        .map_err(|error| format!("cannot create workspace: {error}"))?;
        self.workspaces.push(Workspace {
            id: workspace_id,
            custom_name,
            accent,
            terminals: vec![terminal],
            tabs: vec![TabInfo {
                id: terminal_id,
                layout: PaneLayout::Leaf { terminal_id },
            }],
            next_terminal_number: 2,
        });
        for client in self.clients.values_mut() {
            client.selected.insert(workspace_id, Some(terminal_id));
            if client.active_workspace == 0 {
                client.active_workspace = workspace_id;
            }
        }
        self.clients.get_mut(&client_id).unwrap().active_workspace = workspace_id;
        Ok(Outcome::Catalog)
    }

    fn create_terminal(
        &mut self,
        client_id: ClientId,
        workspace_id: WorkspaceId,
        cwd: Option<String>,
    ) -> std::result::Result<Outcome, String> {
        if self.terminal_count() >= MAX_TERMINALS {
            return Err(format!("terminal limit ({MAX_TERMINALS}) reached"));
        }
        let workspace_index = self
            .workspaces
            .iter()
            .position(|workspace| workspace.id == workspace_id)
            .ok_or_else(|| format!("unknown workspace {workspace_id}"))?;
        let cwd = validate_cwd(cwd.as_deref(), &self.default_cwd)?;
        let terminal_id = self.take_terminal_id();
        let number = self.workspaces[workspace_index].next_terminal_number;
        let grid = self.clients[&client_id].dimensions.unwrap_or(DEFAULT_SIZE);
        let terminal = TerminalRecord::spawn(
            terminal_id,
            format!("Terminal {number}"),
            cwd,
            1,
            grid,
            &self.shell,
            self.tx.clone(),
        )
        .map_err(|error| format!("cannot create terminal: {error}"))?;
        let workspace = &mut self.workspaces[workspace_index];
        workspace.terminals.push(terminal);
        workspace.tabs.push(TabInfo {
            id: terminal_id,
            layout: PaneLayout::Leaf { terminal_id },
        });
        workspace.next_terminal_number += 1;
        for client in self.clients.values_mut() {
            if client.selected.get(&workspace_id) == Some(&None) {
                client.selected.insert(workspace_id, Some(terminal_id));
            }
        }
        let client = self.clients.get_mut(&client_id).unwrap();
        client.active_workspace = workspace_id;
        client.selected.insert(workspace_id, Some(terminal_id));
        Ok(Outcome::Catalog)
    }

    fn split_terminal(
        &mut self,
        client_id: ClientId,
        terminal_id: TerminalId,
        direction: SplitDirection,
    ) -> std::result::Result<Outcome, String> {
        if self.terminal_count() >= MAX_TERMINALS {
            return Err(format!("terminal limit ({MAX_TERMINALS}) reached"));
        }
        let (workspace_index, terminal_index) = self
            .terminal_location(terminal_id)
            .ok_or_else(|| format!("unknown terminal {terminal_id}"))?;
        let workspace_id = self.workspaces[workspace_index].id;
        if self.clients[&client_id].active_workspace != workspace_id {
            return Err(format!(
                "terminal {terminal_id} is not in the active workspace"
            ));
        }
        if self.clients[&client_id]
            .selected
            .get(&workspace_id)
            .copied()
            .flatten()
            != Some(terminal_id)
        {
            return Err(format!("terminal {terminal_id} is not selected"));
        }
        let tab_index = self.workspaces[workspace_index]
            .tabs
            .iter()
            .position(|tab| tab.layout.contains(terminal_id))
            .ok_or_else(|| format!("terminal {terminal_id} is not in a tab"))?;
        if self.workspaces[workspace_index].tabs[tab_index]
            .layout
            .terminal_ids()
            .len()
            >= MAX_PANES_PER_TAB
        {
            return Err(format!(
                "pane limit ({MAX_PANES_PER_TAB}) reached for this tab"
            ));
        }

        let cwd = {
            let terminal = &self.workspaces[workspace_index].terminals[terminal_index];
            terminal
                .session
                .cwd()
                .filter(|path| path.is_dir())
                .or_else(|| {
                    terminal
                        .current_cwd
                        .is_dir()
                        .then(|| terminal.current_cwd.clone())
                })
                .unwrap_or_else(|| self.default_cwd.clone())
        };
        let new_terminal_id = self.take_terminal_id();
        let number = self.workspaces[workspace_index].next_terminal_number;
        let terminal = TerminalRecord::spawn(
            new_terminal_id,
            format!("Terminal {number}"),
            cwd,
            1,
            DEFAULT_SIZE,
            &self.shell,
            self.tx.clone(),
        )
        .map_err(|error| format!("cannot split terminal: {error}"))?;
        let workspace = &mut self.workspaces[workspace_index];
        if !split_layout(
            &mut workspace.tabs[tab_index].layout,
            terminal_id,
            new_terminal_id,
            direction,
        ) {
            return Err(format!("terminal {terminal_id} is not in a tab"));
        }
        workspace.terminals.push(terminal);
        workspace.next_terminal_number += 1;
        self.clients
            .get_mut(&client_id)
            .unwrap()
            .selected
            .insert(workspace_id, Some(new_terminal_id));
        Ok(Outcome::Catalog)
    }

    fn move_pane(
        &mut self,
        client_id: ClientId,
        terminal_id: TerminalId,
        target_terminal_id: TerminalId,
        placement: PanePlacement,
    ) -> std::result::Result<Outcome, String> {
        let (workspace_index, _) = self
            .terminal_location(terminal_id)
            .ok_or_else(|| format!("unknown terminal {terminal_id}"))?;
        let (target_workspace_index, _) = self
            .terminal_location(target_terminal_id)
            .ok_or_else(|| format!("unknown terminal {target_terminal_id}"))?;
        if terminal_id == target_terminal_id {
            return Err(format!(
                "cannot move terminal {terminal_id} relative to itself"
            ));
        }

        let workspace_id = self.workspaces[workspace_index].id;
        if self.clients[&client_id].active_workspace != workspace_id {
            return Err(format!(
                "terminal {terminal_id} is not in the active workspace"
            ));
        }
        if workspace_index != target_workspace_index {
            return Err(format!(
                "terminals {terminal_id} and {target_terminal_id} are not in the same tab"
            ));
        }

        let source_tab_index = self.workspaces[workspace_index]
            .tabs
            .iter()
            .position(|tab| tab.layout.contains(terminal_id))
            .ok_or_else(|| format!("terminal {terminal_id} is not in a tab"))?;
        let target_tab_index = self.workspaces[workspace_index]
            .tabs
            .iter()
            .position(|tab| tab.layout.contains(target_terminal_id))
            .ok_or_else(|| format!("terminal {target_terminal_id} is not in a tab"))?;
        if source_tab_index != target_tab_index {
            return Err(format!(
                "terminals {terminal_id} and {target_terminal_id} are not in the same tab"
            ));
        }

        let selected = self.clients[&client_id]
            .selected
            .get(&workspace_id)
            .copied()
            .flatten()
            .ok_or_else(|| format!("workspace {workspace_id} has no selected terminal"))?;
        if !self.workspaces[workspace_index].tabs[source_tab_index]
            .layout
            .contains(selected)
        {
            return Err(format!("terminal {terminal_id} is not in the selected tab"));
        }

        let original = self.workspaces[workspace_index].tabs[source_tab_index]
            .layout
            .clone();
        let layout = if placement == PanePlacement::Swap {
            let mut layout = original;
            if !swap_layout_terminals(&mut layout, terminal_id, target_terminal_id) {
                return Err("cannot swap pane layout".into());
            }
            layout
        } else {
            let (layout, _) = remove_from_layout(original, terminal_id);
            let mut layout = layout.ok_or_else(|| {
                format!("cannot move the only pane in tab for terminal {terminal_id}")
            })?;
            if !insert_around_terminal(&mut layout, target_terminal_id, terminal_id, placement) {
                return Err(format!("terminal {target_terminal_id} is not in a tab"));
            }
            layout
        };

        self.workspaces[workspace_index].tabs[source_tab_index].layout = layout;
        self.clients
            .get_mut(&client_id)
            .unwrap()
            .selected
            .insert(workspace_id, Some(terminal_id));
        Ok(Outcome::Catalog)
    }

    fn close_workspace(
        &mut self,
        workspace_id: WorkspaceId,
    ) -> std::result::Result<Outcome, String> {
        let index = self
            .workspaces
            .iter()
            .position(|workspace| workspace.id == workspace_id)
            .ok_or_else(|| format!("unknown workspace {workspace_id}"))?;
        let mut workspace = self.workspaces.remove(index);
        for terminal in &mut workspace.terminals {
            terminal.session.terminate();
            self.dirty.remove(&terminal.id);
        }
        // Prefer the next card, or the previous one when closing the last card.
        let fallback = self
            .workspaces
            .get(index)
            .or_else(|| self.workspaces.last())
            .map_or(0, |workspace| workspace.id);
        let mut fallback_sizes = Vec::new();
        for client in self.clients.values_mut() {
            client.selected.remove(&workspace_id);
            for terminal in &workspace.terminals {
                client.viewports.remove(&terminal.id);
            }
            if client.active_workspace == workspace_id {
                client.active_workspace = fallback;
                if let (Some(terminal_id), Some(size)) = (
                    client.selected.get(&fallback).copied().flatten(),
                    client.dimensions,
                ) {
                    fallback_sizes.push((terminal_id, size));
                }
            }
        }
        for (terminal_id, size) in fallback_sizes {
            if !self.terminal_tab_is_split(terminal_id) {
                self.resize_terminal(terminal_id, size);
            }
        }
        Ok(Outcome::Catalog)
    }

    fn close_terminal(&mut self, terminal_id: TerminalId) -> std::result::Result<Outcome, String> {
        let (workspace_index, terminal_index) = self
            .terminal_location(terminal_id)
            .ok_or_else(|| format!("unknown terminal {terminal_id}"))?;
        let workspace_id = self.workspaces[workspace_index].id;
        if self.workspaces[workspace_index].terminals.len() == 1 {
            return self.close_workspace(workspace_id);
        }
        let tab_index = self.workspaces[workspace_index]
            .tabs
            .iter()
            .position(|tab| tab.layout.contains(terminal_id))
            .ok_or_else(|| format!("terminal {terminal_id} is not in a tab"))?;
        let (layout, sibling_fallback) = remove_from_layout(
            self.workspaces[workspace_index].tabs[tab_index]
                .layout
                .clone(),
            terminal_id,
        );
        let fallback = if let Some(layout) = layout {
            self.workspaces[workspace_index].tabs[tab_index].layout = layout;
            sibling_fallback
        } else {
            self.workspaces[workspace_index].tabs.remove(tab_index);
            self.workspaces[workspace_index]
                .tabs
                .get(tab_index)
                .or_else(|| self.workspaces[workspace_index].tabs.last())
                .and_then(|tab| tab.layout.terminal_ids().first().copied())
        };

        let mut terminal = self.workspaces[workspace_index]
            .terminals
            .remove(terminal_index);
        terminal.session.terminate();
        let mut fallback_sizes = Vec::new();
        for client in self.clients.values_mut() {
            client.viewports.remove(&terminal_id);
            if client.selected.get(&workspace_id) == Some(&Some(terminal_id)) {
                client.selected.insert(workspace_id, fallback);
                if client.active_workspace == workspace_id
                    && let Some(size) = client.dimensions
                {
                    fallback_sizes.push(size);
                }
            }
        }
        if let Some(fallback_id) = fallback
            && !self.terminal_tab_is_split(fallback_id)
        {
            for size in fallback_sizes {
                self.resize_terminal(fallback_id, size);
            }
        }
        self.dirty.remove(&terminal_id);
        Ok(Outcome::Catalog)
    }

    fn start_shell(&mut self, terminal_id: TerminalId) -> std::result::Result<Outcome, String> {
        let (workspace_index, terminal_index) = self
            .terminal_location(terminal_id)
            .ok_or_else(|| format!("unknown terminal {terminal_id}"))?;
        let terminal = &self.workspaces[workspace_index].terminals[terminal_index];
        if terminal.exited.is_none() {
            return Err("shell is still running; refusing to replace it".into());
        }
        let generation = terminal.generation + 1;
        let grid = terminal.grid;
        let cwd = terminal.initial_cwd.clone();
        let label = terminal.label.clone();
        let mut replacement = TerminalRecord::spawn(
            terminal_id,
            label,
            cwd,
            generation,
            grid,
            &self.shell,
            self.tx.clone(),
        )
        .map_err(|error| format!("cannot start shell: {error}"))?;
        std::mem::swap(
            &mut replacement,
            &mut self.workspaces[workspace_index].terminals[terminal_index],
        );
        replacement.session.terminate();
        for client in self.clients.values_mut() {
            client.viewports.remove(&terminal_id);
        }
        Ok(Outcome::Catalog)
    }

    fn output(&mut self, terminal_id: TerminalId, generation: u64, bytes: Vec<u8>) {
        let Some((workspace_index, terminal_index)) = self.terminal_location(terminal_id) else {
            return;
        };
        let terminal = &mut self.workspaces[workspace_index].terminals[terminal_index];
        if terminal.generation != generation {
            return;
        }
        terminal.vt.write(&bytes);
        // The reader can deliver final bytes after the child's wait notification.
        // Preserve final done/error reports, but never resurrect transient ones.
        if terminal.exited.is_some() {
            terminal.vt.program_exited();
        }
        if terminal.vt.take_program_status_changed() {
            terminal
                .agent_notifications
                .reconcile(terminal.vt.program_status());
            self.catalog_dirty |= !self.clients.is_empty();
        }
        match terminal.vt.take_responses() {
            Ok(responses) if !responses.is_empty() => {
                if let Some(input) = terminal.session.input()
                    && let Err(error) = input.try_send(responses)
                {
                    eprintln!("griz-core: terminal {terminal_id} response: {error}");
                }
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("griz-core: terminal {terminal_id} response capture: {error}");
            }
        }
        self.queue_frame(terminal_id);
    }

    fn exited(&mut self, terminal_id: TerminalId, generation: u64, status: String) -> Result<()> {
        let Some((workspace_index, terminal_index)) = self.terminal_location(terminal_id) else {
            return Ok(());
        };
        let terminal = &mut self.workspaces[workspace_index].terminals[terminal_index];
        if terminal.generation != generation {
            return Ok(());
        }
        terminal.exited = Some(status);
        terminal.vt.program_exited();
        terminal.vt.take_program_status_changed();
        terminal
            .agent_notifications
            .reconcile(terminal.vt.program_status());
        self.invalidate_stale_pastes();
        self.dirty.clear();
        self.frame_deadline = None;
        self.publish_all()
    }

    fn resize_terminal(&mut self, terminal_id: TerminalId, size: (u16, u16)) {
        let Some((workspace_index, terminal_index)) = self.terminal_location(terminal_id) else {
            return;
        };
        let terminal = &mut self.workspaces[workspace_index].terminals[terminal_index];
        if terminal.grid == size {
            return;
        }
        if let Err(error) = terminal.vt.resize(size.0, size.1) {
            eprintln!("griz-core: terminal {terminal_id} render resize: {error}");
            return;
        }
        if terminal.exited.is_none()
            && let Err(error) = terminal.session.resize(size.0, size.1)
        {
            // A shell may exit between the command and its wait notification.
            eprintln!("griz-core: terminal {terminal_id} PTY resize: {error}");
        }
        terminal.grid = size;
        self.queue_frame(terminal_id);
    }

    fn queue_frame(&mut self, terminal_id: TerminalId) {
        if self.clients.is_empty()
            || (!self.catalog_dirty && !self.terminal_is_observed(terminal_id))
        {
            return;
        }
        self.dirty.insert(terminal_id);
        // Flush the first output after idle immediately. Only sustained output
        // waits for the next frame boundary; do not add a fresh 16 ms to every key.
        let now = Instant::now();
        let deadline = output_frame_deadline(self.last_output_frame, now);
        self.frame_deadline = Some(
            self.frame_deadline
                .map_or(deadline, |old| old.min(deadline)),
        );
    }

    fn flush_dirty(&mut self) -> Result<()> {
        let clients = self
            .clients
            .keys()
            .copied()
            .filter(|client_id| {
                self.catalog_dirty
                    || self.pending_input_acks.contains(client_id)
                    || self
                        .observed_terminal_ids(*client_id)
                        .iter()
                        .any(|terminal_id| self.dirty.contains(terminal_id))
            })
            .collect::<Vec<_>>();
        if !self.dirty.is_empty() {
            self.last_output_frame = Some(Instant::now());
        }
        self.dirty.clear();
        self.catalog_dirty = false;
        self.frame_deadline = None;
        self.publish_views(&clients)
    }

    fn publish_all(&mut self) -> Result<()> {
        self.catalog_dirty = false;
        let clients = self.clients.keys().copied().collect::<Vec<_>>();
        self.publish_views(&clients)
    }

    fn publish_views(&mut self, client_ids: &[ClientId]) -> Result<()> {
        let specs = client_ids
            .iter()
            .filter_map(|client_id| {
                self.clients.get(client_id).map(|client| {
                    let active_terminal = client
                        .selected
                        .get(&client.active_workspace)
                        .copied()
                        .flatten();
                    (
                        *client_id,
                        client.active_workspace,
                        active_terminal,
                        self.observed_terminal_ids(*client_id),
                        client.ack,
                        client.outbox.clone(),
                    )
                })
            })
            .collect::<Vec<_>>();
        let terminal_ids = specs
            .iter()
            .flat_map(|(_, _, _, terminal_ids, _, _)| terminal_ids.iter().copied())
            .collect::<HashSet<_>>();
        let mut screens: HashMap<TerminalId, Screen> = HashMap::new();
        for terminal_id in terminal_ids {
            if let Some((workspace_index, terminal_index)) = self.terminal_location(terminal_id) {
                let screen = self.workspaces[workspace_index].terminals[terminal_index]
                    .vt
                    .screen()?;
                screens.insert(terminal_id, screen);
            }
        }
        for (client_id, active_workspace, active_terminal, terminal_ids, ack, outbox) in specs {
            self.pending_input_acks.remove(&client_id);
            let frames = terminal_ids
                .into_iter()
                .filter_map(|terminal_id| {
                    let (workspace_index, terminal_index) = self.terminal_location(terminal_id)?;
                    let record = &self.workspaces[workspace_index].terminals[terminal_index];
                    Some(TerminalFrame {
                        id: terminal_id,
                        shell_pid: record.session.pid,
                        screen: screens.get(&terminal_id)?.clone(),
                        exited: record.exited.clone(),
                    })
                })
                .collect::<Vec<_>>();
            let preview = self
                .preview_tab(client_id)
                .map(|(target, selected, tab)| PreviewFrame {
                    target,
                    selected,
                    layout: tab.layout.clone(),
                    terminals: frames
                        .iter()
                        .filter(|frame| tab.layout.contains(frame.id))
                        .cloned()
                        .collect(),
                })
                .map(Box::new);
            let visible = self.visible_terminal_ids(client_id);
            let mut terminals: Vec<_> = frames
                .into_iter()
                .filter(|frame| visible.contains(&frame.id))
                .collect();
            // Previews stay live; only real panes use this client's history.
            for frame in &mut terminals {
                let (workspace, terminal) = self.terminal_location(frame.id).unwrap();
                if let Some(viewport) = self.clients[&client_id].viewports.get(&frame.id) {
                    frame.screen = self.workspaces[workspace].terminals[terminal]
                        .vt
                        .screen_at(viewport)?;
                }
            }
            let terminal = active_terminal
                .and_then(|terminal_id| terminals.iter().find(|frame| frame.id == terminal_id))
                .cloned();
            outbox.view(Event::View {
                state: ViewState {
                    core_pid: std::process::id(),
                    default_cwd: self.default_cwd.to_string_lossy().into_owned(),
                    ack,
                    workspaces: self.catalog(client_id),
                    active_workspace,
                    active_terminal,
                    terminals,
                    terminal,
                    preview,
                },
            });
        }
        Ok(())
    }

    fn catalog(&self, client_id: ClientId) -> Vec<WorkspaceInfo> {
        self.workspaces
            .iter()
            .map(|workspace| {
                // Selection is per client, including remembered tabs in inactive workspaces.
                let terminal = self.clients[&client_id]
                    .selected
                    .get(&workspace.id)
                    .copied()
                    .flatten()
                    .and_then(|id| {
                        workspace
                            .terminals
                            .iter()
                            .find(|terminal| terminal.id == id)
                    });
                let cwd = terminal.map_or(self.default_cwd.as_path(), |terminal| {
                    terminal.current_cwd.as_path()
                });
                WorkspaceInfo {
                    id: workspace.id,
                    name: workspace
                        .custom_name
                        .clone()
                        .unwrap_or_else(|| directory_label(cwd)),
                    custom_name: workspace.custom_name.clone(),
                    directory: directory_path(cwd),
                    accent: workspace.accent,
                    terminals: workspace
                        .terminals
                        .iter()
                        .map(TerminalRecord::info)
                        .collect(),
                    tabs: workspace.tabs.clone(),
                }
            })
            .collect()
    }

    fn refresh_directories(&mut self) -> bool {
        let mut changed = false;
        for terminal in self
            .workspaces
            .iter_mut()
            .flat_map(|workspace| &mut workspace.terminals)
        {
            if terminal.exited.is_none()
                && let Some(cwd) = terminal.session.cwd().filter(|path| path.is_absolute())
                && cwd != terminal.current_cwd
            {
                terminal.current_cwd = cwd;
                changed = true;
            }
        }
        // Keep the last known path when a process exits or its cwd cannot be read.
        changed
    }

    fn workspace_cwd(&self, client_id: ClientId, workspace_id: WorkspaceId) -> PathBuf {
        let selected = self.clients[&client_id]
            .selected
            .get(&workspace_id)
            .copied()
            .flatten();
        selected
            .and_then(|id| self.terminal_location(id))
            .and_then(|(workspace, terminal)| {
                let terminal = &self.workspaces[workspace].terminals[terminal];
                terminal
                    .session
                    .cwd()
                    .filter(|path| path.is_dir())
                    .or_else(|| {
                        terminal
                            .current_cwd
                            .is_dir()
                            .then(|| terminal.current_cwd.clone())
                    })
            })
            .unwrap_or_else(|| self.default_cwd.clone())
    }

    fn active_terminal(&self, client_id: ClientId) -> Option<TerminalId> {
        let client = self.clients.get(&client_id)?;
        client
            .selected
            .get(&client.active_workspace)
            .copied()
            .flatten()
    }

    fn visible_terminal_ids(&self, client_id: ClientId) -> Vec<TerminalId> {
        let Some(client) = self.clients.get(&client_id) else {
            return Vec::new();
        };
        let Some(selected) = client
            .selected
            .get(&client.active_workspace)
            .copied()
            .flatten()
        else {
            return Vec::new();
        };
        self.workspaces
            .iter()
            .find(|workspace| workspace.id == client.active_workspace)
            .and_then(|workspace| {
                workspace
                    .tabs
                    .iter()
                    .find(|tab| tab.layout.contains(selected))
            })
            .map_or_else(Vec::new, |tab| tab.layout.terminal_ids())
    }

    fn preview_tab(&self, client_id: ClientId) -> Option<(PreviewTarget, TerminalId, &TabInfo)> {
        let client = self.clients.get(&client_id)?;
        let target = client.preview?;
        let (workspace_id, requested) = match target {
            PreviewTarget::Workspace { workspace_id } => (workspace_id, None),
            PreviewTarget::Terminal {
                workspace_id,
                terminal_id,
            } => (workspace_id, Some(terminal_id)),
        };
        let workspace = self.workspaces.iter().find(|w| w.id == workspace_id)?;
        let selected = requested
            .or_else(|| client.selected.get(&workspace_id).copied().flatten())
            .or_else(|| {
                workspace
                    .tabs
                    .first()?
                    .layout
                    .terminal_ids()
                    .first()
                    .copied()
            })?;
        let tab = workspace
            .tabs
            .iter()
            .find(|tab| tab.layout.contains(selected))?;
        Some((target, selected, tab))
    }

    fn observed_terminal_ids(&self, client_id: ClientId) -> Vec<TerminalId> {
        let mut ids = self.visible_terminal_ids(client_id);
        if let Some((_, _, tab)) = self.preview_tab(client_id) {
            for id in tab.layout.terminal_ids() {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        ids
    }

    fn terminal_tab_is_split(&self, terminal_id: TerminalId) -> bool {
        self.workspaces.iter().any(|workspace| {
            workspace
                .tabs
                .iter()
                .any(|tab| tab.layout.contains(terminal_id) && tab.layout.terminal_ids().len() > 1)
        })
    }

    fn terminal_is_observed(&self, terminal_id: TerminalId) -> bool {
        self.clients.keys().any(|client_id| {
            self.observed_terminal_ids(*client_id)
                .contains(&terminal_id)
        })
    }

    fn terminal_location(&self, terminal_id: TerminalId) -> Option<(usize, usize)> {
        self.workspaces
            .iter()
            .enumerate()
            .find_map(|(workspace_index, workspace)| {
                workspace
                    .terminals
                    .iter()
                    .position(|terminal| terminal.id == terminal_id)
                    .map(|terminal_index| (workspace_index, terminal_index))
            })
    }

    fn terminal_count(&self) -> usize {
        self.workspaces
            .iter()
            .map(|workspace| workspace.terminals.len())
            .sum()
    }

    fn take_workspace_id(&mut self) -> WorkspaceId {
        let id = self.next_workspace_id;
        self.next_workspace_id += 1;
        id
    }

    fn take_terminal_id(&mut self) -> TerminalId {
        let id = self.next_terminal_id;
        self.next_terminal_id += 1;
        id
    }

    fn shutdown(&mut self) {
        for workspace in &mut self.workspaces {
            for terminal in &mut workspace.terminals {
                terminal.session.terminate();
            }
        }
    }
}

fn split_layout(
    layout: &mut PaneLayout,
    terminal_id: TerminalId,
    new_terminal_id: TerminalId,
    direction: SplitDirection,
) -> bool {
    match layout {
        PaneLayout::Leaf { terminal_id: id } if *id == terminal_id => {
            *layout = PaneLayout::Split {
                direction,
                first: Box::new(PaneLayout::Leaf { terminal_id }),
                second: Box::new(PaneLayout::Leaf {
                    terminal_id: new_terminal_id,
                }),
            };
            true
        }
        PaneLayout::Leaf { .. } => false,
        PaneLayout::Split { first, second, .. } => {
            split_layout(first, terminal_id, new_terminal_id, direction)
                || split_layout(second, terminal_id, new_terminal_id, direction)
        }
    }
}

fn swap_layout_terminals(
    layout: &mut PaneLayout,
    first_terminal_id: TerminalId,
    second_terminal_id: TerminalId,
) -> bool {
    fn swap(
        layout: &mut PaneLayout,
        first_terminal_id: TerminalId,
        second_terminal_id: TerminalId,
    ) -> (bool, bool) {
        match layout {
            PaneLayout::Leaf { terminal_id } if *terminal_id == first_terminal_id => {
                *terminal_id = second_terminal_id;
                (true, false)
            }
            PaneLayout::Leaf { terminal_id } if *terminal_id == second_terminal_id => {
                *terminal_id = first_terminal_id;
                (false, true)
            }
            PaneLayout::Leaf { .. } => (false, false),
            PaneLayout::Split { first, second, .. } => {
                let left = swap(first, first_terminal_id, second_terminal_id);
                let right = swap(second, first_terminal_id, second_terminal_id);
                (left.0 || right.0, left.1 || right.1)
            }
        }
    }

    swap(layout, first_terminal_id, second_terminal_id) == (true, true)
}

fn insert_around_terminal(
    layout: &mut PaneLayout,
    target_terminal_id: TerminalId,
    terminal_id: TerminalId,
    placement: PanePlacement,
) -> bool {
    match layout {
        PaneLayout::Leaf {
            terminal_id: target,
        } if *target == target_terminal_id => {
            let target = PaneLayout::Leaf {
                terminal_id: target_terminal_id,
            };
            let source = PaneLayout::Leaf { terminal_id };
            let (direction, first, second) = match placement {
                PanePlacement::Left => (SplitDirection::Vertical, source, target),
                PanePlacement::Right => (SplitDirection::Vertical, target, source),
                PanePlacement::Top => (SplitDirection::Horizontal, source, target),
                PanePlacement::Bottom => (SplitDirection::Horizontal, target, source),
                PanePlacement::Swap => return false,
            };
            *layout = PaneLayout::Split {
                direction,
                first: Box::new(first),
                second: Box::new(second),
            };
            true
        }
        PaneLayout::Leaf { .. } => false,
        PaneLayout::Split { first, second, .. } => {
            insert_around_terminal(first, target_terminal_id, terminal_id, placement)
                || insert_around_terminal(second, target_terminal_id, terminal_id, placement)
        }
    }
}

fn remove_from_layout(
    layout: PaneLayout,
    terminal_id: TerminalId,
) -> (Option<PaneLayout>, Option<TerminalId>) {
    match layout {
        PaneLayout::Leaf { terminal_id: id } if id == terminal_id => (None, None),
        leaf @ PaneLayout::Leaf { .. } => (Some(leaf), None),
        PaneLayout::Split {
            direction,
            first,
            second,
        } => {
            if first.contains(terminal_id) {
                let (first, fallback) = remove_from_layout(*first, terminal_id);
                match first {
                    Some(first) => (
                        Some(PaneLayout::Split {
                            direction,
                            first: Box::new(first),
                            second,
                        }),
                        fallback,
                    ),
                    None => {
                        let fallback = second.terminal_ids().first().copied();
                        (Some(*second), fallback)
                    }
                }
            } else if second.contains(terminal_id) {
                let (second, fallback) = remove_from_layout(*second, terminal_id);
                match second {
                    Some(second) => (
                        Some(PaneLayout::Split {
                            direction,
                            first,
                            second: Box::new(second),
                        }),
                        fallback,
                    ),
                    None => {
                        let fallback = first.terminal_ids().first().copied();
                        (Some(*first), fallback)
                    }
                }
            } else {
                (
                    Some(PaneLayout::Split {
                        direction,
                        first,
                        second,
                    }),
                    None,
                )
            }
        }
    }
}
