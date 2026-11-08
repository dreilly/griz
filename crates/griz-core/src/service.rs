use crate::clients::Outbox;
use crate::session::Session;
use crate::validation::{validate_cwd, workspace_name};
use crate::{ClientId, Message};
use griz_protocol::{
    Command, Event, MAX_INPUT, MAX_TERMINALS, MAX_WORKSPACES, Screen, TerminalFrame, TerminalId,
    TerminalInfo, ViewState, WorkspaceId, WorkspaceInfo, dimensions,
};
use griz_vt::Terminal;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc::Receiver, mpsc::RecvTimeoutError, mpsc::SyncSender};
use std::time::{Duration, Instant};

const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const DEFAULT_SIZE: (u16, u16) = (80, 24);

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct TerminalRecord {
    id: TerminalId,
    label: String,
    initial_cwd: PathBuf,
    generation: u64,
    grid: (u16, u16),
    vt: Terminal,
    session: Session,
    exited: Option<String>,
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
            initial_cwd: cwd,
            generation,
            grid,
            vt,
            session,
            exited: None,
        })
    }

    fn info(&self) -> TerminalInfo {
        TerminalInfo {
            id: self.id,
            label: self.label.clone(),
            initial_cwd: self.initial_cwd.to_string_lossy().into_owned(),
            shell_pid: self.session.pid,
            exited: self.exited.clone(),
        }
    }
}

struct Workspace {
    id: WorkspaceId,
    name: String,
    terminals: Vec<TerminalRecord>,
    next_terminal_number: usize,
}

struct Client {
    active_workspace: WorkspaceId,
    selected: HashMap<WorkspaceId, Option<TerminalId>>,
    dimensions: Option<(u16, u16)>,
    ack: u64,
    outbox: Arc<Outbox>,
}

#[derive(Clone, Copy)]
enum Outcome {
    Requester,
    Catalog,
}

pub struct Service {
    shell: PathBuf,
    default_cwd: PathBuf,
    tx: SyncSender<Message>,
    workspaces: Vec<Workspace>,
    clients: BTreeMap<ClientId, Client>,
    next_workspace_id: WorkspaceId,
    next_terminal_id: TerminalId,
    dirty: HashSet<TerminalId>,
    frame_deadline: Option<Instant>,
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
                name: "Daily".into(),
                terminals: vec![terminal],
                next_terminal_number: 2,
            }],
            clients: BTreeMap::new(),
            next_workspace_id: 2,
            next_terminal_id: 2,
            dirty: HashSet::new(),
            frame_deadline: None,
        })
    }

    pub fn run(&mut self, rx: Receiver<Message>) -> Result<()> {
        let result = (|| {
            loop {
                // A continuously full PTY queue must not starve the publication timer.
                if self
                    .frame_deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
                {
                    self.flush_dirty()?;
                }
                let message = if let Some(deadline) = self.frame_deadline {
                    match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                        Ok(message) => message,
                        Err(RecvTimeoutError::Timeout) => {
                            self.flush_dirty()?;
                            continue;
                        }
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
                        outbox,
                    } => self.attach(client_id, cols, rows, outbox)?,
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
        outbox: Arc<Outbox>,
    ) -> Result<()> {
        let active_workspace = self.workspaces[0].id;
        let selected = self
            .workspaces
            .iter()
            .map(|workspace| {
                (
                    workspace.id,
                    workspace.terminals.first().map(|term| term.id),
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
                ack: 0,
                outbox,
            },
        );
        if let (Some(size), Some(terminal_id)) =
            (client_dimensions, self.active_terminal(client_id))
        {
            self.resize_terminal(terminal_id, size);
        }
        self.publish_views(&[client_id])
    }

    fn detach(&mut self, client_id: ClientId) {
        self.clients.remove(&client_id);
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

        match self.apply_command(client_id, command) {
            Ok(Outcome::Requester) => self.publish_views(&[client_id]),
            Ok(Outcome::Catalog) => {
                self.dirty.clear();
                self.frame_deadline = None;
                self.publish_all()
            }
            Err(message) => {
                outbox.error(Some(serial), message);
                self.publish_views(&[client_id])
            }
        }
    }

    fn apply_command(
        &mut self,
        client_id: ClientId,
        command: Command,
    ) -> std::result::Result<Outcome, String> {
        match command {
            Command::CreateWorkspace { name } => self.create_workspace(client_id, name),
            Command::RenameWorkspace { workspace_id, name } => {
                let name = workspace_name(&name)?;
                let workspace = self
                    .workspaces
                    .iter_mut()
                    .find(|workspace| workspace.id == workspace_id)
                    .ok_or_else(|| format!("unknown workspace {workspace_id}"))?;
                workspace.name = name;
                Ok(Outcome::Catalog)
            }
            Command::SelectWorkspace { workspace_id } => {
                let workspace = self
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.id == workspace_id)
                    .ok_or_else(|| format!("unknown workspace {workspace_id}"))?;
                let fallback = workspace.terminals.first().map(|terminal| terminal.id);
                let client = self.clients.get_mut(&client_id).unwrap();
                let selected = *client.selected.entry(workspace_id).or_insert(fallback);
                let size = client.dimensions;
                if let (Some(terminal_id), Some(size)) = (selected, size) {
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
                if let Some(size) = self.clients[&client_id].dimensions {
                    self.resize_terminal(terminal_id, size);
                }
                self.clients
                    .get_mut(&client_id)
                    .unwrap()
                    .selected
                    .insert(workspace_id, Some(terminal_id));
                Ok(Outcome::Requester)
            }
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

    fn create_workspace(
        &mut self,
        client_id: ClientId,
        name: String,
    ) -> std::result::Result<Outcome, String> {
        if self.workspaces.len() >= MAX_WORKSPACES {
            return Err(format!("workspace limit ({MAX_WORKSPACES}) reached"));
        }
        if self.terminal_count() >= MAX_TERMINALS {
            return Err(format!("terminal limit ({MAX_TERMINALS}) reached"));
        }
        let name = workspace_name(&name)?;
        let workspace_id = self.take_workspace_id();
        let terminal_id = self.take_terminal_id();
        let grid = self.clients[&client_id].dimensions.unwrap_or(DEFAULT_SIZE);
        let terminal = TerminalRecord::spawn(
            terminal_id,
            "Terminal 1".into(),
            self.default_cwd.clone(),
            1,
            grid,
            &self.shell,
            self.tx.clone(),
        )
        .map_err(|error| format!("cannot create workspace: {error}"))?;
        self.workspaces.push(Workspace {
            id: workspace_id,
            name,
            terminals: vec![terminal],
            next_terminal_number: 2,
        });
        for client in self.clients.values_mut() {
            client.selected.insert(workspace_id, Some(terminal_id));
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

    fn close_terminal(&mut self, terminal_id: TerminalId) -> std::result::Result<Outcome, String> {
        let (workspace_index, terminal_index) = self
            .terminal_location(terminal_id)
            .ok_or_else(|| format!("unknown terminal {terminal_id}"))?;
        let workspace_id = self.workspaces[workspace_index].id;
        let mut terminal = self.workspaces[workspace_index]
            .terminals
            .remove(terminal_index);
        terminal.session.terminate();
        let fallback = self.workspaces[workspace_index]
            .terminals
            .first()
            .map(|terminal| terminal.id);
        let mut fallback_sizes = Vec::new();
        for client in self.clients.values_mut() {
            if client.selected.get(&workspace_id) == Some(&Some(terminal_id)) {
                client.selected.insert(workspace_id, fallback);
                if client.active_workspace == workspace_id
                    && let Some(size) = client.dimensions
                {
                    fallback_sizes.push(size);
                }
            }
        }
        if let Some(fallback_id) = fallback {
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
        let responses = terminal.vt.take_responses();
        if !responses.is_empty()
            && let Some(input) = terminal.session.input()
            && let Err(error) = input.try_send(responses)
        {
            eprintln!("griz-core: terminal {terminal_id} response: {error}");
        }
        if self.terminal_is_visible(terminal_id) {
            self.dirty.insert(terminal_id);
            self.frame_deadline
                .get_or_insert_with(|| Instant::now() + FRAME_INTERVAL);
        }
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
        if self.terminal_is_visible(terminal_id) {
            self.dirty.insert(terminal_id);
            self.frame_deadline
                .get_or_insert_with(|| Instant::now() + FRAME_INTERVAL);
        }
    }

    fn flush_dirty(&mut self) -> Result<()> {
        let clients = self
            .clients
            .keys()
            .copied()
            .filter(|client_id| {
                self.active_terminal(*client_id)
                    .is_some_and(|terminal_id| self.dirty.contains(&terminal_id))
            })
            .collect::<Vec<_>>();
        self.dirty.clear();
        self.frame_deadline = None;
        self.publish_views(&clients)
    }

    fn publish_all(&mut self) -> Result<()> {
        let clients = self.clients.keys().copied().collect::<Vec<_>>();
        self.publish_views(&clients)
    }

    fn publish_views(&mut self, client_ids: &[ClientId]) -> Result<()> {
        let catalog = self.catalog();
        let specs = client_ids
            .iter()
            .filter_map(|client_id| {
                self.clients.get(client_id).map(|client| {
                    (
                        *client_id,
                        client.active_workspace,
                        client
                            .selected
                            .get(&client.active_workspace)
                            .copied()
                            .flatten(),
                        client.ack,
                        client.outbox.clone(),
                    )
                })
            })
            .collect::<Vec<_>>();
        let terminal_ids = specs
            .iter()
            .filter_map(|(_, _, terminal_id, _, _)| *terminal_id)
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
        for (_, active_workspace, active_terminal, ack, outbox) in specs {
            let terminal = active_terminal.and_then(|terminal_id| {
                let (workspace_index, terminal_index) = self.terminal_location(terminal_id)?;
                let record = &self.workspaces[workspace_index].terminals[terminal_index];
                Some(TerminalFrame {
                    id: terminal_id,
                    shell_pid: record.session.pid,
                    screen: screens.get(&terminal_id)?.clone(),
                    exited: record.exited.clone(),
                })
            });
            outbox.view(Event::View {
                state: ViewState {
                    core_pid: std::process::id(),
                    default_cwd: self.default_cwd.to_string_lossy().into_owned(),
                    ack,
                    workspaces: catalog.clone(),
                    active_workspace,
                    active_terminal,
                    terminal,
                },
            });
        }
        Ok(())
    }

    fn catalog(&self) -> Vec<WorkspaceInfo> {
        self.workspaces
            .iter()
            .map(|workspace| WorkspaceInfo {
                id: workspace.id,
                name: workspace.name.clone(),
                terminals: workspace
                    .terminals
                    .iter()
                    .map(TerminalRecord::info)
                    .collect(),
            })
            .collect()
    }

    fn active_terminal(&self, client_id: ClientId) -> Option<TerminalId> {
        let client = self.clients.get(&client_id)?;
        client
            .selected
            .get(&client.active_workspace)
            .copied()
            .flatten()
    }

    fn terminal_is_visible(&self, terminal_id: TerminalId) -> bool {
        self.clients
            .keys()
            .any(|client_id| self.active_terminal(*client_id) == Some(terminal_id))
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
