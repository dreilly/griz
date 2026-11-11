//! Per-window sizing and captured pointer gestures. Moving panes is authoritative in core;
//! resizing is presentation-only, like zoom, so windows can use different proportions.
use crate::{Command, State, terminal_mouse};
use griz_protocol::{PaneLayout, PanePlacement, SplitDirection, TabInfo, TerminalId};
use rune::{Drag, Key, KeyEvent, Phase, Tx};
use std::collections::HashMap;

const GAP: f32 = 10.0;
type SplitKey = (TerminalId, String);

struct Sizing {
    layout: PaneLayout,
    ratio: f32,
}

struct Resize {
    key: SplitKey,
    layout: PaneLayout,
    rect: [f32; 4],
    original: f32,
}

pub(super) struct Move {
    pub source: TerminalId,
    pub target: Option<(TerminalId, PanePlacement)>,
    started: bool,
}

#[derive(Default)]
pub struct Interaction {
    ratios: HashMap<SplitKey, Sizing>,
    split_rects: HashMap<String, [f32; 4]>,
    pub(super) pane_rects: HashMap<TerminalId, [f32; 4]>,
    scope: Option<(TabInfo, bool)>,
    resizing: Option<Resize>,
    pub(super) dragging: Option<Move>,
}

fn node_at<'a>(mut node: &'a PaneLayout, path: &str) -> Option<&'a PaneLayout> {
    for branch in path.strip_prefix("root")?.split('-').skip(1) {
        let PaneLayout::Split { first, second, .. } = node else {
            return None;
        };
        node = match branch {
            "first" => first,
            "second" => second,
            _ => return None,
        };
    }
    Some(node)
}

pub fn reconcile(state: &mut State) {
    let scope = state
        .active_tab()
        .cloned()
        .map(|tab| (tab, state.pane_zoomed()));
    if state.pane_interaction.scope != scope {
        // A remote layout change must not commit an old captured gesture.
        cancel(state);
        state.pane_interaction.scope = scope;
    }
    // Keep unchanged geometry: Rune reports layout only when a rect changes. Clearing
    // every rect on a swap would lose unaffected siblings and ancestor dividers forever.
    // Moved/new nodes report their updated rects on the next layout pass.
    let visible = state
        .active_tab()
        .map(|t| t.layout.terminal_ids())
        .unwrap_or_default();
    let zoomed = state.pane_zoomed();
    let active = state.active_terminal();
    state
        .pane_interaction
        .pane_rects
        .retain(|id, _| visible.contains(id) && (!zoomed || active == Some(*id)));
    let scope = &state.pane_interaction.scope;
    state.pane_interaction.split_rects.retain(|path, _| {
        scope.as_ref().is_some_and(|(tab, zoomed)| {
            !zoomed && matches!(node_at(&tab.layout, path), Some(PaneLayout::Split { .. }))
        })
    });
    let snapshot = state.snapshot.as_ref();
    state
        .pane_interaction
        .ratios
        .retain(|(tab_id, path), sizing| {
            snapshot.is_some_and(|s| {
                s.workspaces
                    .iter()
                    .flat_map(|w| &w.tabs)
                    .any(|t| t.id == *tab_id && node_at(&t.layout, path) == Some(&sizing.layout))
            })
        });
}

fn enabled(state: &State) -> bool {
    state.ready()
        && !state.settings_open
        && state.dialog.is_none()
        && !state.palette.is_open()
        && !state.pane_zoomed()
}

pub(super) fn ratio(state: &State, path: &str, layout: &PaneLayout) -> f32 {
    state
        .active_tab()
        .and_then(|tab| {
            state
                .pane_interaction
                .ratios
                .get(&(tab.id, path.to_owned()))
        })
        .filter(|s| &s.layout == layout)
        .map_or(0.5, |s| s.ratio)
}

pub(super) fn split_rect(state: &mut State, tab_id: TerminalId, path: &str, rect: [f32; 4]) {
    if state.active_tab().is_some_and(|t| t.id == tab_id) && !state.pane_zoomed() {
        state
            .pane_interaction
            .split_rects
            .insert(path.to_owned(), rect);
    }
}

pub(super) fn pane_rect(state: &mut State, id: TerminalId, rect: [f32; 4]) {
    if state.pane_visible(id) {
        state.pane_interaction.pane_rects.insert(id, rect);
    }
}

// Reserve enough space for a usable header and terminal. When the window is too
// small for every nested pane, share the available room rather than overflowing.
fn minimum(node: &PaneLayout, axis: SplitDirection, control_height: f32) -> f32 {
    match node {
        PaneLayout::Leaf { .. } => {
            if axis == SplitDirection::Vertical {
                // Four right-aligned controls, gaps/padding, and half the centered grip.
                9.0 * control_height + 56.0
            } else {
                100.0_f32.max(control_height + 60.0)
            }
        }
        PaneLayout::Split {
            direction,
            first,
            second,
        } => {
            let a = minimum(first, axis, control_height);
            let b = minimum(second, axis, control_height);
            if *direction == axis {
                a + b + GAP
            } else {
                a.max(b)
            }
        }
    }
}

pub(super) fn resize(
    state: &mut State,
    path: &str,
    layout: &PaneLayout,
    drag: &mut Drag,
    tx: &mut Tx,
) {
    if drag.phase == Phase::Down {
        if !enabled(state) {
            return;
        }
        let Some(tab) = state.active_tab() else {
            return;
        };
        let key = (tab.id, path.to_owned());
        let Some(rect) = state.pane_interaction.split_rects.get(path).copied() else {
            return;
        };
        let original = ratio(state, path, layout);
        terminal_mouse::cancel(state);
        state.preedit.clear();
        state.pane_interaction.resizing = Some(Resize {
            key,
            layout: layout.clone(),
            rect,
            original,
        });
        tx.blur();
        return;
    }
    if drag.cancelled() || !enabled(state) {
        cancel(state);
        return;
    }
    let Some(resize) = &state.pane_interaction.resizing else {
        return;
    };
    if resize.key.1 != path || &resize.layout != layout {
        return;
    }
    let PaneLayout::Split {
        direction,
        first,
        second,
    } = layout
    else {
        return;
    };
    let vertical = *direction == SplitDirection::Vertical;
    let span = (resize.rect[if vertical { 2 } else { 3 }] - GAP).max(1.0);
    let control_height =
        28.0_f32.max(state.settings.ui_font(rune::tok::type_::LABEL).line_px() + 10.0);
    let a = minimum(first, *direction, control_height);
    let b = minimum(second, *direction, control_height);
    let floor = if a + b <= span { a / span } else { a / (a + b) };
    let ceiling = if a + b <= span { 1.0 - b / span } else { floor };
    let delta = if vertical { drag.dx } else { drag.dy };
    let ratio = (resize.original + delta / span).clamp(floor, ceiling);
    state.pane_interaction.ratios.insert(
        resize.key.clone(),
        Sizing {
            layout: layout.clone(),
            ratio,
        },
    );
    if drag.phase == Phase::Up {
        state.pane_interaction.resizing = None;
    }
}

pub(super) fn reset(state: &mut State, path: &str) {
    if enabled(state)
        && let Some(tab) = state.active_tab()
    {
        let key = (tab.id, path.to_owned());
        state.pane_interaction.ratios.remove(&key);
    }
}

fn placement([x, y, w, h]: [f32; 4], [px, py]: [f32; 2]) -> Option<PanePlacement> {
    if w <= 0.0 || h <= 0.0 || px < x || py < y || px > x + w || py > y + h {
        return None;
    }
    let edges = [
        ((px - x) / w, PanePlacement::Left),
        ((x + w - px) / w, PanePlacement::Right),
        ((py - y) / h, PanePlacement::Top),
        ((y + h - py) / h, PanePlacement::Bottom),
    ];
    let (distance, edge) = edges
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap();
    Some(if distance < 0.25 {
        edge
    } else {
        PanePlacement::Swap
    })
}

pub(super) fn move_pane(state: &mut State, source: TerminalId, drag: &mut Drag, tx: &mut Tx) {
    if drag.phase == Phase::Down {
        if !enabled(state) || !state.pane_visible(source) {
            return;
        }
        terminal_mouse::cancel(state);
        state.preedit.clear();
        state.pane_interaction.dragging = Some(Move {
            source,
            target: None,
            started: false,
        });
        tx.blur();
        return;
    }
    if drag.cancelled() || !enabled(state) {
        cancel(state);
        return;
    }
    let target = state
        .pane_interaction
        .pane_rects
        .iter()
        .filter(|(id, _)| **id != source && state.pane_visible(**id))
        .find_map(|(id, rect)| placement(*rect, drag.window_point()).map(|p| (*id, p)));
    let Some(moving) = &mut state.pane_interaction.dragging else {
        return;
    };
    if moving.source != source {
        return;
    }
    moving.started |= drag.dx.hypot(drag.dy) >= 4.0;
    moving.target = if moving.started { target } else { None };
    if drag.phase == Phase::Up {
        let moving = state.pane_interaction.dragging.take().unwrap();
        if let Some((target_terminal_id, placement)) = moving.target {
            state.change(Command::MovePane {
                terminal_id: source,
                target_terminal_id,
                placement,
            });
        } else if !moving.started && state.active_terminal() != Some(source) {
            state.change(Command::SelectTerminal {
                terminal_id: source,
            });
        }
    }
}

pub(super) fn cancel(state: &mut State) {
    state.pane_interaction.dragging = None;
    if let Some(resize) = state.pane_interaction.resizing.take() {
        state.pane_interaction.ratios.insert(
            resize.key,
            Sizing {
                layout: resize.layout,
                ratio: resize.original,
            },
        );
    }
}

pub fn gesture_key(state: &mut State, event: &KeyEvent) -> bool {
    let active =
        state.pane_interaction.dragging.is_some() || state.pane_interaction.resizing.is_some();
    if active && event.key == Key::Escape {
        cancel(state);
    }
    // A captured layout gesture must not send keystrokes into a shell.
    active
}

pub(super) fn preview_rect([_, _, w, h]: [f32; 4], placement: PanePlacement) -> [f32; 4] {
    match placement {
        PanePlacement::Swap => [0.0, 0.0, w, h],
        PanePlacement::Left => [0.0, 0.0, w / 2.0, h],
        PanePlacement::Right => [w / 2.0, 0.0, w / 2.0, h],
        PanePlacement::Top => [0.0, 0.0, w, h / 2.0],
        PanePlacement::Bottom => [0.0, h / 2.0, w, h / 2.0],
    }
}
