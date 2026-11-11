//! Program-reported OSC 7501 records across every workspace and terminal pane.
use crate::{Command, State, harness_icons::Harness, ui::Palette, workspace_ui::accent_color};
use griz_protocol::{AgentStatus, ProgramStatusKind, ProgramStatusRecord, TerminalId, WorkspaceId};
use rune::{Color, El, col, el, oklch, row, text};

const COMPLETION_FADE: &[rune::Beat] = &[
    rune::Beat::new(0.0).opacity(1.0),
    rune::Beat::new(1.1).opacity(0.0),
];

pub(crate) type CompletionSweeps =
    std::collections::HashMap<(TerminalId, String, u64), CompletionSweep>;

pub(crate) struct CompletionSweep {
    offset: rune::Tween<[f32; 2]>,
    opacity: rune::Tween<f32>,
}

/// Animate only a completion observed on this live connection, never a historical
/// Done record on attachment, reconnection, or a replacement shell/core.
pub(crate) fn update_completions(state: &mut State, next: &griz_protocol::ViewState) {
    let Some(previous) = state
        .snapshot
        .as_ref()
        .filter(|old| state.online && old.core_pid == next.core_pid)
    else {
        state.completion_sweeps.clear();
        return;
    };
    let mut present = std::collections::HashSet::new();
    for terminal in next.workspaces.iter().flat_map(|w| &w.terminals) {
        for agent in &terminal.agents {
            if agent.record.status != AgentStatus::Done {
                continue;
            }
            let key = (terminal.id, agent.record.id.clone(), agent.generation);
            present.insert(key.clone());
            let transitioned = previous
                .workspaces
                .iter()
                .flat_map(|w| &w.terminals)
                .find(|t| t.id == terminal.id)
                .and_then(|t| {
                    t.agents.iter().find(|a| {
                        a.record.id == agent.record.id && a.generation == agent.generation
                    })
                })
                .is_some_and(|old| {
                    old.record.status != AgentStatus::Done
                        && agent.record.revision > old.record.revision
                });
            if transitioned {
                let duration = std::time::Duration::from_millis(1100);
                state.completion_sweeps.insert(
                    key,
                    CompletionSweep {
                        offset: rune::Tween::linear([-76.0, 0.0], [240.0, 0.0], duration),
                        opacity: rune::Tween::linear(0.30, 0.0, duration),
                    },
                );
            }
        }
    }
    state
        .completion_sweeps
        .retain(|key, _| present.contains(key));
}

fn status_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Idle => "Idle",
        AgentStatus::Working => "Working",
        AgentStatus::Blocked => "Waiting",
        AgentStatus::Done => "Done",
        AgentStatus::Error => "Error",
    }
}

fn status_color(status: AgentStatus, p: &Palette) -> Color {
    match status {
        AgentStatus::Idle => p.muted,
        AgentStatus::Working => oklch(0.78, 0.12, 250.0),
        AgentStatus::Blocked => oklch(0.82, 0.13, 80.0),
        AgentStatus::Done => p.accent,
        AgentStatus::Error => oklch(0.74, 0.16, 25.0),
    }
}

/// Outside the grid, disarm invisible direction/formatting controls as well as
/// control characters. Render everything else as ordinary text, never markup.
fn display_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(*c as u32,
        0x00ad | 0x034f | 0x061c | 0x180e | 0x200b..=0x200f |
        0x2028..=0x202e | 0x2060..=0x206f | 0xfeff)
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

fn record_label(record: &ProgramStatusRecord) -> String {
    [&record.title, &record.app, &record.id]
        .into_iter()
        .map(|s| display_text(s))
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| "Program".into())
}

pub(crate) fn section(state: &State, p: &Palette) -> El {
    let agents: Vec<_> = state
        .snapshot
        .iter()
        .flat_map(|s| &s.workspaces)
        .flat_map(|w| {
            w.terminals
                .iter()
                .flat_map(move |t| t.agents.iter().map(move |a| (w, t, a)))
        })
        .collect();
    let unread = agents.iter().filter(|(_, _, a)| a.unread).count();
    let mut list = col()
        .key("agent-list")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(4.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        });
    if agents.is_empty() {
        list = list.child(
            text("No agents reporting status")
                .key("agents-empty")
                .font(p.label)
                .color(p.muted),
        );
    }
    for status in [
        AgentStatus::Working,
        AgentStatus::Blocked,
        AgentStatus::Error,
        AgentStatus::Done,
        AgentStatus::Idle,
    ] {
        let group: Vec<_> = agents
            .iter()
            .filter(|(_, _, a)| a.record.status == status)
            .collect();
        if group.is_empty() {
            continue;
        }
        let color = status_color(status, p);
        for (workspace, terminal, agent) in group {
            let workspace_id = workspace.id;
            let terminal_id = terminal.id;
            let record = &agent.record;
            let id = record.id.clone();
            let generation = agent.generation;
            let revision = record.revision;
            let label = record_label(record);
            let harness = Harness::from_app(&record.app);
            let message = display_text(&record.message);
            let detail = match (message.is_empty(), record.progress) {
                (true, None) => String::new(),
                (true, Some(progress)) => format!("{progress}%"),
                (false, None) => message,
                (false, Some(progress)) => format!("{message} · {progress}%"),
            };
            let kind = match record.kind {
                Some(ProgramStatusKind::Permission) => "Approval",
                Some(ProgramStatusKind::Question) => "Question",
                Some(ProgramStatusKind::Auth) => "Authentication",
                None => "",
            };
            let active = state.snapshot.as_ref().is_some_and(|s| {
                s.active_workspace == workspace_id && s.active_terminal == Some(terminal_id)
            });
            let accent = accent_color(workspace.accent);
            let key = if id.is_empty() {
                format!("agent-{terminal_id}")
            } else {
                format!("agent-{terminal_id}:{id}")
            };
            let mut entry = col()
                .key(key.clone())
                .role("button")
                .label(format!(
                    "{label} · {} · {} · {}{}",
                    harness.name(),
                    workspace.name,
                    status_label(status),
                    if agent.unread {
                        " · new notification"
                    } else {
                        ""
                    }
                ))
                .help(format!(
                    "{label} · {} · {} · {kind} · {detail}",
                    terminal.label, terminal.directory
                ))
                .focusable(true)
                .disabled(!state.ready())
                .w_full()
                .min_w(0.0)
                .padding(8.0)
                .gap(3.0)
                .radius(6.0)
                .fill(if active {
                    accent.alpha(0.12)
                } else if agent.unread {
                    color.alpha(0.08)
                } else {
                    Color::TRANSPARENT
                })
                .hover(p.ink.alpha(0.06))
                .focus_ring(2.0, 2.0, accent)
                .on_press_tx(move |s: &mut State, tx| {
                    focus(s, workspace_id, terminal_id, &id, generation, revision);
                    tx.blur();
                })
                .child(
                    row()
                        .w_full()
                        .min_w(0.0)
                        .gap(6.0)
                        .center_y()
                        .child(harness.badge(format!("{key}-harness-icon"), p))
                        .child(
                            text(label)
                                .font(p.label)
                                .color(p.ink)
                                .grow(1.0)
                                .min_w(0.0)
                                .truncate(),
                        )
                        .child(
                            text(if agent.unread { "●" } else { "○" })
                                .font(p.label)
                                .color(color),
                        ),
                )
                .child(
                    text(format!("{} · {}", workspace.name, terminal.label))
                        .font(p.label)
                        .color(p.muted)
                        .truncate(),
                );
            // Only the decorative outline breathes; text, hit targets, and layout
            // stay still. Rune's renderer freezes loops under Reduce Motion and
            // suspends them when the window is not visible.
            let blue = oklch(0.75, 0.12, 250.0);
            if matches!(status, AgentStatus::Working | AgentStatus::Done) {
                let mut outline = el()
                    .key(format!("{key}-status-border"))
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .radius(6.0);
                outline = if status == AgentStatus::Working {
                    outline
                        .border_gradient(1.0, oklch(0.86, 0.08, 245.0), blue.alpha(0.65))
                        .looping(rune::LoopSpec::opacity(2.8, 0.0, 0.55, 0.95))
                } else {
                    outline.border(1.0, blue.alpha(0.85))
                };
                entry = entry.child(outline);
            }
            if let Some(sweep) =
                state
                    .completion_sweeps
                    .get(&(terminal_id, record.id.clone(), generation))
            {
                // Non-interactive overlay: clipping keeps the wave inside this card.
                // Tween sources survive view rebuilds/remounts, so a read receipt or
                // sidebar toggle cannot restart a completed sweep.
                entry = entry.child(
                    el().key(format!("{key}-completion-clip"))
                        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                        .clip_rounded(6.0)
                        .sequence(rune::Sequence::new(COMPLETION_FADE))
                        .child(
                            el().key(format!("{key}-completion-sweep"))
                                .pin(Some(0.0), Some(0.0), None, Some(0.0))
                                .w(76.0)
                                .linear_gradient(
                                    std::f32::consts::FRAC_PI_2,
                                    [(0.0, blue.alpha(0.0)), (0.65, blue), (1.0, blue.alpha(0.0))],
                                )
                                .live_offset(&sweep.offset)
                                .live_opacity(&sweep.opacity),
                        ),
                );
            }
            if !detail.is_empty() {
                entry = entry.child(text(detail).font(p.label).color(p.muted).truncate());
            }
            list = list.child(entry);
        }
    }
    col()
        .key("agents-section")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(8.0)
        .pt(12.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], p.chrome_line)
        .child(
            row()
                .w_full()
                .center_y()
                .child(text("AGENTS").font(p.section).color(p.muted))
                .child(el().grow(1.0))
                .child(
                    text(agents.len().to_string())
                        .key("agent-count")
                        .font(p.label)
                        .color(p.muted),
                ),
        )
        .children((unread > 0).then(|| {
            text(format!(
                "{unread} new notification{}",
                if unread == 1 { "" } else { "s" }
            ))
            .key("agent-notifications")
            .role("status")
            .font(p.label)
            .color(p.accent)
        }))
        .child(list)
}

fn focus(
    state: &mut State,
    workspace_id: WorkspaceId,
    terminal_id: TerminalId,
    id: &str,
    generation: u64,
    revision: u64,
) {
    if !state.ready() || state.settings_open || state.dialog.is_some() || state.palette.is_open() {
        return;
    }
    if !state.snapshot.as_ref().is_some_and(|s| {
        s.workspaces.iter().any(|w| {
            w.id == workspace_id
                && w.terminals.iter().any(|t| {
                    t.id == terminal_id
                        && t.agents
                            .iter()
                            .any(|a| a.record.id == id && a.generation == generation)
                })
        })
    }) {
        return;
    }
    state.send(Command::AcknowledgeAgent {
        terminal_id,
        generation,
        id: id.into(),
        revision,
    });
    if state
        .snapshot
        .as_ref()
        .is_some_and(|s| s.active_workspace != workspace_id)
    {
        state.change(Command::SelectWorkspace { workspace_id });
        if state.pending.is_none() {
            return;
        }
        // Gate typing on the final ack, never the intermediate workspace view.
        if let Some(serial) = state.send(Command::SelectTerminal { terminal_id }) {
            state.pending = Some(serial);
        }
    } else {
        state.change(Command::SelectTerminal { terminal_id });
    }
}
