//! Recursive tab layouts. Every leaf retains its own rounded terminal frame.
use crate::{
    Command, State, terminal_view,
    ui::{Palette, button},
};
use griz_protocol::{PaneLayout, SplitDirection, TerminalId};
use rune::{El, Icon, col, el, icon, row, text};

static ZOOM_ICON: Icon = Icon {
    name: "griz-pane-zoom",
    view: 16.0,
    body: r#"<path d="M9 3h4v4M13 3L9 7M7 13H3V9M3 13l4-4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

static UNZOOM_ICON: Icon = Icon {
    name: "griz-pane-unzoom",
    view: 16.0,
    body: r#"<path d="M13 3L9 7m0-4v4h4M3 13l4-4m-4 0h4v4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

pub fn stage(state: &State, p: &Palette) -> El {
    if let Some(tab) = state.active_tab() {
        if state.pane_zoomed() {
            return card(state, p, state.active_terminal(), true);
        }
        layout(
            state,
            p,
            &tab.layout,
            tab.layout.terminal_ids().len() > 1,
            "root",
        )
    } else {
        card(state, p, state.active_terminal(), false)
    }
}

fn layout(state: &State, p: &Palette, node: &PaneLayout, split: bool, path: &str) -> El {
    match node {
        PaneLayout::Leaf { terminal_id } => card(state, p, Some(*terminal_id), split),
        PaneLayout::Split {
            direction,
            first,
            second,
        } => {
            let container = match direction {
                SplitDirection::Horizontal => col(),
                SplitDirection::Vertical => row(),
            };
            container
                .key(format!("pane-split-{path}"))
                .grow(1.0)
                .min_w(0.0)
                .min_h(0.0)
                .gap(10.0)
                .child(layout(state, p, first, split, &format!("{path}-first")))
                .child(layout(state, p, second, split, &format!("{path}-second")))
        }
    }
}

fn card(state: &State, p: &Palette, id: Option<TerminalId>, split: bool) -> El {
    let active = id == state.active_terminal();
    let accent = crate::ui::workspace_accent(state);
    let key = |base: &str| {
        if split {
            format!("{base}-{}", id.unwrap_or(0))
        } else {
            base.to_owned()
        }
    };
    let number = state
        .workspace()
        .and_then(|w| w.terminals.iter().position(|t| Some(t.id) == id))
        .map(|i| i + 1);
    let label = number
        .map(|n| n.to_string())
        .unwrap_or_else(|| "Terminal".into());
    let exited = id
        .and_then(|id| state.pane_frame(id))
        .is_some_and(|t| t.exited.is_some());
    let mut header = row()
        .key(key("terminal-header"))
        .h(34.0_f32.max(p.label.line_px() + 12.0))
        .w_full()
        .px(12.0)
        .gap(10.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], p.line)
        .on_press_tx(move |s: &mut State, tx| {
            if let Some(id) = id {
                if s.active_terminal() == Some(id) {
                    s.acknowledge_focused_agents();
                } else {
                    s.change(Command::SelectTerminal { terminal_id: id });
                }
            }
            tx.blur();
        })
        .child(
            text(">_")
                .key(key("terminal-header-symbol"))
                .font(p.label)
                .color(if active { accent } else { p.muted }),
        )
        .child(text(label.clone()).font(p.label).color(p.ink).truncate())
        .child(el().grow(1.0));
    if exited {
        header = header.child(
            button(key("start-shell"), "Start new shell", p)
                .disabled(!state.ready())
                .on_press_tx(move |s: &mut State, tx| {
                    if let Some(terminal_id) = id {
                        s.change(Command::StartShell { terminal_id });
                    }
                    tx.blur();
                }),
        );
    }
    header = header.child(
        row()
            .key(key("pane-controls"))
            .gap(4.0)
            .center_y()
            .child(split_button(
                state,
                p,
                key("split-vertical"),
                id,
                SplitDirection::Vertical,
            ))
            .child(split_button(
                state,
                p,
                key("split-horizontal"),
                id,
                SplitDirection::Horizontal,
            ))
            .children(split.then(|| {
                let zoomed = state.pane_zoomed();
                let help = format!(
                    "{} ({})",
                    if zoomed {
                        "Restore split layout"
                    } else {
                        "Zoom pane"
                    },
                    state
                        .settings
                        .label(crate::settings::Action::TogglePaneZoom),
                );
                pane_control(key("zoom-pane"), &help, p)
                    .border(if zoomed { 1.0 } else { 0.0 }, accent)
                    .focus_ring(2.0, -2.0, accent)
                    .child(
                        icon(
                            if zoomed { &UNZOOM_ICON } else { &ZOOM_ICON },
                            18.0 * state.settings.font_size / 14.0,
                        )
                        .key(key("zoom-pane-icon"))
                        .color(if zoomed { accent } else { p.muted }),
                    )
                    .disabled(!state.can_zoom_pane())
                    .on_press_tx(move |s: &mut State, tx| {
                        if let Some(id) = id {
                            s.toggle_pane_zoom(id);
                        }
                        tx.blur();
                    })
            }))
            .child(
                pane_button(
                    key("close-pane"),
                    "×",
                    &format!("Close terminal {label}"),
                    p,
                )
                .disabled(!state.ready() || id.is_none())
                .on_press_tx(move |s: &mut State, tx| {
                    if let Some(id) = id {
                        s.request_close_terminal(id, tx);
                    }
                }),
            ),
    );
    col()
        .key(key("terminal-card"))
        .grow(1.0)
        .min_h(0.0)
        .min_w(0.0)
        .padding(1.0)
        .radius(11.0)
        .fill(terminal_view::background_for(state.settings.theme))
        .border(1.0, if active { accent.alpha(0.55) } else { p.line })
        .focus_within_ring(1.0, -1.0, accent.alpha(0.85))
        .child(
            col()
                .key(key("terminal-content"))
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .min_w(0.0)
                .radius(10.0)
                .clip_rounded(10.0)
                .child(header)
                .child(terminal_view::viewport(state, id, split)),
        )
}

fn pane_button(key: String, icon: &str, help: &str, p: &Palette) -> El {
    pane_control(key, help, p).child(text(icon).font(p.label).color(p.muted))
}

fn pane_control(key: String, help: &str, p: &Palette) -> El {
    row()
        .key(key)
        .role("button")
        .label(help)
        .help(help)
        .focusable(true)
        .w(p.control_height())
        .h(p.control_height())
        .center()
        .radius(4.0)
        .hover(p.button_fill())
        .focus_ring(2.0, -2.0, p.accent)
}

fn split_button(
    state: &State,
    p: &Palette,
    key: String,
    id: Option<TerminalId>,
    direction: SplitDirection,
) -> El {
    let (icon, help) = match direction {
        SplitDirection::Horizontal => ("↕", "Split horizontally (top / bottom)"),
        SplitDirection::Vertical => ("↔", "Split vertically (left / right)"),
    };
    pane_button(key, icon, help, p)
        .disabled(!state.can_split() || id.is_none())
        .on_press_tx(move |s: &mut State, tx| {
            if let Some(terminal_id) = id {
                s.split_terminal(terminal_id, direction);
            }
            tx.blur();
        })
}
