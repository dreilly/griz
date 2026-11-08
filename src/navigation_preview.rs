//! Read-only miniature of the highlighted navigation target's tab.
use crate::{State, terminal_view, ui::Palette};
use griz_protocol::{PaneLayout, PreviewFrame, PreviewTarget, SplitDirection};
use rune::{El, col, el, row, text};

pub fn panel(
    state: &State,
    p: &Palette,
    target: Option<PreviewTarget>,
    title: &str,
    width: f32,
    height: f32,
) -> El {
    let frame = state
        .snapshot
        .as_ref()
        .and_then(|s| s.preview.as_ref())
        .filter(|frame| Some(frame.target) == target && state.ready());
    let header_height = p.label.line_px() + 12.0;
    let content_height = (height - header_height - 6.0).max(0.0);
    let content = if let Some(frame) = frame {
        layout(state, p, frame, &frame.layout, width, content_height)
    } else {
        el().size(width, content_height)
            .fill(terminal_view::background_for(state.settings.theme))
            .center()
            .child(
                text(if target.is_none() {
                    "Select a workspace or terminal to preview"
                } else if !state.ready() {
                    "Preview unavailable while disconnected"
                } else {
                    "Loading preview…"
                })
                .font(p.label)
                .color(p.muted)
                .wrap()
                .padding(12.0),
            )
    };
    col()
        .key("palette-preview")
        .role("region")
        .label("Workspace preview")
        .size(width, height)
        .gap(6.0)
        .inert()
        .child(
            text(title)
                .key("palette-preview-title")
                .font(p.label)
                .color(p.muted)
                .h(header_height)
                .truncate(),
        )
        .child(content)
}

fn layout(
    state: &State,
    p: &Palette,
    frame: &PreviewFrame,
    node: &PaneLayout,
    width: f32,
    height: f32,
) -> El {
    match node {
        PaneLayout::Leaf { terminal_id } => {
            let mut card = el()
                .key(format!("palette-preview-pane-{terminal_id}"))
                .size(width, height)
                .radius(5.0)
                .clip_soft(5.0)
                .fill(terminal_view::background_for(state.settings.theme));
            if let Some(terminal) = frame.terminals.iter().find(|t| t.id == *terminal_id) {
                card = card.child(
                    terminal_view::preview(
                        *terminal_id,
                        &terminal.screen,
                        &state.settings,
                        (width - 2.0).max(0.0),
                        (height - 2.0).max(0.0),
                    )
                    .absolute(1.0, 1.0),
                );
            }
            card.border(
                1.0,
                if *terminal_id == frame.selected {
                    p.accent
                } else {
                    p.line
                },
            )
        }
        PaneLayout::Split {
            direction,
            first,
            second,
        } => {
            let gap = 6.0;
            let (container, w, h) = match direction {
                SplitDirection::Horizontal => (col(), width, ((height - gap) / 2.0).max(0.0)),
                SplitDirection::Vertical => (row(), ((width - gap) / 2.0).max(0.0), height),
            };
            container
                .size(width, height)
                .gap(gap)
                .child(layout(state, p, frame, first, w, h))
                .child(layout(state, p, frame, second, w, h))
        }
    }
}
