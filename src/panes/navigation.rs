//! Directional focus uses the complete layout, including panes hidden by zoom.
use crate::{State, settings::Action};
use griz_protocol::{PaneLayout, SplitDirection, TerminalId};

pub(crate) fn neighbor(state: &State, direction: Action) -> Option<TerminalId> {
    let active = state.active_terminal()?;
    let mut panes = Vec::new();
    rectangles(
        state,
        "root",
        &state.active_tab()?.layout,
        [0.0, 0.0, 1.0, 1.0],
        &mut panes,
    );
    let (_, current) = panes.iter().find(|(id, _)| *id == active)?;
    let horizontal = matches!(direction, Action::FocusPaneLeft | Action::FocusPaneRight);
    let forward = matches!(direction, Action::FocusPaneRight | Action::FocusPaneDown);
    let axis = if horizontal { 0 } else { 1 };
    let cross = 1 - axis;
    panes
        .iter()
        .filter(|(id, _)| *id != active)
        .filter_map(|(id, rect)| {
            let gap = if forward {
                rect[axis] - (current[axis] + current[axis + 2])
            } else {
                current[axis] - (rect[axis] + rect[axis + 2])
            };
            let overlap = (rect[cross] + rect[cross + 2]).min(current[cross] + current[cross + 2])
                - rect[cross].max(current[cross]);
            // Only neighbors sharing some of the perpendicular span count; no diagonal jumps.
            if gap < -0.0001 || overlap <= 0.0001 {
                return None;
            }
            let offset = ((rect[cross] + rect[cross + 2] / 2.0)
                - (current[cross] + current[cross + 2] / 2.0))
                .abs();
            Some((*id, gap.max(0.0), offset))
        })
        .min_by(|a, b| {
            a.1.total_cmp(&b.1)
                .then(a.2.total_cmp(&b.2))
                .then(a.0.cmp(&b.0))
        })
        .map(|(id, _, _)| id)
}

fn rectangles(
    state: &State,
    path: &str,
    node: &PaneLayout,
    rect: [f32; 4],
    out: &mut Vec<(TerminalId, [f32; 4])>,
) {
    match node {
        PaneLayout::Leaf { terminal_id } => out.push((*terminal_id, rect)),
        PaneLayout::Split {
            direction,
            first,
            second,
        } => {
            let axis = if *direction == SplitDirection::Vertical {
                0
            } else {
                1
            };
            let mut a = rect;
            let mut b = rect;
            a[axis + 2] *= super::interaction::ratio(state, path, node);
            b[axis] += a[axis + 2];
            b[axis + 2] -= a[axis + 2];
            rectangles(state, &format!("{path}-first"), first, a, out);
            rectangles(state, &format!("{path}-second"), second, b, out);
        }
    }
}
