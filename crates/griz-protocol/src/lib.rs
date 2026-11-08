//! Versioned local IPC shared by the GUI and the session service. No GUI or FFI dependencies.
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::path::PathBuf;

pub const VERSION: u32 = 4;
pub const MAX_MESSAGE: usize = 16 * 1024 * 1024;
pub const MAX_INPUT: usize = 256 * 1024;
pub const MAX_COLS: u16 = 300;
pub const MAX_ROWS: u16 = 120;
pub const MAX_WORKSPACES: usize = 16;
pub const MAX_TERMINALS: usize = 64;
pub type WorkspaceId = u64;
pub type TerminalId = u64;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Cell {
    pub text: String,
    /// Display columns: 0 for the continuation of a wide glyph, otherwise 1 or 2.
    pub width: u8,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Screen {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<Vec<Cell>>,
    pub cursor_x: u16,
    pub cursor_y: u16,
    pub cursor_visible: bool,
    pub title: String,
    pub application_cursor: bool,
    pub bracketed_paste: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TerminalInfo {
    pub id: TerminalId,
    pub label: String,
    /// Starting directory, not a claim about the shell's current working directory.
    pub initial_cwd: String,
    pub shell_pid: u32,
    pub exited: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceInfo {
    pub id: WorkspaceId,
    pub name: String,
    pub terminals: Vec<TerminalInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TerminalFrame {
    pub id: TerminalId,
    pub shell_pid: u32,
    pub screen: Screen,
    pub exited: Option<String>,
}

/// A consistent catalog + selected screen for one client. Selection is per client.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ViewState {
    pub core_pid: u32,
    pub default_cwd: String,
    /// Highest command serial processed on this connection, including rejected commands.
    pub ack: u64,
    pub workspaces: Vec<WorkspaceInfo>,
    /// Zero when no workspaces remain; workspace IDs are always nonzero.
    pub active_workspace: WorkspaceId,
    pub active_terminal: Option<TerminalId>,
    pub terminal: Option<TerminalFrame>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Attach { version: u32, cols: u16, rows: u16 },
    Command { serial: u64, command: Command },
    Shutdown,
}

/// Every terminal operation is addressed explicitly, never to a global "current" shell.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Blank names use the selected shell's current directory as the name.
    /// The first terminal inherits that directory, or the service default if none is selected.
    CreateWorkspace {
        name: String,
    },
    /// Blank names use the directory of the selected terminal in this workspace.
    RenameWorkspace {
        workspace_id: WorkspaceId,
        name: String,
    },
    /// Remove the workspace and end all of its terminals and running processes.
    CloseWorkspace {
        workspace_id: WorkspaceId,
    },
    SelectWorkspace {
        workspace_id: WorkspaceId,
    },
    /// Create one tab/shell in this workspace. None uses the service's default directory.
    CreateTerminal {
        workspace_id: WorkspaceId,
        cwd: Option<String>,
    },
    SelectTerminal {
        terminal_id: TerminalId,
    },
    /// Explicitly end this terminal, including its running process.
    /// Closing the final terminal also removes its workspace.
    CloseTerminal {
        terminal_id: TerminalId,
    },
    Input {
        terminal_id: TerminalId,
        data: Vec<u8>,
    },
    Resize {
        terminal_id: TerminalId,
        cols: u16,
        rows: u16,
    },
    /// Replace only an ended shell, retaining its terminal ID and starting directory.
    StartShell {
        terminal_id: TerminalId,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Stopping,
    View {
        state: ViewState,
    },
    Error {
        serial: Option<u64>,
        message: String,
    },
}

/// Length-prefixed JSON. Bound allocation before reading or deserializing untrusted sizes.
pub fn receive<T: serde::de::DeserializeOwned>(r: &mut impl Read) -> io::Result<T> {
    let mut len = [0; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid IPC message length",
        ));
    }
    let mut bytes = vec![0; len];
    r.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn send(w: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC message too large",
        ));
    }
    w.write_all(&(bytes.len() as u32).to_be_bytes())?;
    w.write_all(&bytes)
}

pub fn dimensions(cols: u16, rows: u16) -> (u16, u16) {
    (cols.clamp(2, MAX_COLS), rows.clamp(2, MAX_ROWS))
}

pub fn socket_path() -> io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("GRIZ_SOCKET") {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    Ok(PathBuf::from(home).join(".local/state/griz/core.sock"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip() {
        let mut bytes = Vec::new();
        send(
            &mut bytes,
            &Request::Command {
                serial: 1,
                command: Command::Resize {
                    terminal_id: 7,
                    cols: 80,
                    rows: 24,
                },
            },
        )
        .unwrap();
        assert!(matches!(
            receive::<Request>(&mut bytes.as_slice()).unwrap(),
            Request::Command {
                serial: 1,
                command: Command::Resize {
                    terminal_id: 7,
                    cols: 80,
                    rows: 24
                }
            }
        ));
    }
    #[test]
    fn oversized_input_is_rejected_before_allocation() {
        assert!(receive::<Request>(&mut u32::MAX.to_be_bytes().as_slice()).is_err());
    }
}
