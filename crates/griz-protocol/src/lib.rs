//! Development IPC shared by the GUI and the session service. No GUI or FFI dependencies.
//! The app and core evolve together; wire compatibility is not maintained during development.
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::path::PathBuf;

pub const MAX_MESSAGE: usize = 16 * 1024 * 1024;
pub const MAX_INPUT: usize = 256 * 1024;
pub const MAX_COLS: u16 = 300;
pub const MAX_ROWS: u16 = 120;
pub const MAX_WORKSPACES: usize = 16;
pub const MAX_TERMINALS: usize = 64;
/// Bound frames per tab; a client view includes the active tab and, optionally, one preview tab.
pub const MAX_PANES_PER_TAB: usize = 8;
pub type WorkspaceId = u64;
pub type TerminalId = u64;

/// Horizontal splits stack panes top/bottom; vertical splits place them side by side.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaneLayout {
    Leaf {
        terminal_id: TerminalId,
    },
    Split {
        direction: SplitDirection,
        first: Box<PaneLayout>,
        second: Box<PaneLayout>,
    },
}

impl PaneLayout {
    pub fn terminal_ids(&self) -> Vec<TerminalId> {
        let mut ids = Vec::new();
        self.collect_terminal_ids(&mut ids);
        ids
    }

    pub fn contains(&self, id: TerminalId) -> bool {
        match self {
            Self::Leaf { terminal_id } => *terminal_id == id,
            Self::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }

    fn collect_terminal_ids(&self, ids: &mut Vec<TerminalId>) {
        match self {
            Self::Leaf { terminal_id } => ids.push(*terminal_id),
            Self::Split { first, second, .. } => {
                first.collect_terminal_ids(ids);
                second.collect_terminal_ids(ids);
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TabInfo {
    /// Stable tab identity, initially equal to the tab's first terminal ID.
    pub id: TerminalId,
    pub layout: PaneLayout,
}

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

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    WheelUp,
    WheelDown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MouseInput {
    pub action: MouseAction,
    pub button: Option<MouseButton>,
    pub x: f32,
    pub y: f32,
    pub cell_width: u16,
    pub cell_height: u16,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
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
    /// True while any terminal mouse tracking mode (X10/1000/1002/1003) is active.
    #[serde(default)]
    pub mouse_tracking: bool,
    /// Rows between this client's viewport and the live terminal area.
    #[serde(default)]
    pub scrollback_offset: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TerminalInfo {
    pub id: TerminalId,
    pub label: String,
    /// Starting directory, not a claim about the shell's current working directory.
    pub initial_cwd: String,
    /// This terminal's live shell directory, abbreviating HOME with ~.
    /// Retains its last known value after shell exit, independently of client selection.
    pub directory: String,
    pub shell_pid: u32,
    pub exited: Option<String>,
    /// OSC 7501 records from this terminal, with client-independent attention state.
    #[serde(default)]
    pub agents: Vec<AgentInfo>,
}

/// Explicit lifecycle state, never inferred from terminal output or inactivity.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Error,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProgramStatusKind {
    Permission,
    Question,
    Auth,
}

/// One OSC 7501 record. Text is decoded, untrusted, and never interpreted as markup.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgramStatusRecord {
    /// Empty string denotes the root record; other IDs are hierarchical paths.
    pub id: String,
    pub status: AgentStatus,
    pub app: String,
    pub title: String,
    pub message: String,
    pub kind: Option<ProgramStatusKind>,
    pub progress: Option<u8>,
    /// Monotonic within a shell generation, including across clear/reset.
    pub revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentInfo {
    /// Presentation snapshot: app inheritance resolved, message preview ≤512 bytes.
    /// The VT retains the original record and its full protocol-sized message.
    #[serde(flatten)]
    pub record: ProgramStatusRecord,
    pub generation: u64,
    /// Completion, input request, or failure has not been acknowledged yet.
    pub unread: bool,
}

/// Shared workspace chrome color; independent of terminal ANSI colors.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAccent {
    Blue,
    Purple,
    Pink,
    Orange,
    #[default]
    Green,
    Graphite,
}

impl WorkspaceAccent {
    pub const ALL: [Self; 6] = [
        Self::Blue,
        Self::Purple,
        Self::Pink,
        Self::Orange,
        Self::Green,
        Self::Graphite,
    ];
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceInfo {
    pub id: WorkspaceId,
    /// Display label: a custom name, or the selected terminal's directory basename.
    pub name: String,
    /// None means the name follows this client's selected terminal automatically.
    pub custom_name: Option<String>,
    /// Selected terminal's full directory for display, abbreviating HOME with ~.
    pub directory: String,
    pub accent: WorkspaceAccent,
    pub terminals: Vec<TerminalInfo>,
    pub tabs: Vec<TabInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TerminalFrame {
    pub id: TerminalId,
    pub shell_pid: u32,
    pub screen: Screen,
    pub exited: Option<String>,
}

/// A read-only preview subscription, independent of the client's real selection.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PreviewTarget {
    Workspace {
        workspace_id: WorkspaceId,
    },
    Terminal {
        workspace_id: WorkspaceId,
        terminal_id: TerminalId,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PreviewFrame {
    pub target: PreviewTarget,
    pub layout: PaneLayout,
    pub selected: TerminalId,
    pub terminals: Vec<TerminalFrame>,
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
    /// Frames for every pane in this client's active tab, in layout order.
    pub terminals: Vec<TerminalFrame>,
    /// The selected frame, retained for callers that only support one terminal.
    pub terminal: Option<TerminalFrame>,
    /// Only populated while this client is browsing a navigation preview.
    #[serde(default)]
    pub preview: Option<Box<PreviewFrame>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Attach {
        cols: u16,
        rows: u16,
        /// Used only by the first attachment to initialize a fresh core's workspace.
        /// Later attachments never recolor existing workspaces. Older clients use green.
        #[serde(default)]
        default_workspace_accent: WorkspaceAccent,
    },
    Command {
        serial: u64,
        command: Command,
    },
    Shutdown,
}

/// Every terminal operation is addressed explicitly, never to a global "current" shell.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Acknowledge exactly the notification the client saw, not a newer transition.
    AcknowledgeAgent {
        terminal_id: TerminalId,
        generation: u64,
        id: String,
        revision: u64,
    },
    /// Subscribe to one tab's screens without selecting, resizing, or focusing it.
    SetPreview {
        target: Option<PreviewTarget>,
    },
    /// Blank names use the selected shell's current directory as the name.
    /// The first terminal inherits that directory, or the service default if none is selected.
    CreateWorkspace {
        name: String,
        /// Chosen by the creating GUI. Older clients omit it and get green.
        #[serde(default)]
        accent: WorkspaceAccent,
    },
    /// Blank names use the directory of the selected terminal in this workspace.
    RenameWorkspace {
        workspace_id: WorkspaceId,
        name: String,
    },
    /// Change workspace chrome for every client without changing their selections.
    SetWorkspaceAccent {
        workspace_id: WorkspaceId,
        accent: WorkspaceAccent,
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
    /// Split the selected pane, placing the new terminal second in the split.
    SplitTerminal {
        terminal_id: TerminalId,
        direction: SplitDirection,
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
    Mouse {
        terminal_id: TerminalId,
        event: MouseInput,
    },
    /// Scroll this client's viewport, never another window or the PTY itself.
    /// Positive rows move down toward the live terminal area.
    Scroll {
        terminal_id: TerminalId,
        rows: i32,
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
    fn pane_layout_helpers_and_schema() {
        let layout = PaneLayout::Split {
            direction: SplitDirection::Horizontal,
            first: Box::new(PaneLayout::Leaf { terminal_id: 7 }),
            second: Box::new(PaneLayout::Split {
                direction: SplitDirection::Vertical,
                first: Box::new(PaneLayout::Leaf { terminal_id: 8 }),
                second: Box::new(PaneLayout::Leaf { terminal_id: 9 }),
            }),
        };
        assert_eq!(layout.terminal_ids(), vec![7, 8, 9]);
        assert!(layout.contains(8));
        assert!(!layout.contains(10));
        assert_eq!(
            serde_json::to_value(&layout).unwrap(),
            serde_json::json!({
                "type": "split",
                "direction": "horizontal",
                "first": {"type": "leaf", "terminal_id": 7},
                "second": {
                    "type": "split",
                    "direction": "vertical",
                    "first": {"type": "leaf", "terminal_id": 8},
                    "second": {"type": "leaf", "terminal_id": 9}
                }
            })
        );
        assert_eq!(
            serde_json::to_value(Command::SplitTerminal {
                terminal_id: 7,
                direction: SplitDirection::Vertical,
            })
            .unwrap(),
            serde_json::json!({
                "type": "split_terminal",
                "terminal_id": 7,
                "direction": "vertical"
            })
        );
    }

    #[test]
    fn mouse_input_schema_and_screen_default() {
        let event = MouseInput {
            action: MouseAction::Motion,
            button: Some(MouseButton::Left),
            x: 12.5,
            y: 34.0,
            cell_width: 8,
            cell_height: 16,
            shift: true,
            alt: false,
            ctrl: true,
        };
        assert_eq!(
            serde_json::to_value(Command::Mouse {
                terminal_id: 7,
                event: event.clone(),
            })
            .unwrap(),
            serde_json::json!({
                "type": "mouse",
                "terminal_id": 7,
                "event": {
                    "action": "motion",
                    "button": "left",
                    "x": 12.5,
                    "y": 34.0,
                    "cell_width": 8,
                    "cell_height": 16,
                    "shift": true,
                    "alt": false,
                    "ctrl": true
                }
            })
        );

        let screen: Screen = serde_json::from_value(serde_json::json!({
            "cols": 1,
            "rows": 1,
            "cells": [],
            "cursor_x": 0,
            "cursor_y": 0,
            "cursor_visible": true,
            "title": "",
            "application_cursor": false,
            "bracketed_paste": false
        }))
        .unwrap();
        assert!(!screen.mouse_tracking);
        assert_eq!(screen.scrollback_offset, 0);
        assert_eq!(
            serde_json::to_value(Command::Scroll {
                terminal_id: 7,
                rows: -3
            })
            .unwrap(),
            serde_json::json!({ "type": "scroll", "terminal_id": 7, "rows": -3 })
        );
    }

    #[test]
    fn workspace_creation_defaults_legacy_clients_to_green_and_preserves_explicit_accents() {
        let legacy: Command =
            serde_json::from_str(r#"{"type":"create_workspace","name":"Project"}"#).unwrap();
        assert!(matches!(
            legacy,
            Command::CreateWorkspace {
                accent: WorkspaceAccent::Green,
                ..
            }
        ));
        for accent in WorkspaceAccent::ALL {
            let command = Command::CreateWorkspace {
                name: "Project".into(),
                accent,
            };
            let decoded: Command =
                serde_json::from_value(serde_json::to_value(command).unwrap()).unwrap();
            assert!(
                matches!(decoded, Command::CreateWorkspace { name, accent: chosen } if name == "Project" && chosen == accent)
            );
        }
        assert!(
            serde_json::from_str::<Command>(
                r#"{"type":"create_workspace","name":"","accent":"invalid"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn attach_defaults_legacy_clients_to_green_and_preserves_explicit_accents() {
        let legacy: Request =
            serde_json::from_str(r#"{"type":"attach","cols":80,"rows":24}"#).unwrap();
        assert!(matches!(
            legacy,
            Request::Attach {
                default_workspace_accent: WorkspaceAccent::Green,
                ..
            }
        ));
        for accent in WorkspaceAccent::ALL {
            let request = Request::Attach {
                cols: 80,
                rows: 24,
                default_workspace_accent: accent,
            };
            let decoded: Request =
                serde_json::from_value(serde_json::to_value(request).unwrap()).unwrap();
            assert!(
                matches!(decoded, Request::Attach { cols: 80, rows: 24, default_workspace_accent } if default_workspace_accent == accent)
            );
        }
        assert!(
            serde_json::from_str::<Request>(
                r#"{"type":"attach","cols":80,"rows":24,"default_workspace_accent":"invalid"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn workspace_accents_round_trip() {
        for accent in WorkspaceAccent::ALL {
            let mut bytes = Vec::new();
            send(
                &mut bytes,
                &Command::SetWorkspaceAccent {
                    workspace_id: 2,
                    accent,
                },
            )
            .unwrap();
            assert!(matches!(
                receive::<Command>(&mut bytes.as_slice()).unwrap(),
                Command::SetWorkspaceAccent { workspace_id: 2, accent: decoded } if decoded == accent
            ));
        }
        assert!(serde_json::from_str::<WorkspaceAccent>(r#""not-a-color""#).is_err());
    }

    #[test]
    fn oversized_input_is_rejected_before_allocation() {
        assert!(receive::<Request>(&mut u32::MAX.to_be_bytes().as_slice()).is_err());
    }
}
