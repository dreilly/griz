//! Workspace navigation and one-terminal-per-tab presentation. No session ownership here.
use crate::{Command, Dialog, State, TITLEBAR_HEIGHT, terminal_view};
use rune::tok::type_;
use rune::{Color, Cx, El, PopupItem, PopupMenu, TextStyle, col, el, oklch, row, text, text_input};

pub(crate) struct Palette {
    pub bg: Color,
    pub panel: Color,
    pub line: Color,
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
            workspaces = workspaces.child(
                col()
                    .key(format!("workspace-{id}"))
                    .role("button")
                    .label(format!("Switch to workspace {}", workspace.name))
                    .focusable(true)
                    .disabled(!state.ready())
                    .w_full()
                    .padding(12.0)
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
                    .on_secondary_press(move |s: &mut State, press, tx| {
                        if !s.ready() || s.settings_open || s.dialog.is_some() {
                            return;
                        }
                        let menu = PopupMenu::new()
                            .item(PopupItem::new("rename", "Rename workspace…"))
                            .separator()
                            .item(PopupItem::new("close", "Close workspace…"));
                        tx.popup_menu(menu, press, move |s: &mut State, picked, tx| {
                            let Some(workspace) = s.snapshot.as_ref().and_then(|snapshot| {
                                snapshot.workspaces.iter().find(|w| w.id == id)
                            }) else {
                                return;
                            };
                            let dialog = match picked {
                                Some("rename") => Dialog::RenameWorkspace(id),
                                Some("close") => Dialog::CloseWorkspace(id, workspace.name.clone()),
                                _ => return,
                            };
                            s.open_dialog(dialog, tx);
                        });
                    })
                    .child(
                        text(workspace.name.clone())
                            .font(p.label)
                            .color(if active { p.accent } else { p.ink })
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
                text("Switching or closing this window leaves your terminals running.")
                    .font(p.label)
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
            .child(text("Rune + libghostty-vt").font(p.label).color(p.muted))
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
                        p.ink.alpha(0.10)
                    } else {
                        Color::TRANSPARENT
                    })
                    .child(
                        button(format!("tab-{id}"), &label, &p)
                            .label(format!("Select terminal {number}"))
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
                            .focus_ring(2.0, 2.0, p.accent)
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
        .border(1.0, p.line)
        .focus_within_ring(1.0, -1.0, p.accent.alpha(0.65))
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
                        .child(text(">_").font(p.label).color(p.muted))
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
        .children(dialog(state, &p))
        .children(crate::settings_ui::panel(state, &p))
}

const EMPTY_WORKSPACE_ART: &str = include_str!("empty_workspace.txt");

fn empty_workspaces(state: &State, p: &Palette, cx: &Cx<State>) -> El {
    // Small, fixed-size characters make this an illustration, not UI typography.
    // Only shrink it to fit smaller panes; never apply the user's font settings.
    let [width, height] = cx.layout_size("empty-workspaces").unwrap_or([800.0, 600.0]);
    let base = TextStyle::new(12.0, 400.0)
        .family(rune::Family::SystemMono)
        .lh(11.0)
        .features("'liga' 0, 'calt' 0");
    let art_width = EMPTY_WORKSPACE_ART
        .lines()
        .map(|line| rune::text::advance(line, base))
        .fold(0.0_f32, f32::max);
    let art_height = EMPTY_WORKSPACE_ART.lines().count() as f32 * base.line_px();
    let cursor_height = 8.0;
    let cursor_width = art_height * 0.36;
    let prompt_gap = 32.0;
    let available_width = (width - 48.0).max(1.0);
    // A fixed caption allowance keeps the artwork independent of UI font size,
    // while leaving room for the normally scaling caption to wrap when needed.
    let available_height = (height - 192.0).max(1.0);
    let scale = (available_width / (art_width + prompt_gap + cursor_width))
        .min(available_height / art_height)
        .min(1.0);
    col()
        .key("empty-workspaces")
        .role("region")
        .label("No workspaces")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .padding(24.0)
        .center()
        .gap(24.0)
        .child(
            row()
                .key("empty-workspaces-icon")
                .h(art_height * scale)
                .gap(prompt_gap * scale)
                .center_y()
                .inert()
                .child(
                    text(EMPTY_WORKSPACE_ART.trim_end())
                        .key("empty-workspaces-symbol")
                        .font(TextStyle {
                            size: base.size * scale,
                            line_height: base.line_px() * scale,
                            ..base
                        })
                        .color(p.muted.alpha(0.45)),
                )
                .child(
                    el().key("empty-workspaces-cursor")
                        .size(cursor_width * scale, cursor_height * scale)
                        .self_end()
                        .fill(p.muted.alpha(0.45))
                        .cycle(rune::Cycle {
                            period: 1.6,
                            phase: 0.0,
                            stops: &[(0.0, 1.0), (0.5, 0.2), (1.0, 1.0)],
                            still: 1.0,
                            fps: 30.0,
                        }),
                ),
        )
        .child(
            col()
                .key("empty-workspaces-create")
                .role("button")
                .label("Create a workspace to get started")
                .focusable(true)
                .disabled(!state.ready())
                .max_w(available_width)
                .padding(12.0)
                .px(16.0)
                .center()
                .radius(6.0)
                .fill(p.accent.alpha(0.12))
                .border(1.0, p.accent.alpha(0.25))
                .hover(p.accent.alpha(0.18))
                .press(p.accent.alpha(0.24))
                .focus_ring(2.0, 2.0, p.accent)
                .on_press_tx(|s: &mut State, tx| {
                    s.change(Command::CreateWorkspace {
                        name: String::new(),
                    });
                    tx.blur();
                })
                .child(
                    text("Create a workspace to get started")
                        .key("empty-workspaces-message")
                        .font(p.label)
                        .color(p.ink)
                        .wrap(),
                ),
        )
}

fn dialog(state: &State, p: &Palette) -> Option<El> {
    let dialog = state.dialog.as_ref()?;
    let (title, hint, submit, placeholder) = match dialog {
        Dialog::NewWorkspace => (
            "New workspace",
            "Name is optional. Leave it blank to use the current terminal directory (e.g. ~/code). The new workspace starts there.".to_owned(),
            "Create workspace",
            "Optional name",
        ),
        Dialog::RenameWorkspace(_) => (
            "Rename workspace",
            "Leave blank to use this workspace’s selected terminal directory (e.g. ~/code). Renaming does not move its terminals.".to_owned(),
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
