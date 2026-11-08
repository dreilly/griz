//! Recursive tab layouts. Every leaf retains its own rounded terminal frame.
use crate::{
    Command, State,
    settings::Action,
    terminal_view,
    ui::{Palette, button},
};
use griz_protocol::{PaneLayout, SplitDirection, TerminalId};
use rune::{El, col, el, row, text};

pub fn stage(state: &State, p: &Palette) -> El {
    if let Some(tab) = state.active_tab() {
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
            if let Some(id) = id
                && s.active_terminal() != Some(id)
            {
                s.change(Command::SelectTerminal { terminal_id: id });
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
    if split {
        header = header.child(
            button(key("close-pane"), "×", p)
                .label(format!("Close terminal {label}"))
                .disabled(!state.ready())
                .on_press_tx(move |s: &mut State, tx| {
                    if let Some(id) = id {
                        s.request_close_terminal(id, tx);
                    }
                }),
        );
    }
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

pub fn split_button(state: &State, p: &Palette, action: Action) -> El {
    let (key, label, help) = match action {
        Action::SplitHorizontal => ("split-horizontal", "↕", "Split horizontally (top / bottom)"),
        _ => ("split-vertical", "↔", "Split vertically (left / right)"),
    };
    button(key, label, p)
        .label(help)
        .help(help)
        .disabled(!state.can_split())
        .on_press_tx(move |s: &mut State, tx| s.act(action, tx))
}
