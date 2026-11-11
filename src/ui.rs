//! Workspace navigation and terminal tabs. No session ownership here.
use crate::{Command, Dialog, State, TITLEBAR_HEIGHT, terminal_view};
use rune::tok::type_;
use rune::{Color, Cx, El, Icon, TextStyle, col, el, icon, oklch, row, text, text_input};

static SIDEBAR_ICON: Icon = Icon {
    name: "griz-sidebar",
    view: 16.0,
    body: r#"<rect x="2" y="2.5" width="12" height="11" rx="2" stroke="currentColor" stroke-width="1" fill="none"/><path d="M6 3V13" stroke="currentColor" stroke-width="1"/>"#,
};

static SEARCH_ICON: Icon = Icon {
    name: "griz-search",
    view: 16.0,
    body: r#"<circle cx="6.75" cy="6.75" r="4.25" stroke="currentColor" stroke-width="1.25" fill="none"/><path d="M10 10L13.5 13.5" stroke="currentColor" stroke-width="1.25" stroke-linecap="round"/>"#,
};

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

    pub fn button_fill(&self) -> Color {
        self.ink.alpha(0.08)
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

/// Regular actions use the same Soft surface as the button-style sample.
pub(crate) fn button(key: impl Into<String>, label: &str, p: &Palette) -> El {
    styled_button(key, label, p, crate::settings::WorkspaceButtonStyle::Soft)
}

fn styled_button(
    key: impl Into<String>,
    label: &str,
    p: &Palette,
    style: crate::settings::WorkspaceButtonStyle,
) -> El {
    styled_button_with_leading(key, label, p, style, [])
}

fn styled_button_with_leading(
    key: impl Into<String>,
    label: &str,
    p: &Palette,
    style: crate::settings::WorkspaceButtonStyle,
    leading: impl IntoIterator<Item = El>,
) -> El {
    button_surface(
        row()
            .key(key)
            .role("button")
            .label(label)
            .focusable(true)
            .h(p.control_height())
            .px(10.0)
            .gap(6.0)
            .center_y()
            .children(leading)
            .child(text(label).font(p.label).color(button_ink(style, p))),
        style,
        p,
    )
}

pub(crate) fn workspace_accent(state: &State) -> Color {
    state
        .workspace()
        .map(|w| crate::workspace_ui::accent_color(w.accent))
        .unwrap_or(oklch(0.78, 0.10, 155.0))
}

fn sidebar_toggle(state: &State, p: &Palette) -> El {
    button_surface(
        row()
            .key("toggle-sidebar")
            .role("button")
            .label("Toggle sidebar")
            .help(if state.sidebar_open {
                "Hide sidebar"
            } else {
                "Show sidebar"
            })
            .focusable(true)
            .size(p.control_height(), p.control_height())
            .center()
            .child(
                icon(&SIDEBAR_ICON, 16.0)
                    .key("sidebar-icon")
                    .color(if state.sidebar_open { p.ink } else { p.muted }),
            ),
        crate::settings::WorkspaceButtonStyle::Soft,
        p,
    )
    .on_press_tx(|s: &mut State, tx| {
        s.act(crate::settings::Action::ToggleSidebar, tx);
    })
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
            row()
                .key("quick-open")
                .role("button")
                .label("Search workspaces and terminals")
                .help("Open quick search")
                .focusable(true)
                .no_window_drag()
                .grow(3.0)
                .min_w(0.0)
                .max_w(560.0)
                .h(p.control_height())
                .px(10.0)
                .gap(8.0)
                .center_y()
                .radius(6.0)
                .fill(p.ink.alpha(0.04))
                .border(1.0, p.line)
                .hover(p.ink.alpha(0.08))
                .focus_ring(2.0, 2.0, p.accent)
                .child(icon(&SEARCH_ICON, 16.0).key("search-icon").color(p.muted))
                .child(
                    text("Search workspaces and terminals…")
                        .font(p.label)
                        .color(p.muted)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .children(
                    state
                        .settings
                        .chord(crate::settings::Action::QuickOpen)
                        .map(|chord| {
                            text(chord.label())
                                .key("quick-open-shortcut")
                                .font(p.label)
                                .color(p.muted)
                        }),
                )
                .on_press_tx(|s: &mut State, tx| {
                    s.act(crate::settings::Action::QuickOpen, tx);
                }),
        )
        .child(el().grow(1.0));

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
                            .color(accent)
                            .truncate(),
                    ),
            );
        }
    }
    let sidebar = if state.sidebar_open {
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
                    .key("workspace-controls")
                    .w_full()
                    .px(4.0)
                    .gap(4.0)
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
                    )
                    .child(sidebar_toggle(state, &p)),
            )
            .child(workspaces)
            .child(crate::agents_ui::section(state, &p))
    } else {
        col()
            .key("sidebar-rail")
            .w(p.control_height() + 24.0)
            .h_full()
            .padding(12.0)
            .fill(p.panel)
            .border_sides([0.0, 1.0, 0.0, 0.0], p.chrome_line)
            .child(sidebar_toggle(state, &p))
    };

    let mut tabs = row().key("terminal-tab-items").h_full().gap(4.0).center_y();
    if let Some(workspace) = state.workspace() {
        for (index, tab) in workspace.tabs.iter().enumerate() {
            let number = index + 1;
            let id = tab.id;
            let active = state
                .active_terminal()
                .is_some_and(|id| tab.layout.contains(id));
            let target = state
                .active_terminal()
                .filter(|_| active)
                .unwrap_or_else(|| tab.layout.terminal_ids()[0]);
            let terminal = workspace.terminals.iter().find(|t| t.id == target);
            let pane_count = tab.layout.terminal_ids().len();
            let label = format!(
                "{}{}",
                number,
                if terminal.is_some_and(|t| t.exited.is_some()) {
                    " · exited".to_owned()
                } else if pane_count > 1 {
                    format!(" · {pane_count} panes")
                } else {
                    String::new()
                }
            );
            let terminal_number = workspace
                .terminals
                .iter()
                .position(|t| t.id == target)
                .map(|i| i + 1)
                .unwrap_or(number);
            let harnesses = crate::harness_icons::for_tab(workspace, tab);
            let harness_names = harnesses
                .iter()
                .map(|h| h.name())
                .collect::<Vec<_>>()
                .join(", ");
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
                        styled_button_with_leading(
                            format!("tab-{id}"),
                            &label,
                            &p,
                            crate::settings::WorkspaceButtonStyle::Ghost,
                            harnesses
                                .iter()
                                .map(|h| h.badge(format!("tab-{id}-harness-{}", h.key()), &p)),
                        )
                        .label(if harness_names.is_empty() {
                            format!("Select terminal {number}")
                        } else {
                            format!("Select terminal {number} · {harness_names}")
                        })
                        .focus_ring(2.0, 2.0, workspace_accent)
                        .grow(1.0)
                        .disabled(!state.ready())
                        .on_press_tx(move |s: &mut State, tx| {
                            s.change(Command::SelectTerminal {
                                terminal_id: target,
                            });
                            tx.blur();
                        }),
                    )
                    .child(
                        row()
                            .key(format!("close-terminal-{id}"))
                            .role("button")
                            .label(format!("Close terminal {terminal_number}"))
                            .focusable(true)
                            .w(20.0_f32.max(p.label.line_px() + 4.0))
                            .h(20.0_f32.max(p.label.line_px() + 4.0))
                            .center()
                            .radius(3.0)
                            .focus_ring(2.0, 2.0, workspace_accent)
                            .child(text("×").font(p.label).color(p.muted))
                            .disabled(!state.ready())
                            .on_press_tx(move |s: &mut State, tx| {
                                s.request_close_terminal(target, tx);
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
    let terminal_card = crate::panes::stage(state, &p);
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
                    .child(tabs),
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
                && !s.palette.is_open()
                && crate::terminal_focus(s, tx)
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
                .child(sidebar)
                .child(terminal),
        )
        .children(state.terminal_mouse.copied_until.map(|_| {
            row()
                .key("clipboard-toast")
                .role("status")
                .label("Copied to clipboard")
                .live_region(rune::LiveRegion::Polite)
                .px(14.0)
                .py(9.0)
                .gap(8.0)
                .center_y()
                .fill(p.panel)
                .border(1.0, p.line)
                .radius(8.0)
                .anchored(
                    "terminal-status",
                    rune::Side::Top,
                    rune::Align::Center,
                    12.0,
                )
                .inert()
                .child(text("✓").font(p.label).color(p.accent))
                .child(text("Copied to clipboard").font(p.label).color(p.ink))
        }))
        .children(dialog(state, &p, cx))
        .children(crate::settings_ui::panel(state, &p))
        .children(crate::command_palette::panel(state, &p, cx))
}

fn button_ink(style: crate::settings::WorkspaceButtonStyle, p: &Palette) -> Color {
    if style == crate::settings::WorkspaceButtonStyle::Solid {
        p.bg
    } else {
        p.ink
    }
}

/// Theme-relative colors keep Soft neutral on both charcoal and midnight surfaces.
/// Regular controls, the empty-state action, and gallery samples share every interaction state.
fn button_surface(element: El, style: crate::settings::WorkspaceButtonStyle, p: &Palette) -> El {
    use crate::settings::WorkspaceButtonStyle;
    let (fill, border, hover, press, radius) = match style {
        WorkspaceButtonStyle::Soft => (
            p.button_fill(),
            p.button_fill(),
            p.ink.alpha(0.12),
            p.ink.alpha(0.18),
            6.0,
        ),
        WorkspaceButtonStyle::Outline => (
            Color::TRANSPARENT,
            p.ink.alpha(0.35),
            p.ink.alpha(0.05),
            p.ink.alpha(0.10),
            6.0,
        ),
        WorkspaceButtonStyle::Solid => (
            p.ink,
            Color::TRANSPARENT,
            p.ink.alpha(0.90),
            p.ink.alpha(0.75),
            6.0,
        ),
        WorkspaceButtonStyle::Ghost => (
            Color::TRANSPARENT,
            Color::TRANSPARENT,
            p.ink.alpha(0.06),
            p.ink.alpha(0.12),
            6.0,
        ),
        WorkspaceButtonStyle::Pill => (
            p.ink.alpha(0.08),
            p.ink.alpha(0.18),
            p.ink.alpha(0.12),
            p.ink.alpha(0.18),
            999.0,
        ),
        WorkspaceButtonStyle::Accent => (
            p.accent.alpha(0.12),
            p.accent.alpha(0.25),
            p.accent.alpha(0.18),
            p.accent.alpha(0.24),
            6.0,
        ),
    };
    element
        .radius(radius)
        .fill(fill)
        .border(1.0, border)
        .hover(hover)
        .press(press)
        .focus_ring(2.0, 2.0, p.ink.alpha(0.65))
}

/// Shared by the real action and Settings samples so previews stay exact.
pub(crate) fn workspace_button(
    key: impl Into<String>,
    style: crate::settings::WorkspaceButtonStyle,
    p: &Palette,
    label_key: impl Into<String>,
) -> El {
    button_surface(
        col()
            .key(key)
            .role("button")
            .label("Create a workspace to get started")
            .focusable(true)
            .padding(12.0)
            .px(16.0)
            .center()
            .child(
                text("Create a workspace to get started")
                    .key(label_key)
                    .font(p.label)
                    .color(button_ink(style, p))
                    .wrap(),
            ),
        style,
        p,
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
        Dialog::CloseWorkspace(_, name) | Dialog::CloseTerminal(_, name) => (
            "Close workspace?",
            name.clone(),
            "Close workspace",
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
                                    .on_press_tx(|s: &mut State, tx| s.submit_dialog(tx)),
                            ),
                    ),
            ),
    )
}
