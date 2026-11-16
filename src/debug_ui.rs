//! Opt-in runtime diagnostics for development and support sessions.
use crate::{State, ui::Palette};
use rune::{El, Family, TextStyle, col, text};

pub fn panel(state: &State, p: &Palette) -> El {
    let mono = TextStyle::new(p.label.size, 400.0).family(Family::SystemMono);
    let snapshot = state.snapshot.as_ref();
    let workspace = state.workspace();
    let terminal_id = state.active_terminal();
    let terminal = workspace.and_then(|workspace| {
        workspace
            .terminals
            .iter()
            .find(|terminal| Some(terminal.id) == terminal_id)
    });
    let frame = terminal_id.and_then(|id| state.pane_frame(id));
    let socket = griz_protocol::socket_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|error| format!("unavailable: {error}"));
    let connection = if state.offline {
        "offline frame"
    } else if state.online {
        "connected"
    } else if state.client.is_some() {
        "connecting / disconnected"
    } else {
        "not started"
    };
    let workspace_count = snapshot.map_or(0, |snapshot| snapshot.workspaces.len());
    let terminal_count = snapshot.map_or(0, |snapshot| {
        snapshot
            .workspaces
            .iter()
            .map(|workspace| workspace.terminals.len())
            .sum()
    });
    let agent_count = snapshot.map_or(0, |snapshot| {
        snapshot
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.terminals)
            .map(|terminal| terminal.agents.len())
            .sum()
    });
    let unread_count = snapshot.map_or(0, |snapshot| {
        snapshot
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.terminals)
            .flat_map(|terminal| &terminal.agents)
            .filter(|agent| agent.unread)
            .count()
    });

    let mut contents = col()
        .key("debug-panel-content")
        .w_full()
        .gap(16.0)
        .padding(14.0)
        .child(
            col()
                .gap(3.0)
                .child(text("SESSION INSPECTOR").font(p.section).color(p.accent))
                .child(
                    text("Live metadata only; terminal contents are not included.")
                        .font(p.label)
                        .color(p.muted)
                        .wrap(),
                ),
        )
        .child(section(
            "PROCESS",
            vec![
                ("GUI PID", std::process::id().to_string()),
                (
                    "Core PID",
                    snapshot
                        .map(|snapshot| snapshot.core_pid.to_string())
                        .unwrap_or_else(|| "—".into()),
                ),
                ("Connection", connection.into()),
                ("Socket", socket),
                (
                    "Pending command",
                    state
                        .pending
                        .map(|serial| serial.to_string())
                        .unwrap_or_else(|| "none".into()),
                ),
                (
                    "Last ack",
                    snapshot
                        .map(|snapshot| snapshot.ack.to_string())
                        .unwrap_or_else(|| "—".into()),
                ),
                (
                    "Default cwd",
                    snapshot
                        .map(|snapshot| snapshot.default_cwd.clone())
                        .unwrap_or_else(|| "—".into()),
                ),
            ],
            p,
            mono,
        ))
        .child(section(
            "CATALOG",
            vec![
                ("Workspaces", workspace_count.to_string()),
                ("Terminals", terminal_count.to_string()),
                ("Agents", agent_count.to_string()),
                ("Unread", unread_count.to_string()),
            ],
            p,
            mono,
        ));

    if let Some(workspace) = workspace {
        contents = contents.child(section(
            "WORKSPACE",
            vec![
                ("ID", workspace.id.to_string()),
                ("Name", workspace.name.clone()),
                ("Directory", workspace.directory.clone()),
                ("Accent", format!("{:?}", workspace.accent).to_lowercase()),
                ("Tabs", workspace.tabs.len().to_string()),
                ("Terminals", workspace.terminals.len().to_string()),
                (
                    "Active tab",
                    state
                        .active_tab()
                        .map(|tab| tab.id.to_string())
                        .unwrap_or_else(|| "—".into()),
                ),
                (
                    "Pane count",
                    state
                        .active_tab()
                        .map(|tab| tab.layout.terminal_ids().len().to_string())
                        .unwrap_or_else(|| "0".into()),
                ),
            ],
            p,
            mono,
        ));
    }

    if let Some(terminal) = terminal {
        let unread = terminal.agents.iter().filter(|agent| agent.unread).count();
        contents = contents.child(section(
            "TERMINAL SESSION",
            vec![
                ("Terminal ID", terminal.id.to_string()),
                ("Label", terminal.label.clone()),
                ("Shell PID", terminal.shell_pid.to_string()),
                ("Generation", terminal.generation.to_string()),
                ("Initial cwd", terminal.initial_cwd.clone()),
                ("Current cwd", terminal.directory.clone()),
                (
                    "Exit status",
                    terminal.exited.clone().unwrap_or_else(|| "running".into()),
                ),
                ("Agents", terminal.agents.len().to_string()),
                ("Unread agents", unread.to_string()),
            ],
            p,
            mono,
        ));
    }

    if let Some(frame) = frame {
        let screen = &frame.screen;
        let mut modes = Vec::new();
        if screen.application_cursor {
            modes.push("application cursor");
        }
        if screen.bracketed_paste {
            modes.push("bracketed paste");
        }
        if screen.mouse_tracking {
            modes.push("mouse tracking");
        }
        if modes.is_empty() {
            modes.push("none");
        }
        let pane_size = state
            .pane_sizes
            .get(&frame.id)
            .map(|(width, height)| format!("{width:.0} × {height:.0} pt"))
            .unwrap_or_else(|| "—".into());
        let requested_grid = state
            .pane_grids
            .get(&frame.id)
            .map(|(cols, rows)| format!("{cols} × {rows}"))
            .unwrap_or_else(|| "—".into());
        let requested_cell = state
            .pane_cell_sizes
            .get(&frame.id)
            .map(|(width, height)| format!("{width} × {height} px"))
            .unwrap_or_else(|| "—".into());
        contents = contents.child(section(
            "LIVE FRAME",
            vec![
                ("Grid", format!("{} × {}", screen.cols, screen.rows)),
                ("Requested grid", requested_grid),
                ("Pane size", pane_size),
                (
                    "Cell size",
                    format!("{} × {} px", screen.cell_width, screen.cell_height),
                ),
                ("Requested cell", requested_cell),
                (
                    "Cursor",
                    format!(
                        "{},{} · {:?} · {}",
                        screen.cursor_x,
                        screen.cursor_y,
                        screen.cursor_shape,
                        if screen.cursor_visible {
                            "visible"
                        } else {
                            "hidden"
                        }
                    ),
                ),
                ("Modes", modes.join(", ")),
                ("Scrollback offset", screen.scrollback_offset.to_string()),
                ("Images", screen.images.len().to_string()),
                (
                    "Title",
                    if screen.title.is_empty() {
                        "—".into()
                    } else {
                        screen.title.clone()
                    },
                ),
                ("Device scale", format!("{:.2}×", state.device_scale.get())),
            ],
            p,
            mono,
        ));
    }

    col()
        .key("debug-panel")
        .w(292.0_f32.max(p.label.size * 19.0))
        .h_full()
        .min_h(0.0)
        .fill(p.panel)
        .border_sides([0.0, 0.0, 0.0, 1.0], p.chrome_line)
        .child(contents.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
            stick_bottom: false,
        }))
}

fn section(
    title: &'static str,
    rows: Vec<(&'static str, String)>,
    p: &Palette,
    mono: TextStyle,
) -> El {
    let mut section = col()
        .key(format!(
            "debug-section-{}",
            title.to_ascii_lowercase().replace(' ', "-")
        ))
        .w_full()
        .gap(8.0)
        .child(text(title).font(p.section).color(p.muted));
    for (label, value) in rows {
        let slug = label.to_ascii_lowercase().replace(' ', "-");
        section = section.child(
            col()
                .key(format!("debug-row-{slug}"))
                .w_full()
                .gap(2.0)
                .child(text(label).font(p.section).color(p.muted))
                .child(
                    text(value)
                        .key(format!("debug-value-{slug}"))
                        .font(mono)
                        .color(p.ink)
                        .wrap(),
                ),
        );
    }
    section
}
