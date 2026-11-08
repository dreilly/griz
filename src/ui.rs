//! Workspace navigation and one-terminal-per-tab presentation. No session ownership here.
use crate::{Command, Dialog, State, TITLEBAR_HEIGHT, terminal_view};
use rune::tok::type_;
use rune::{Color, Cx, El, TextStyle, col, el, oklch, row, text, text_input};

struct Palette {
    bg: Color,
    panel: Color,
    line: Color,
    ink: Color,
    muted: Color,
    accent: Color,
}

fn palette() -> Palette {
    Palette {
        bg: oklch(0.17, 0.005, 260.0),
        panel: oklch(0.20, 0.005, 260.0),
        line: oklch(0.29, 0.005, 260.0),
        ink: oklch(0.91, 0.005, 260.0),
        muted: oklch(0.64, 0.01, 260.0),
        accent: oklch(0.78, 0.10, 155.0),
    }
}

fn button(key: impl Into<String>, label: &str, p: &Palette) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .focusable(true)
        .h(28.0)
        .px(10.0)
        .center_y()
        .radius(5.0)
        .hover(p.ink.alpha(0.08))
        .press(p.ink.alpha(0.12))
        .focus_ring(2.0, 2.0, p.accent)
        .child(text(label).font(type_::LABEL).color(p.ink))
}

pub fn view(state: &State, cx: &mut Cx<State>) -> El {
    let p = palette();
    let insets = cx.chrome_insets();
    cx.caption_color(p.ink);
    let titlebar = row()
        .key("titlebar")
        .h(TITLEBAR_HEIGHT)
        .w_full()
        .pl(insets.leading + 16.0)
        .pr(insets.trailing + 16.0)
        .center_y()
        .gap(12.0)
        .fill(p.panel)
        .window_drag()
        .child(text("griz").font(type_::TITLE).color(p.ink))
        .child(el().grow(1.0))
        .child(
            button("toggle-sidebar", "Sidebar", &p).on_press_tx(|s: &mut State, tx| {
                s.sidebar_open = !s.sidebar_open;
                tx.blur();
            }),
        );

    let mut workspaces = col()
        .key("workspace-list")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(6.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        });
    if let Some(snapshot) = &state.snapshot {
        for workspace in &snapshot.workspaces {
            let id = workspace.id;
            let active = snapshot.active_workspace == id;
            let running = workspace
                .terminals
                .iter()
                .filter(|t| t.exited.is_none())
                .count();
            workspaces = workspaces.child(
                col()
                    .key(format!("workspace-{id}"))
                    .role("button")
                    .label(format!("Switch to workspace {}", workspace.name))
                    .focusable(true)
                    .disabled(!state.ready())
                    .w_full()
                    .padding(12.0)
                    .gap(5.0)
                    .radius(6.0)
                    .fill(if active {
                        p.accent.alpha(0.12)
                    } else {
                        Color::TRANSPARENT
                    })
                    .hover(p.ink.alpha(0.06))
                    .focus_ring(2.0, 2.0, p.accent)
                    .on_press_tx(move |s: &mut State, tx| {
                        s.change(Command::SelectWorkspace { workspace_id: id });
                        tx.blur();
                    })
                    .child(
                        text(workspace.name.clone())
                            .font(type_::LABEL)
                            .color(if active { p.accent } else { p.ink })
                            .truncate(),
                    )
                    .child(
                        text(format!(
                            "{} tabs · {running} running",
                            workspace.terminals.len()
                        ))
                        .font(type_::LABEL)
                        .color(p.muted),
                    ),
            );
        }
    }
    let sidebar = state.sidebar_open.then(|| {
        col()
            .key("sidebar")
            .w(210.0)
            .h_full()
            .padding(12.0)
            .gap(12.0)
            .fill(p.panel)
            .child(
                row()
                    .w_full()
                    .px(4.0)
                    .center_y()
                    .child(text("WORKSPACES").font(type_::SECTION).color(p.muted))
                    .child(el().grow(1.0))
                    .child(
                        button("new-workspace", "+", &p)
                            .label("New workspace")
                            .disabled(!state.ready())
                            .on_press_tx(|s: &mut State, tx| {
                                s.open_dialog(Dialog::NewWorkspace, tx)
                            }),
                    ),
            )
            .child(workspaces)
            .child(
                text("Switching or closing this window leaves your terminals running.")
                    .font(type_::LABEL)
                    .color(p.muted)
                    .wrap(),
            )
            .child(
                button("reconnect", "Reconnect", &p)
                    .disabled(state.offline)
                    .on_press_tx(|s: &mut State, tx| {
                        s.connect();
                        tx.blur();
                    }),
            )
            .child(
                text("Rune + libghostty-vt")
                    .font(type_::LABEL)
                    .color(p.muted),
            )
    });

    let workspace_name = state
        .workspace()
        .map(|w| w.name.as_str())
        .unwrap_or("Workspace");
    let mut tabs = row()
        .key("terminal-tabs")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .gap(4.0)
        .center_y()
        .scroll_x();
    if let Some(workspace) = state.workspace() {
        for terminal in &workspace.terminals {
            let id = terminal.id;
            let active = state.active_terminal() == Some(id);
            let label = format!(
                "{}{}",
                terminal.label,
                if terminal.exited.is_some() {
                    " · exited"
                } else {
                    ""
                }
            );
            tabs = tabs.child(
                button(format!("tab-{id}"), &label, &p)
                    .disabled(!state.ready())
                    .fill(if active {
                        p.ink.alpha(0.10)
                    } else {
                        Color::TRANSPARENT
                    })
                    .on_press_tx(move |s: &mut State, tx| {
                        s.change(Command::SelectTerminal { terminal_id: id });
                        tx.blur();
                    }),
            );
        }
    }
    let terminal = col()
        .key("terminal-pane")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .child(
            row()
                .key("workspace-header")
                .h(38.0)
                .w_full()
                .px(12.0)
                .center_y()
                .gap(8.0)
                .child(
                    text(workspace_name)
                        .font(type_::LABEL)
                        .color(p.muted)
                        .truncate(),
                )
                .child(
                    button("rename-workspace", "Rename", &p)
                        .disabled(!state.ready() || state.workspace().is_none())
                        .on_press_tx(|s: &mut State, tx| {
                            if let Some(w) = s.workspace() {
                                s.open_dialog(Dialog::RenameWorkspace(w.id), tx);
                            }
                        }),
                )
                .child(el().grow(1.0))
                .children(state.shell_exited().then(|| {
                    button("start-shell", "Start new shell", &p)
                        .disabled(!state.ready())
                        .fill(p.accent.alpha(0.15))
                        .on_press_tx(|s: &mut State, tx| {
                            if s.shell_exited()
                                && let Some(terminal_id) = s.active_terminal()
                            {
                                s.change(Command::StartShell { terminal_id });
                            }
                            tx.blur();
                        })
                }))
                .children(state.active_terminal().map(|terminal_id| {
                    let label = state
                        .workspace()
                        .and_then(|w| w.terminals.iter().find(|t| t.id == terminal_id))
                        .map(|t| t.label.clone())
                        .unwrap_or_default();
                    button("close-terminal", "Close tab", &p)
                        .disabled(!state.ready())
                        .on_press_tx(move |s: &mut State, tx| {
                            s.open_dialog(Dialog::CloseTerminal(terminal_id, label.clone()), tx)
                        })
                })),
        )
        .child(
            row()
                .h(38.0)
                .w_full()
                .px(12.0)
                .gap(8.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], p.line)
                .child(tabs)
                .child(
                    button("new-terminal", "+ Terminal", &p)
                        .disabled(!state.ready() || state.workspace().is_none())
                        .on_press_tx(|s: &mut State, tx| {
                            if let Some(w) = s.workspace() {
                                s.open_dialog(Dialog::NewTerminal(w.id), tx);
                            }
                        }),
                ),
        )
        .child(terminal_view::viewport(state))
        .child(
            row()
                .key("terminal-status")
                .h(30.0)
                .w_full()
                .px(12.0)
                .center_y()
                .border_sides([1.0, 0.0, 0.0, 0.0], p.line)
                .child(
                    text(
                        state
                            .notice
                            .as_deref()
                            .unwrap_or(if state.pending.is_some() {
                                "Updating selection…"
                            } else {
                                &state.status
                            }),
                    )
                    .font(type_::LABEL)
                    .color(if state.notice.is_some() {
                        p.accent
                    } else {
                        p.muted
                    })
                    .truncate(),
                ),
        );

    col()
        .key("root")
        .w_full()
        .h_full()
        .fill(p.bg)
        .on_paste(|s: &mut State, text, tx| {
            if s.dialog.is_none() && tx.focused().is_none_or(|key| key == "terminal") {
                let bracketed = s.screen().is_some_and(|screen| screen.bracketed_paste);
                s.input(crate::input::paste(text, bracketed));
            }
        })
        .child(titlebar)
        .child(
            row()
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .children(sidebar)
                .child(terminal),
        )
        .children(dialog(state, &p))
}

fn dialog(state: &State, p: &Palette) -> Option<El> {
    let dialog = state.dialog.as_ref()?;
    let (title, hint, submit, placeholder) = match dialog {
        Dialog::NewWorkspace => (
            "New workspace",
            "A named collection of terminals, not a project directory.".to_owned(),
            "Create workspace",
            "e.g. Daily",
        ),
        Dialog::RenameWorkspace(_) => (
            "Rename workspace",
            "This changes the name, not the terminals or their directories.".to_owned(),
            "Rename",
            "Workspace name",
        ),
        Dialog::NewTerminal(_) => (
            "New terminal",
            format!(
                "Optional absolute starting directory. Blank uses {}. Each tab can use a different directory.",
                state
                    .snapshot
                    .as_ref()
                    .map(|s| s.default_cwd.as_str())
                    .unwrap_or("the service default")
            ),
            "Open terminal",
            "/Users/you/code/anything",
        ),
        Dialog::CloseTerminal(_, label) => (
            "Close terminal?",
            format!(
                "Close {label} and end its shell and foreground job? This is different from switching tabs or closing the window."
            ),
            "Close terminal",
            "",
        ),
    };
    let input = (!matches!(dialog, Dialog::CloseTerminal(..))).then(|| {
        text_input(&state.draft, TextStyle::new(14.0, 400.0))
            .key("dialog-input")
            .label(if matches!(dialog, Dialog::NewTerminal(_)) {
                "Starting directory"
            } else {
                "Workspace name"
            })
            .single_line()
            .w_full()
            .padding(10.0)
            .radius(5.0)
            .fill(p.ink.alpha(0.06))
            .color(p.ink)
            .placeholder(placeholder, p.muted)
            .on_edit(|s: &mut State| &mut s.draft)
            .on_submit(|s: &mut State, tx| s.submit_dialog(tx))
    });
    let valid = match dialog {
        Dialog::NewWorkspace | Dialog::RenameWorkspace(_) => {
            let name = state.draft.text.trim();
            !name.is_empty() && name.chars().count() <= 80 && !name.chars().any(char::is_control)
        }
        _ => true,
    };
    Some(
        el().key("dialog-scrim")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .fill(Color::BLACK.alpha(0.55))
            .center()
            .child(
                col()
                    .key("dialog")
                    .w(410.0)
                    .padding(22.0)
                    .gap(16.0)
                    .radius(10.0)
                    .fill(p.panel)
                    .role("dialog")
                    .label(title)
                    .modal(true, |s: &mut State| s.dialog = None)
                    .child(text(title).font(type_::TITLE).color(p.ink))
                    .child(text(hint).font(type_::LABEL).color(p.muted).wrap())
                    .children(input)
                    .child(
                        row()
                            .w_full()
                            .gap(8.0)
                            .child(el().grow(1.0))
                            .child(button("dialog-cancel", "Cancel", p).on_press_tx(
                                |s: &mut State, tx| {
                                    s.dialog = None;
                                    tx.blur();
                                },
                            ))
                            .child(
                                button("dialog-submit", submit, p)
                                    .disabled(!valid || !state.ready())
                                    .fill(p.accent.alpha(0.15))
                                    .on_press_tx(|s: &mut State, tx| s.submit_dialog(tx)),
                            ),
                    ),
            ),
    )
}
