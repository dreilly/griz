//! Client-local selection and mouse gestures; application input and history requests use IPC.
use crate::{
    Command, State,
    terminal_view::{PAD, cell_size},
};
use griz_protocol::{MouseAction, MouseButton, MouseInput, Screen, TerminalId};
use rune::{Drag, Mods, Phase, PointerButton, Tx, WheelEvent};
use std::time::{Duration, Instant};

const TOAST_DURATION: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(crate) struct Interaction {
    pub selection: Option<Selection>,
    gesture: Option<Gesture>,
    wheel: Option<(TerminalId, bool, f32)>,
    pub copied_until: Option<Instant>,
}

struct Gesture {
    id: TerminalId,
    /// The focus command queued by this very press may still be awaiting its frame.
    focus_pending: Option<u64>,
    reporting: bool,
    core_pid: u32,
    shell_pid: u32,
    button: MouseButton,
    last: (f32, f32),
}

#[derive(Clone, Copy)]
enum Unit {
    Character,
    Word,
    Line,
}

pub(crate) struct Selection {
    pub id: TerminalId,
    screen: Screen,
    anchor: usize,
    head: usize,
    unit: Unit,
    origin: (usize, usize),
}

impl Selection {
    fn new(id: TerminalId, screen: Screen, x: f32, y: f32, cw: f32, ch: f32, clicks: u32) -> Self {
        let unit = match clicks {
            2 => Unit::Word,
            3.. => Unit::Line,
            _ => Unit::Character,
        };
        let mut selection = Self {
            id,
            screen,
            anchor: 0,
            head: 0,
            unit,
            origin: (0, 0),
        };
        let point = selection.point(x, y, cw, ch);
        selection.origin = selection.bounds(point);
        selection.anchor = selection.origin.0;
        selection.head = selection.origin.1;
        selection
    }

    fn point(&self, x: f32, y: f32, cw: f32, ch: f32) -> usize {
        let row = ((y - PAD) / ch)
            .floor()
            .clamp(0.0, f32::from(self.screen.rows - 1)) as usize;
        let col = match self.unit {
            Unit::Character => ((x - PAD) / cw).round(),
            _ => ((x - PAD) / cw).floor(),
        }
        .clamp(0.0, f32::from(self.screen.cols)) as usize;
        row * usize::from(self.screen.cols) + col
    }

    fn bounds(&self, point: usize) -> (usize, usize) {
        let cols = usize::from(self.screen.cols);
        let row = (point / cols).min(usize::from(self.screen.rows) - 1);
        let start = row * cols;
        match self.unit {
            Unit::Character => (point, point),
            Unit::Line => (start, start + cols),
            Unit::Word => {
                let cells = &self.screen.cells[row];
                let mut col = (point - start).min(cells.len() - 1);
                while col > 0 && cells[col].width == 0 {
                    col -= 1;
                }
                let class = |col: usize| {
                    let c = cells[col].text.chars().next().unwrap_or(' ');
                    if c.is_whitespace() {
                        0
                    } else if c.is_alphanumeric() || c == '_' {
                        1
                    } else {
                        2
                    }
                };
                let kind = class(col);
                let mut left = col;
                let mut right = col + usize::from(cells[col].width.max(1));
                while left > 0 && (cells[left - 1].width == 0 || class(left - 1) == kind) {
                    left -= 1;
                }
                while right < cells.len() && (cells[right].width == 0 || class(right) == kind) {
                    right += 1;
                }
                (start + left, start + right)
            }
        }
    }

    fn extend(&mut self, x: f32, y: f32, cw: f32, ch: f32) {
        let point = self.point(x, y, cw, ch);
        let (start, end) = self.bounds(point);
        if point < self.origin.0 {
            self.anchor = self.origin.1;
            self.head = start;
        } else {
            self.anchor = self.origin.0;
            self.head = end;
        }
    }

    /// Expand endpoints to include an entire wide grapheme, never half of one.
    fn range(&self) -> (usize, usize) {
        let cols = usize::from(self.screen.cols);
        let (mut start, mut end) = (self.anchor.min(self.head), self.anchor.max(self.head));
        if start == end {
            return (start, end);
        }
        let cell = |p: usize| {
            self.screen
                .cells
                .get(p / cols)
                .and_then(|r| r.get(p % cols))
        };
        while start % cols > 0 && cell(start).is_some_and(|c| c.width == 0) {
            start -= 1;
        }
        if cell(end).is_some_and(|c| c.width == 0) {
            end += 1;
        }
        (start, end)
    }

    pub fn columns(&self, row: usize) -> Option<(usize, usize)> {
        let cols = usize::from(self.screen.cols);
        let (start, end) = self.range();
        let left = start.max(row * cols);
        let right = end.min((row + 1) * cols);
        (right > left).then(|| (left - row * cols, right - row * cols))
    }

    fn text(&self) -> String {
        let multiline = self.anchor / usize::from(self.screen.cols)
            != self.head / usize::from(self.screen.cols);
        let selected: Vec<_> = self
            .screen
            .cells
            .iter()
            .enumerate()
            .filter_map(|(row, cells)| self.columns(row).map(|columns| (row, cells, columns)))
            .collect();
        let mut text = String::new();
        for (index, (row, cells, (start, end))) in selected.iter().enumerate() {
            let trim_padding =
                *end == cells.len() && (multiline || !matches!(self.unit, Unit::Character));
            let mut pending_empty = 0;
            for cell in &cells[*start..*end] {
                if cell.width == 0 || cell.wide_spacer_head {
                    continue;
                }
                if cell.text.is_empty() {
                    pending_empty += usize::from(cell.width.max(1));
                } else {
                    text.extend(std::iter::repeat_n(' ', pending_empty));
                    pending_empty = 0;
                    text.push_str(&cell.text);
                }
            }
            if !trim_padding {
                text.extend(std::iter::repeat_n(' ', pending_empty));
            }
            if index + 1 < selected.len() && !self.screen.is_row_soft_wrapped(*row) {
                text.push('\n');
            }
        }
        text
    }
}

fn selectable(state: &State, id: TerminalId, pending: Option<u64>) -> bool {
    !state.settings_open
        && state.dialog.is_none()
        && !state.palette.is_open()
        && (state.pending.is_none() || state.pending == pending)
        && state.pane_frame(id).is_some()
}

fn available(state: &State, id: TerminalId, pending: Option<u64>) -> bool {
    selectable(state, id, pending)
        && state.online
        && !state.offline
        && state
            .pane_frame(id)
            .is_some_and(|frame| frame.exited.is_none())
}

fn event(
    state: &State,
    action: MouseAction,
    button: Option<MouseButton>,
    x: f32,
    y: f32,
    mods: Mods,
) -> MouseInput {
    let (cw, ch) = cell_size(&state.settings);
    MouseInput {
        action,
        button,
        x: (x - PAD).max(0.0),
        y: (y - PAD).max(0.0),
        cell_width: cw as u16,
        cell_height: ch as u16,
        shift: mods.shift,
        alt: mods.alt,
        ctrl: mods.ctrl,
    }
}

fn send(
    state: &mut State,
    id: TerminalId,
    action: MouseAction,
    button: Option<MouseButton>,
    x: f32,
    y: f32,
    mods: Mods,
) {
    let event = event(state, action, button, x, y, mods);
    state.send(Command::Mouse {
        terminal_id: id,
        event,
    });
}

pub fn drag(state: &mut State, id: TerminalId, drag: &Drag, tx: &mut Tx) {
    tx.instant();
    let (cw, ch) = cell_size(&state.settings);
    if drag.phase == Phase::Down {
        if !selectable(state, id, None) {
            return;
        }
        let mods = tx.mods();
        let reporting = available(state, id, None)
            && !mods.shift
            && state
                .pane_frame(id)
                .is_some_and(|f| f.screen.mouse_tracking && f.screen.scrollback_offset == 0);
        let button = match tx.button() {
            PointerButton::Middle => MouseButton::Middle,
            _ => MouseButton::Left,
        };
        state.terminal_mouse.selection = None;
        if state.active_terminal() != Some(id) {
            state.change(Command::SelectTerminal { terminal_id: id });
        } else {
            state.acknowledge_focused_agents();
        }
        tx.blur();
        if reporting {
            send(
                state,
                id,
                MouseAction::Press,
                Some(button),
                drag.x,
                drag.y,
                mods,
            );
        } else if button == MouseButton::Left {
            let screen = state.pane_frame(id).unwrap().screen.clone();
            state.terminal_mouse.selection = Some(Selection::new(
                id,
                screen,
                drag.x,
                drag.y,
                cw,
                ch,
                drag.clicks,
            ));
        }
        state.terminal_mouse.gesture = Some(Gesture {
            id,
            focus_pending: state.pending,
            reporting,
            button,
            last: (drag.x, drag.y),
            core_pid: state.snapshot.as_ref().unwrap().core_pid,
            shell_pid: state.pane_frame(id).unwrap().shell_pid,
        });
        return;
    }
    let Some(mut gesture) = state.terminal_mouse.gesture.take() else {
        return;
    };
    if gesture.id != id
        || !selectable(state, id, gesture.focus_pending)
        || (gesture.reporting && !available(state, id, gesture.focus_pending))
    {
        state.terminal_mouse.selection = None;
        return;
    }
    if gesture.reporting {
        let action = if drag.phase == Phase::Up {
            MouseAction::Release
        } else {
            MouseAction::Motion
        };
        // A cancelled gesture must still release the button the application received.
        send(
            state,
            id,
            action,
            Some(gesture.button),
            drag.x,
            drag.y,
            tx.mods(),
        );
        tx.live_only();
    } else if drag.cancelled() {
        state.terminal_mouse.selection = None;
    } else if let Some(selection) = &mut state.terminal_mouse.selection {
        selection.extend(drag.x, drag.y, cw, ch);
        if drag.phase == Phase::Up {
            copy_selection(state, tx);
            if state
                .terminal_mouse
                .selection
                .as_ref()
                .is_some_and(|s| s.range().0 == s.range().1)
            {
                state.terminal_mouse.selection = None;
            }
        }
    }
    if drag.phase != Phase::Up {
        gesture.last = (drag.x, drag.y);
        state.terminal_mouse.gesture = Some(gesture);
    }
}

pub fn motion(state: &mut State, id: TerminalId, x: f32, y: f32, tx: &mut Tx) {
    if state.terminal_mouse.gesture.is_none()
        && available(state, id, None)
        && !tx.mods().shift
        && state
            .pane_frame(id)
            .is_some_and(|f| f.screen.mouse_tracking && f.screen.scrollback_offset == 0)
    {
        send(state, id, MouseAction::Motion, None, x, y, tx.mods());
    }
    tx.live_only();
}

pub fn secondary(state: &mut State, id: TerminalId, x: f32, y: f32, tx: &mut Tx) {
    if available(state, id, None)
        && !tx.mods().shift
        && state
            .pane_frame(id)
            .is_some_and(|f| f.screen.mouse_tracking && f.screen.scrollback_offset == 0)
    {
        if state.active_terminal() != Some(id) {
            state.change(Command::SelectTerminal { terminal_id: id });
        }
        send(
            state,
            id,
            MouseAction::Press,
            Some(MouseButton::Right),
            x,
            y,
            tx.mods(),
        );
        send(
            state,
            id,
            MouseAction::Release,
            Some(MouseButton::Right),
            x,
            y,
            tx.mods(),
        );
        tx.blur();
    }
}

pub fn wheel(state: &mut State, id: TerminalId, wheel: WheelEvent, tx: &mut Tx) -> bool {
    if !selectable(state, id, None) || !state.online || state.offline {
        state.terminal_mouse.wheel = None;
        tx.live_only();
        return false;
    }
    let reporting = available(state, id, None)
        && !wheel.mods.shift
        && state
            .pane_frame(id)
            .is_some_and(|f| f.screen.mouse_tracking && f.screen.scrollback_offset == 0);
    if wheel.phase == rune::GesturePhase::Began {
        state.terminal_mouse.wheel = None;
    }
    let (_, ch) = cell_size(&state.settings);
    let previous = state
        .terminal_mouse
        .wheel
        .filter(|(old, old_reporting, _)| *old == id && *old_reporting == reporting)
        .map_or(0.0, |(_, _, delta)| delta);
    let delta = previous + wheel.dy / ch;
    let steps = delta.trunc().clamp(-32.0, 32.0) as i32;
    state.terminal_mouse.wheel =
        (wheel.phase != rune::GesturePhase::Ended).then_some((id, reporting, delta.fract()));
    if !reporting {
        if steps != 0 {
            if state.terminal_mouse.selection.take().is_some() {
                tx.instant();
            } else {
                tx.live_only();
            }
            state.send(Command::Scroll {
                terminal_id: id,
                rows: -steps,
            });
        } else {
            tx.live_only();
        }
        return true;
    }
    tx.live_only();
    let button = if steps > 0 {
        MouseButton::WheelUp
    } else {
        MouseButton::WheelDown
    };
    for _ in 0..steps.abs() {
        send(
            state,
            id,
            MouseAction::Press,
            Some(button),
            wheel.x,
            wheel.y,
            wheel.mods,
        );
    }
    true
}

pub fn copy_selection(state: &mut State, tx: &mut Tx) -> bool {
    let Some(selection) = &state.terminal_mouse.selection else {
        return false;
    };
    if selection.range().0 == selection.range().1 {
        return false;
    }
    let text = selection.text();
    if text.is_empty() {
        // A normalized blank line is still a selection: never turn Copy into Ctrl-C.
        return true;
    }
    tx.copy(text);
    state.terminal_mouse.copied_until = Some(Instant::now() + TOAST_DURATION);
    tx.wake_after(TOAST_DURATION);
    true
}

/// Don't let a selection silently move to unrelated text after output, resizing or switching.
pub fn reconcile(state: &mut State) {
    let valid = state
        .terminal_mouse
        .selection
        .as_ref()
        .is_none_or(|selection| {
            state.pane_frame(selection.id).is_some_and(|frame| {
                frame.screen.cols == selection.screen.cols
                    && frame.screen.rows == selection.screen.rows
                    && frame.screen.cells == selection.screen.cells
                    && (0..usize::from(frame.screen.rows)).all(|row| {
                        frame.screen.is_row_soft_wrapped(row)
                            == selection.screen.is_row_soft_wrapped(row)
                    })
            })
        });
    if !valid {
        state.terminal_mouse.selection = None;
    }
    if state.terminal_mouse.gesture.as_ref().is_some_and(|g| {
        state
            .snapshot
            .as_ref()
            .is_none_or(|s| s.core_pid != g.core_pid)
            || state
                .pane_frame(g.id)
                .is_none_or(|f| f.shell_pid != g.shell_pid)
    }) {
        state.terminal_mouse.gesture = None;
        state.terminal_mouse.selection = None;
    }
}

pub fn poll(state: &mut State, tx: &mut Tx) -> bool {
    if let Some(until) = state.terminal_mouse.copied_until {
        let now = Instant::now();
        if now >= until {
            state.terminal_mouse.copied_until = None;
            tx.instant();
            return true;
        }
        tx.wake_after(until - now);
    }
    false
}

pub fn cancel(state: &mut State) {
    if let Some(gesture) = state.terminal_mouse.gesture.take()
        && gesture.reporting
    {
        send(
            state,
            gesture.id,
            MouseAction::Release,
            Some(gesture.button),
            gesture.last.0,
            gesture.last.1,
            Mods::default(),
        );
    }
    discard(state);
}

/// A replacement core/shell must never receive the previous program's release.
pub fn discard(state: &mut State) {
    state.terminal_mouse.gesture = None;
    state.terminal_mouse.selection = None;
    state.terminal_mouse.wheel = None;
}
