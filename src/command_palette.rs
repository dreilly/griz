//! Client-side quick navigation and command discovery. No PTY/session ownership here.
use crate::{Command, Dialog, State, settings::Action, ui::Palette};
use griz_protocol::{PreviewTarget, TerminalId, WorkspaceId};
use rune::{Color, Cx, El, Key, KeyEvent, TextEdit, Tx, col, el, row, text, text_input};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Navigate,
    Commands,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Workspace(WorkspaceId),
    Terminal(WorkspaceId, TerminalId),
    Action(Action),
    Settings,
    RenameWorkspace(WorkspaceId),
    CloseWorkspace(WorkspaceId),
    TerminalDirectory(WorkspaceId),
    StartShell(TerminalId),
    Reconnect,
}

pub(crate) struct PaletteState {
    mode: Option<Mode>,
    query: TextEdit,
    selected: Option<Target>,
    requested_preview: Option<PreviewTarget>,
}

impl Default for PaletteState {
    fn default() -> Self {
        Self {
            mode: None,
            query: TextEdit::new(""),
            selected: None,
            requested_preview: None,
        }
    }
}

impl PaletteState {
    pub fn is_open(&self) -> bool {
        self.mode.is_some()
    }

    pub fn reset_preview(&mut self) {
        self.requested_preview = None;
    }

    pub fn close(&mut self) {
        self.mode = None;
        self.query.focused = false;
        self.selected = None;
    }
}

struct Entry {
    target: Target,
    title: String,
    detail: String,
    shortcut: String,
    enabled: bool,
}

impl Entry {
    fn new(
        target: Target,
        title: impl Into<String>,
        detail: impl Into<String>,
        enabled: bool,
    ) -> Self {
        Self {
            target,
            title: title.into(),
            detail: detail.into(),
            shortcut: String::new(),
            enabled,
        }
    }

    fn key(&self) -> String {
        match self.target {
            Target::Workspace(id) => format!("palette-workspace-{id}"),
            Target::Terminal(_, id) => format!("palette-terminal-{id}"),
            Target::Action(action) => format!("palette-action-{}", action.id()),
            Target::Settings => "palette-settings".into(),
            Target::RenameWorkspace(_) => "palette-rename-workspace".into(),
            Target::CloseWorkspace(_) => "palette-close-workspace".into(),
            Target::TerminalDirectory(_) => "palette-terminal-directory".into(),
            Target::StartShell(_) => "palette-start-shell".into(),
            Target::Reconnect => "palette-reconnect".into(),
        }
    }
}

pub(crate) fn open(state: &mut State, action: Action, tx: &mut Tx) {
    if state.settings_open || state.dialog.is_some() {
        return;
    }
    let mode = if action == Action::QuickOpen {
        Mode::Navigate
    } else {
        Mode::Commands
    };
    state.palette.mode = Some(mode);
    state.palette.query = TextEdit::new("");
    state.palette.query.focused = true;
    state.palette.selected = None;
    state.preedit.clear();
    sync_preview(state);
    tx.blur();
}

pub(crate) fn close(state: &mut State) {
    state.palette.close();
    sync_preview(state);
}

/// Keep the subscription tied to the same resolved highlight used by Enter and rendering.
/// This also handles a target disappearing or a connection becoming unavailable.
pub(crate) fn sync_preview(state: &mut State) {
    let entries = entries(state);
    let target = selected_index(state, &entries).and_then(|index| match entries[index].target {
        Target::Workspace(workspace_id) => Some(PreviewTarget::Workspace { workspace_id }),
        Target::Terminal(workspace_id, terminal_id) => Some(PreviewTarget::Terminal {
            workspace_id,
            terminal_id,
        }),
        _ => None,
    });
    if target != state.palette.requested_preview
        && state.send(Command::SetPreview { target }).is_some()
    {
        state.palette.requested_preview = target;
    }
}

// All query words must match. Prefer contiguous matches, but allow abbreviations such as
// “nwsp” for “New workspace”. Work on Unicode chars, never byte offsets into user labels.
fn score(haystack: &str, query: &str) -> Option<usize> {
    let haystack = haystack.to_lowercase();
    let mut total = 0;
    for word in query.split_whitespace() {
        if let Some(index) = haystack.find(word) {
            total += 1000 + word.chars().count() * 20 + 100_usize.saturating_sub(index);
        } else {
            let mut chars = haystack.chars().enumerate();
            let mut first = None;
            let mut last = 0;
            for wanted in word.chars() {
                let (index, _) = chars.find(|(_, c)| *c == wanted)?;
                first.get_or_insert(index);
                last = index;
            }
            total += 100_usize.saturating_sub(last - first.unwrap_or(0));
        }
    }
    Some(total)
}

fn entries(state: &State) -> Vec<Entry> {
    let mut entries = Vec::new();
    match state.palette.mode {
        Some(Mode::Navigate) => {
            if let Some(snapshot) = &state.snapshot {
                for workspace in &snapshot.workspaces {
                    entries.push(Entry::new(
                        Target::Workspace(workspace.id),
                        &workspace.name,
                        format!("Workspace · {}", workspace.directory),
                        state.ready(),
                    ));
                    for (index, terminal) in workspace.terminals.iter().enumerate() {
                        // Automatic workspace names describe the selected tab, not its siblings.
                        // Only a custom workspace name is shared search context for every tab.
                        let directory_name = std::path::Path::new(&terminal.directory)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or(&terminal.directory);
                        let prefix = match &workspace.custom_name {
                            Some(name) => format!("{name} › {directory_name}"),
                            None => directory_name.to_owned(),
                        };
                        entries.push(Entry::new(
                            Target::Terminal(workspace.id, terminal.id),
                            format!(
                                "{} › Terminal {}{}",
                                prefix,
                                index + 1,
                                if terminal.exited.is_some() {
                                    " · exited"
                                } else {
                                    ""
                                }
                            ),
                            format!("Terminal · {}", terminal.directory),
                            state.ready(),
                        ));
                    }
                }
            }
        }
        Some(Mode::Commands) => {
            for action in Action::ALL {
                if matches!(action, Action::QuickOpen | Action::CommandPalette) {
                    continue;
                }
                let available = match action {
                    Action::ToggleSidebar | Action::IncreaseFontSize | Action::DecreaseFontSize => {
                        true
                    }
                    Action::NewWorkspace => state.ready(),
                    Action::SplitHorizontal | Action::SplitVertical => state.can_split(),
                    _ => state.ready() && state.active_terminal().is_some(),
                };
                let (title, detail) = if action == Action::NewWorkspace {
                    (
                        "New workspace here…",
                        state
                            .workspace()
                            .map(|w| w.directory.as_str())
                            .or_else(|| state.snapshot.as_ref().map(|s| s.default_cwd.as_str()))
                            .unwrap_or("Start a workspace in the current directory"),
                    )
                } else {
                    (action.label(), "App command")
                };
                let mut entry = Entry::new(Target::Action(action), title, detail, available);
                entry.shortcut = state
                    .settings
                    .chord(action)
                    .map(|c| c.label())
                    .unwrap_or_default();
                entries.push(entry);
            }
            entries.push(Entry::new(
                Target::Settings,
                "Open Settings…",
                "Appearance and keyboard shortcuts",
                true,
            ));
            if let Some(workspace) = state.workspace() {
                entries.push(Entry::new(
                    Target::TerminalDirectory(workspace.id),
                    "New terminal in directory…",
                    "Choose an absolute starting directory",
                    state.ready(),
                ));
                entries.push(Entry::new(
                    Target::RenameWorkspace(workspace.id),
                    "Rename workspace…",
                    &workspace.name,
                    state.ready(),
                ));
                entries.push(Entry::new(
                    Target::CloseWorkspace(workspace.id),
                    "Close workspace…",
                    &workspace.name,
                    state.ready(),
                ));
            }
            if state.shell_exited()
                && let Some(id) = state.active_terminal()
            {
                entries.push(Entry::new(
                    Target::StartShell(id),
                    "Start new shell",
                    "Replace the exited shell in the active terminal",
                    state.ready(),
                ));
            }
            entries.push(Entry::new(
                Target::Reconnect,
                "Reconnect to service",
                "Reattach without stopping running shells",
                !state.offline && !state.online,
            ));
        }
        None => return entries,
    }
    let query = state.palette.query.text.to_lowercase();
    let mut ranked: Vec<_> = entries
        .into_iter()
        .filter_map(|entry| {
            let searchable = format!("{} {}", entry.title, entry.detail);
            score(&searchable, &query).map(|score| (score, entry))
        })
        .collect();
    ranked.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    ranked.into_iter().map(|(_, entry)| entry).collect()
}

fn selected_index(state: &State, entries: &[Entry]) -> Option<usize> {
    entries
        .iter()
        .position(|e| e.enabled && Some(e.target) == state.palette.selected)
        .or_else(|| entries.iter().position(|e| e.enabled))
}

fn activate(state: &mut State, target: Target, tx: &mut Tx) {
    // Revalidate against the latest catalog and connection, not the frame the user clicked.
    if !entries(state)
        .iter()
        .any(|e| e.target == target && e.enabled)
    {
        return;
    }
    close(state);
    tx.blur();
    match target {
        Target::Workspace(workspace_id) => state.change(Command::SelectWorkspace { workspace_id }),
        Target::Terminal(workspace_id, terminal_id) => {
            if state
                .snapshot
                .as_ref()
                .is_some_and(|s| s.active_workspace != workspace_id)
            {
                // The core requires SelectWorkspace before a terminal in another workspace.
                // Queue both in order and gate typing on the final selection acknowledgement.
                state.change(Command::SelectWorkspace { workspace_id });
                if state.pending.is_none() {
                    return;
                }
                if let Some(serial) = state.send(Command::SelectTerminal { terminal_id }) {
                    state.pending = Some(serial);
                }
            } else {
                state.change(Command::SelectTerminal { terminal_id });
            }
        }
        Target::Action(action) => state.act(action, tx),
        Target::Settings => state.open_settings(tx),
        Target::RenameWorkspace(id) => state.open_dialog(Dialog::RenameWorkspace(id), tx),
        Target::CloseWorkspace(id) => {
            if let Some(w) = state.workspace().filter(|w| w.id == id) {
                state.open_dialog(Dialog::CloseWorkspace(id, w.name.clone()), tx);
            }
        }
        Target::TerminalDirectory(id) => state.open_dialog(Dialog::NewTerminal(id), tx),
        Target::StartShell(terminal_id) => state.change(Command::StartShell { terminal_id }),
        Target::Reconnect => state.connect(),
    }
}

pub(crate) fn key(state: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    if !state.palette.is_open() {
        return false;
    }
    if event.key == Key::Escape {
        close(state);
        tx.blur();
        return true;
    }
    let ctrl = event.mods.ctrl
        && !event.mods.alt
        && !event.mods.logo
        && !event.mods.cmd
        && !event.mods.shift;
    let letter = match &event.key {
        Key::Text(t) if t.chars().count() == 1 => t.chars().next(),
        Key::Command(c) => Some(*c),
        _ => None,
    }
    .map(|c| c.to_ascii_lowercase());
    let next = event.key == Key::Down || (ctrl && letter == Some('n'));
    let previous = event.key == Key::Up || (ctrl && letter == Some('p'));
    if next || previous {
        let entries = entries(state);
        let enabled: Vec<_> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.enabled)
            .map(|(i, _)| i)
            .collect();
        if !enabled.is_empty() {
            let current = selected_index(state, &entries).unwrap();
            let index = enabled.iter().position(|i| *i == current).unwrap();
            let index = if next {
                (index + 1) % enabled.len()
            } else {
                (index + enabled.len() - 1) % enabled.len()
            };
            let entry = &entries[enabled[index]];
            state.palette.selected = Some(entry.target);
            tx.scroll_into_view(entry.key());
            sync_preview(state);
        }
        return true;
    }
    if event.key == Key::Enter {
        if !tx.key_repeat() {
            let entries = entries(state);
            if let Some(index) = selected_index(state, &entries) {
                activate(state, entries[index].target, tx);
            }
        }
        return true;
    }
    false
}

pub(crate) fn panel(state: &State, p: &Palette, cx: &Cx<State>) -> Option<El> {
    let mode = state.palette.mode?;
    let entries = entries(state);
    let selected = selected_index(state, &entries);
    let [width, height] = cx.layout_size("root").unwrap_or([1100.0, 720.0]);
    let top = (height * 0.12).clamp(12.0, 80.0);
    let show_preview = mode == Mode::Navigate && height >= 480.0;
    let side_by_side = show_preview && width >= 900.0;
    let panel_width = (width - 24.0).clamp(1.0, if side_by_side { 1040.0 } else { 640.0 });
    let panel_height = (height - top - 16.0).clamp(1.0, if show_preview { 560.0 } else { 460.0 });
    let inner_width = (panel_width - 24.0).max(1.0);
    let list_width = if side_by_side {
        inner_width * 0.42
    } else {
        inner_width
    };
    let mut list = col()
        .key("palette-results")
        .role("listbox")
        .label("Palette results")
        .w(list_width)
        .grow(1.0)
        .min_h(0.0)
        .gap(4.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        });
    for (index, entry) in entries.iter().enumerate() {
        let target = entry.target;
        let active = selected == Some(index);
        list = list.child(
            row()
                .key(entry.key())
                .role("option")
                .label(&entry.title)
                .w_full()
                .padding(10.0)
                .gap(12.0)
                .center_y()
                .radius(6.0)
                .fill(if active {
                    p.accent.alpha(0.15)
                } else {
                    Color::TRANSPARENT
                })
                .hover(p.ink.alpha(0.07))
                .disabled(!entry.enabled)
                .preserve_focus_on_press(true)
                .on_press_tx(move |s: &mut State, tx| activate(s, target, tx))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(3.0)
                        .child(
                            text(&entry.title)
                                .font(p.label)
                                .color(if active { p.accent } else { p.ink })
                                .truncate(),
                        )
                        .child(text(&entry.detail).font(p.label).color(p.muted).truncate()),
                )
                .child(text(&entry.shortcut).font(p.label).color(p.muted)),
        );
    }
    if entries.is_empty() {
        list = list.child(
            text(
                if mode == Mode::Navigate
                    && state
                        .snapshot
                        .as_ref()
                        .is_none_or(|s| s.workspaces.is_empty())
                {
                    "No workspaces yet. Open the command palette to create one."
                } else {
                    "No matches"
                },
            )
            .key("palette-empty")
            .font(p.label)
            .color(p.muted)
            .wrap()
            .padding(12.0),
        );
    }
    let body = if show_preview {
        let selected_entry = selected.map(|index| &entries[index]);
        let target = selected_entry.and_then(|entry| match entry.target {
            Target::Workspace(workspace_id) => Some(PreviewTarget::Workspace { workspace_id }),
            Target::Terminal(workspace_id, terminal_id) => Some(PreviewTarget::Terminal {
                workspace_id,
                terminal_id,
            }),
            _ => None,
        });
        let preview_width = if side_by_side {
            inner_width - list_width - 12.0
        } else {
            inner_width
        };
        // Reserve the search, heading, footer and their padding before fitting the miniature.
        let available = (panel_height
            - 80.0
            - p.section.line_px()
            - p.input.line_px()
            - p.label.line_px() * 2.0)
            .max(40.0);
        let preview_height = if side_by_side {
            available
        } else {
            (available * 0.45).min(180.0)
        };
        let preview = crate::navigation_preview::panel(
            state,
            p,
            target,
            selected_entry.map_or("PREVIEW", |entry| &entry.title),
            preview_width,
            preview_height,
        );
        let container = if side_by_side { row() } else { col() };
        container
            .key("palette-body")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .gap(12.0)
            .child(list)
            .child(preview)
    } else {
        list
    };
    Some(
        el().key("palette-scrim")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .fill(Color::BLACK.alpha(0.55))
            .child(
                col()
                    .key("command-palette")
                    .absolute(((width - panel_width) / 2.0).max(12.0), top)
                    .w(panel_width)
                    .h(panel_height)
                    .padding(12.0)
                    .gap(10.0)
                    .radius(12.0)
                    .fill(p.panel)
                    .border(1.0, p.chrome_line)
                    .shadow(8.0, 28.0, Color::BLACK.alpha(0.4))
                    .role("dialog")
                    .label(if mode == Mode::Navigate {
                        "Jump to workspace or terminal"
                    } else {
                        "Command palette"
                    })
                    .modal(true, close)
                    .on_key(key)
                    .child(
                        text(if mode == Mode::Navigate {
                            "JUMP TO"
                        } else {
                            "COMMANDS"
                        })
                        .font(p.section)
                        .color(p.muted),
                    )
                    .child({
                        let mut input = text_input(&state.palette.query, p.input)
                            .key("palette-input")
                            .label("Search palette")
                            .single_line()
                            .w_full()
                            .padding(12.0)
                            .radius(6.0)
                            .fill(p.ink.alpha(0.06))
                            .color(p.ink)
                            .placeholder(
                                if mode == Mode::Navigate {
                                    "Search workspaces, terminals, directories…"
                                } else {
                                    "What would you like to do?"
                                },
                                p.muted,
                            )
                            .on_edit(|s: &mut State| &mut s.palette.query)
                            .on_input_change(|s: &mut State, tx| {
                                s.palette.selected = None;
                                sync_preview(s);
                                if let Some(entry) = self::entries(s).iter().find(|e| e.enabled) {
                                    tx.scroll_into_view(entry.key());
                                }
                            })
                            .on_input_key(crate::app_shortcut)
                            .on_submit(|s: &mut State, tx| {
                                let entries = self::entries(s);
                                if let Some(index) = selected_index(s, &entries) {
                                    activate(s, entries[index].target, tx);
                                }
                            });
                        if let Some(index) = selected {
                            input = input.active_descendant(entries[index].key());
                        }
                        input
                    })
                    .child(body)
                    .child(
                        text("↑ ↓ / Ctrl-N Ctrl-P to move · Enter to select · Esc to dismiss")
                            .font(p.label)
                            .color(p.muted)
                            .wrap(),
                    ),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::score;
    #[test]
    fn matching_is_fuzzy_multiword_and_unicode_safe() {
        assert!(score("New workspace here", "nwsp").is_some());
        assert!(score("Daily › Terminal 2 · /code/griz", "griz daily 2").is_some());
        assert!(score("Équipe › Terminal 1", "éq").is_some());
        assert!(score("Daily", "xyz").is_none());
        assert!(
            score("New workspace", "workspace").unwrap()
                > score("Work space", "workspace").unwrap()
        );
    }
}
