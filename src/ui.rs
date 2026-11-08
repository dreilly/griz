//! Workspace navigation and one-terminal-per-tab presentation. No session ownership here.
use crate::{Command, Dialog, State, TITLEBAR_HEIGHT, terminal_view};
use rune::tok::type_;
use rune::{Color, Cx, El, TextStyle, col, el, oklch, row, text, text_input};

pub(crate) struct Palette {
    pub bg: Color,
    pub panel: Color,
    pub line: Color,
    pub chrome_line: Color,
    pub ink: Color,
    pub muted: Color,
    pub accent: Color,
    pub label: TextStyle,
    pub title: TextStyle,
    pub section: TextStyle,
    pub input: TextStyle,
}

impl Palette {
    pub fn control_height(&self) -> f32 {
        28.0_f32.max(self.label.line_px() + 10.0)
    }
}

fn palette(settings: &crate::settings::Settings) -> Palette {
    let midnight = settings.theme == crate::settings::Theme::Midnight;
    Palette {
        bg: terminal_view::color(if midnight { [12, 17, 29] } else { [17, 17, 20] }),
        panel: terminal_view::color(if midnight { [17, 24, 39] } else { [17, 17, 20] }),
        line: terminal_view::color(if midnight { [42, 54, 75] } else { [44, 46, 49] }),
        chrome_line: oklch(0.36, 0.025, 255.0),
        ink: oklch(0.91, 0.005, 260.0),
        muted: oklch(0.64, 0.01, 260.0),
        accent: oklch(0.78, 0.10, 155.0),
        label: settings.ui_font(type_::LABEL),
        title: settings.ui_font(type_::TITLE),
        section: settings.ui_font(type_::SECTION),
        input: settings.ui_font(TextStyle::new(14.0, 400.0)),
    }
}

pub(crate) fn button(key: impl Into<String>, label: &str, p: &Palette) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .focusable(true)
        .h(p.control_height())
        .px(10.0)
        .center_y()
        .radius(5.0)
        .hover(p.ink.alpha(0.08))
        .press(p.ink.alpha(0.12))
        .focus_ring(2.0, 2.0, p.accent)
        .child(text(label).font(p.label).color(p.ink))
}

pub fn view(state: &State, cx: &mut Cx<State>) -> El {
    let p = palette(&state.settings);
    let workspace_accent = state
        .workspace()
        .map(|workspace| crate::workspace_ui::accent_color(workspace.accent))
        .unwrap_or(p.accent);
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
        .border_sides([0.0, 0.0, 1.0, 0.0], p.chrome_line)
        .window_drag()
        .child(text("griz").font(p.title).color(p.ink))
        .child(el().grow(1.0))
        .child(
            button("toggle-sidebar", "Sidebar", &p).on_press_tx(|s: &mut State, tx| {
                s.act(crate::settings::Action::ToggleSidebar, tx);
            }),
        )
        .child(button("open-settings", "Settings…", &p).on_press_tx(State::open_settings));

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
            let accent = crate::workspace_ui::accent_color(workspace.accent);
            workspaces = workspaces.child(
                col()
                    .key(format!("workspace-{id}"))
                    .role("button")
                    .label(format!("Switch to workspace {}", workspace.name))
                    .help(workspace.directory.clone())
                    .focusable(true)
                    .disabled(!state.ready())
                    .w_full()
                    .padding(12.0)
                    .radius(6.0)
                    .fill(if active {
                        accent.alpha(0.12)
                    } else {
                        Color::TRANSPARENT
                    })
                    .hover(p.ink.alpha(0.06))
                    .focus_ring(2.0, 2.0, accent)
                    .on_press_tx(move |s: &mut State, tx| {
                        s.change(Command::SelectWorkspace { workspace_id: id });
                        tx.blur();
                    })
                    .on_secondary_press(move |s: &mut State, press, tx| {
                        if !s.ready() || s.settings_open || s.dialog.is_some() {
                            return;
                        }
                        s.open_dialog(Dialog::WorkspaceMenu(id, (press.x, press.y)), tx);
                    })
                    .child(
                        text(workspace.name.clone())
                            .font(p.label)
                            .color(if active { accent } else { p.ink })
                            .truncate(),
                    ),
            );
        }
    }
    let sidebar = state.sidebar_open.then(|| {
        col()
            .key("sidebar")
            .w(210.0_f32.max(p.label.size * 14.0))
            .h_full()
            .padding(12.0)
            .gap(12.0)
            .fill(p.panel)
            .border_sides([0.0, 1.0, 0.0, 0.0], p.chrome_line)
            .child(
                row()
                    .w_full()
                    .px(4.0)
                    .center_y()
                    .child(text("WORKSPACES").font(p.section).color(p.muted))
                    .child(el().grow(1.0))
                    .child(
                        button("new-workspace", "+", &p)
                            .label("New workspace")
                            .disabled(!state.ready())
                            .on_press_tx(|s: &mut State, tx| {
                                s.act(crate::settings::Action::NewWorkspace, tx)
                            }),
                    ),
            )
            .child(workspaces)
            .child(
                button("reconnect", "Reconnect", &p)
                    .disabled(state.offline)
                    .on_press_tx(|s: &mut State, tx| {
                        s.connect();
                        tx.blur();
                    }),
            )
    });

    let mut tabs = row().key("terminal-tab-items").h_full().gap(4.0).center_y();
    if let Some(workspace) = state.workspace() {
        for (index, terminal) in workspace.terminals.iter().enumerate() {
            let number = index + 1;
            let id = terminal.id;
            let active = state.active_terminal() == Some(id);
            let label = format!(
                "{}{}",
                number,
                if terminal.exited.is_some() {
                    " · exited"
                } else {
                    ""
                }
            );
            let close_label = format!("terminal {number}");
            tabs = tabs.child(
                row()
                    .key(format!("tab-container-{id}"))
                    .min_w(80.0)
                    .h(p.control_height())
                    .pr(4.0)
                    .center_y()
                    .radius(5.0)
                    .fill(if active {
                        workspace_accent.alpha(0.15)
                    } else {
                        Color::TRANSPARENT
                    })
                    .child(
                        button(format!("tab-{id}"), &label, &p)
                            .label(format!("Select terminal {number}"))
                            .focus_ring(2.0, 2.0, workspace_accent)
                            .grow(1.0)
                            .disabled(!state.ready())
                            .on_press_tx(move |s: &mut State, tx| {
                                s.change(Command::SelectTerminal { terminal_id: id });
                                tx.blur();
                            }),
                    )
                    .child(
                        row()
                            .key(format!("close-terminal-{id}"))
                            .role("button")
                            .label(format!("Close terminal {number}"))
                            .focusable(true)
                            .w(20.0_f32.max(p.label.line_px() + 4.0))
                            .h(20.0_f32.max(p.label.line_px() + 4.0))
                            .center()
                            .radius(3.0)
                            .focus_ring(2.0, 2.0, workspace_accent)
                            .child(text("×").font(p.label).color(p.muted))
                            .disabled(!state.ready())
                            .on_press_tx(move |s: &mut State, tx| {
                                s.open_dialog(Dialog::CloseTerminal(id, close_label.clone()), tx);
                            }),
                    ),
            );
        }
    }
    // A scroll viewport stretches its sole content child to its width. Keep
    // that child a row, even with one tab, so the tab itself stays content-sized.
    let tabs = row()
        .key("terminal-tabs")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .scroll_x()
        .child(tabs);
    let terminal_label = state
        .workspace()
        .and_then(|w| {
            w.terminals
                .iter()
                .position(|t| Some(t.id) == state.active_terminal())
        })
        .map(|index| (index + 1).to_string())
        .unwrap_or_else(|| "Terminal".into());
    let terminal_card = col()
        .key("terminal-card")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .min_w(0.0)
        .padding(1.0)
        .radius(11.0)
        .fill(terminal_view::background_for(state.settings.theme))
        .border(1.0, workspace_accent.alpha(0.55))
        .focus_within_ring(1.0, -1.0, workspace_accent.alpha(0.85))
        .child(
            // Clip content to the inside of the frame, not the outer edge.
            // Otherwise opaque viewport corners paint over the curved border.
            col()
                .key("terminal-content")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .min_w(0.0)
                .radius(10.0)
                .clip_rounded(10.0)
                .child(
                    row()
                        .key("terminal-header")
                        .h(34.0_f32.max(p.label.line_px() + 12.0))
                        .w_full()
                        .px(12.0)
                        .gap(10.0)
                        .center_y()
                        .border_sides([0.0, 0.0, 1.0, 0.0], p.line)
                        .child(
                            text(">_")
                                .key("terminal-header-symbol")
                                .font(p.label)
                                .color(workspace_accent),
                        )
                        .child(text(terminal_label).font(p.label).color(p.ink).truncate())
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
                        })),
                )
                .child(terminal_view::viewport(state)),
        );
    let terminal = if state
        .snapshot
        .as_ref()
        .is_some_and(|s| s.workspaces.is_empty())
    {
        empty_workspaces(state, &p, cx)
    } else {
        col()
            .key("terminal-pane")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .child(
                row()
                    .h(38.0_f32.max(p.control_height() + 10.0))
                    .w_full()
                    .px(12.0)
                    .gap(8.0)
                    .center_y()
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
            .child(
                col()
                    .key("terminal-stage")
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .min_w(0.0)
                    .padding(10.0)
                    .child(terminal_card),
            )
            .child(
                row()
                    .key("terminal-status")
                    .h(30.0_f32.max(p.label.line_px() + 10.0))
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
                        .font(p.label)
                        .color(if state.notice.is_some() {
                            p.accent
                        } else {
                            p.muted
                        })
                        .truncate(),
                    ),
            )
    };

    col()
        .key("root")
        .w_full()
        .h_full()
        .fill(p.bg)
        .on_paste(|s: &mut State, text, tx| {
            if !s.settings_open
                && s.dialog.is_none()
                && tx.focused().is_none_or(|key| key == "terminal")
            {
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
        .children(dialog(state, &p, cx))
        .children(crate::settings_ui::panel(state, &p))
}

/// Shared by the real action and Settings samples so previews stay exact.
pub(crate) fn workspace_button(
    key: impl Into<String>,
    style: crate::settings::WorkspaceButtonStyle,
    p: &Palette,
    label_key: impl Into<String>,
) -> El {
    use crate::settings::WorkspaceButtonStyle;
    let (fill, border, hover, press, ink, radius) = match style {
        WorkspaceButtonStyle::Soft => (
            p.ink.alpha(0.08),
            p.ink.alpha(0.08),
            p.ink.alpha(0.12),
            p.ink.alpha(0.18),
            p.ink,
            6.0,
        ),
        WorkspaceButtonStyle::Outline => (
            Color::TRANSPARENT,
            p.ink.alpha(0.35),
            p.ink.alpha(0.05),
            p.ink.alpha(0.10),
            p.ink,
            6.0,
        ),
        WorkspaceButtonStyle::Solid => (
            p.ink,
            Color::TRANSPARENT,
            p.ink.alpha(0.90),
            p.ink.alpha(0.75),
            p.bg,
            6.0,
        ),
        WorkspaceButtonStyle::Ghost => (
            Color::TRANSPARENT,
            Color::TRANSPARENT,
            p.ink.alpha(0.06),
            p.ink.alpha(0.12),
            p.ink,
            6.0,
        ),
        WorkspaceButtonStyle::Pill => (
            p.ink.alpha(0.08),
            p.ink.alpha(0.18),
            p.ink.alpha(0.12),
            p.ink.alpha(0.18),
            p.ink,
            999.0,
        ),
        WorkspaceButtonStyle::Accent => (
            p.accent.alpha(0.12),
            p.accent.alpha(0.25),
            p.accent.alpha(0.18),
            p.accent.alpha(0.24),
            p.ink,
            6.0,
        ),
    };
    col()
        .key(key)
        .role("button")
        .label("Create a workspace to get started")
        .focusable(true)
        .padding(12.0)
        .px(16.0)
        .center()
        .radius(radius)
        .fill(fill)
        .border(1.0, border)
        .hover(hover)
        .press(press)
        .focus_ring(2.0, 2.0, p.ink.alpha(0.65))
        .child(
            text("Create a workspace to get started")
                .key(label_key)
                .font(p.label)
                .color(ink)
                .wrap(),
        )
}

fn empty_workspaces(state: &State, p: &Palette, cx: &Cx<State>) -> El {
    let [width, _] = cx.layout_size("empty-workspaces").unwrap_or([800.0, 600.0]);
    let available_width = (width - 48.0).max(1.0);
    col()
        .key("empty-workspaces")
        .role("region")
        .label("No workspaces")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .padding(24.0)
        .center()
        .child(
            workspace_button(
                "empty-workspaces-create",
                state.settings.workspace_button_style,
                p,
                "empty-workspaces-message",
            )
            .disabled(!state.ready())
            .max_w(available_width)
            .on_press_tx(|s: &mut State, tx| {
                s.change(Command::CreateWorkspace {
                    name: String::new(),
                });
                tx.blur();
            }),
        )
}

fn dialog(state: &State, p: &Palette, cx: &Cx<State>) -> Option<El> {
    let dialog = state.dialog.as_ref()?;
    let (title, hint, submit, placeholder) = match dialog {
        Dialog::WorkspaceMenu(id, position) => {
            return crate::workspace_ui::menu(state, *id, *position, p, cx);
        }
        Dialog::NewWorkspace => (
            "New workspace",
            "Name is optional. Leave it blank to follow the selected terminal’s current directory (e.g. griz). Hover the workspace to see its full path. The new workspace starts in the current directory.".to_owned(),
            "Create workspace",
            "Optional name",
        ),
        Dialog::RenameWorkspace(_) => (
            "Rename workspace",
            "Leave blank to follow this workspace’s selected terminal directory automatically. The label updates as you cd or switch tabs; hover it to see the full path. A custom name stays fixed. Renaming does not move its terminals.".to_owned(),
            "Rename",
            "Optional name",
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
        Dialog::CloseWorkspace(_, name) => (
            "Close workspace?",
            format!(
                "Close “{name}” and end all of its terminals, shells, and foreground jobs? This removes the workspace for every connected window."
            ),
            "Close workspace",
            "",
        ),
        Dialog::CloseTerminal(_, label) => (
            "Close terminal?",
            format!(
                "Close {label} and end its shell and foreground job? Closing the final tab also closes its workspace for every connected window. Switching tabs or closing the window leaves terminals running."
            ),
            "Close terminal",
            "",
        ),
    };
    let input = (!matches!(
        dialog,
        Dialog::CloseTerminal(..) | Dialog::CloseWorkspace(..)
    ))
    .then(|| {
        text_input(&state.draft, p.input)
            .key("dialog-input")
            .label(if matches!(dialog, Dialog::NewTerminal(_)) {
                "Starting directory"
            } else {
                "Workspace name (optional)"
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
            name.chars().count() <= 80 && !name.chars().any(char::is_control)
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
                    .child(text(title).font(p.title).color(p.ink))
                    .child(text(hint).font(p.label).color(p.muted).wrap())
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
