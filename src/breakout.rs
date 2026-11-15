//! A hidden, client-only Breakout game. No workspace or PTY state is changed.
use crate::{State, ui::Palette};
use rune::{Color, Cx, El, Key, KeyEvent, Tx, col, el, row, text};
use std::time::Duration;

const PADDLE_W: f32 = 0.17;
const PADDLE_Y: f32 = 0.91;
const BALL_R: f32 = 7.0;

const LABELS: [&str; 12] = [
    "π PI",
    "CODEX",
    "CLAUDE",
    "OPENCODE",
    "ANTHROPIC",
    "OPENAI",
    "GPT",
    "TOKENS",
    "CONTEXT",
    "RATE LIMITS",
    "HALLUCINATIONS",
    "AI SLOP",
];

const QUIPS: [&str; 8] = [
    "Context window shattered",
    "Rate limit revoked",
    "Benchmark optimized (legally)",
    "One fewer hallucination",
    "Agent successfully unlooped",
    "Tokens returned to the wild",
    "Prompt injection deflected",
    "Model alignment: mostly straight",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Ready,
    Playing,
    Won,
    Lost,
}

#[derive(Clone, Debug)]
struct Brick {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: usize,
    alive: bool,
}

pub(crate) struct Game {
    pub(crate) phase: Phase,
    pub(crate) score: u32,
    pub(crate) lives: u8,
    pub(crate) paddle_x: f32,
    pub(crate) ball: [f32; 2],
    velocity: [f32; 2],
    board_size: [f32; 2],
    bricks: Vec<Brick>,
    combo: u32,
    hits: usize,
    last_hit: Option<usize>,
    last_tick: Option<f64>,
}

impl Default for Game {
    fn default() -> Self {
        let cols = 6;
        let rows = 4;
        let margin = 0.055;
        let gap = 0.012;
        let width = (1.0 - margin * 2.0 - gap * (cols - 1) as f32) / cols as f32;
        let mut bricks = Vec::with_capacity(cols * rows);
        for row in 0..rows {
            for col in 0..cols {
                bricks.push(Brick {
                    x: margin + col as f32 * (width + gap),
                    y: 0.12 + row as f32 * 0.072,
                    w: width,
                    h: 0.048,
                    label: (row * cols + col) % LABELS.len(),
                    alive: true,
                });
            }
        }
        Self {
            phase: Phase::Ready,
            score: 0,
            lives: 3,
            paddle_x: 0.5,
            ball: [0.5, 0.86],
            velocity: [0.29, -0.48],
            board_size: [900.0, 540.0],
            bricks,
            combo: 1,
            hits: 0,
            last_hit: None,
            last_tick: None,
        }
    }
}

impl Game {
    fn move_paddle(&mut self, delta: f32) {
        self.paddle_x = (self.paddle_x + delta).clamp(PADDLE_W / 2.0, 1.0 - PADDLE_W / 2.0);
        if self.phase == Phase::Ready {
            self.ball[0] = self.paddle_x;
        }
    }

    fn point_paddle(&mut self, x: f32) {
        self.paddle_x = x.clamp(PADDLE_W / 2.0, 1.0 - PADDLE_W / 2.0);
        if self.phase == Phase::Ready {
            self.ball[0] = self.paddle_x;
        }
    }

    fn launch(&mut self) {
        match self.phase {
            Phase::Ready => {
                let direction = if (self.hits + self.lives as usize) % 2 == 0 {
                    -1.0
                } else {
                    1.0
                };
                self.velocity = [0.29 * direction, -0.48];
                self.phase = Phase::Playing;
                self.last_tick = None;
            }
            Phase::Won | Phase::Lost => *self = Self::default(),
            Phase::Playing => {}
        }
    }

    fn reset_ball(&mut self) {
        self.phase = Phase::Ready;
        self.paddle_x = 0.5;
        self.ball = [self.paddle_x, 0.86];
        self.velocity = [0.29, -0.48];
        self.combo = 1;
        self.last_tick = None;
    }

    fn step(&mut self, dt: f32) {
        if self.phase != Phase::Playing {
            return;
        }
        let dt = dt.clamp(0.0, 1.0 / 30.0);
        let previous = self.ball;
        self.ball[0] += self.velocity[0] * dt;
        self.ball[1] += self.velocity[1] * dt;

        let rx = BALL_R / self.board_size[0].max(1.0);
        let ry = BALL_R / self.board_size[1].max(1.0);
        if self.ball[0] - rx <= 0.0 {
            self.ball[0] = rx;
            self.velocity[0] = self.velocity[0].abs();
        } else if self.ball[0] + rx >= 1.0 {
            self.ball[0] = 1.0 - rx;
            self.velocity[0] = -self.velocity[0].abs();
        }
        if self.ball[1] - ry <= 0.025 {
            self.ball[1] = 0.025 + ry;
            self.velocity[1] = self.velocity[1].abs();
        }

        let paddle_left = self.paddle_x - PADDLE_W / 2.0;
        let paddle_top = PADDLE_Y;
        if self.velocity[1] > 0.0
            && self.ball[1] + ry >= paddle_top
            && previous[1] + ry <= paddle_top + 0.025
            && self.ball[0] + rx >= paddle_left
            && self.ball[0] - rx <= paddle_left + PADDLE_W
        {
            self.ball[1] = paddle_top - ry;
            let hit = ((self.ball[0] - self.paddle_x) / (PADDLE_W / 2.0)).clamp(-1.0, 1.0);
            let speed = (self.velocity[0].hypot(self.velocity[1]) * 1.025).clamp(0.5, 0.82);
            self.velocity[0] = hit * speed * 0.78;
            self.velocity[1] = -(speed * speed - self.velocity[0] * self.velocity[0])
                .max(0.08)
                .sqrt();
            self.combo = 1;
        }

        let mut hit_brick = None;
        for (index, brick) in self.bricks.iter().enumerate() {
            if brick.alive
                && self.ball[0] + rx >= brick.x
                && self.ball[0] - rx <= brick.x + brick.w
                && self.ball[1] + ry >= brick.y
                && self.ball[1] - ry <= brick.y + brick.h
            {
                hit_brick = Some(index);
                break;
            }
        }
        if let Some(index) = hit_brick {
            let brick = &mut self.bricks[index];
            brick.alive = false;
            let vertical = previous[1] + ry <= brick.y || previous[1] - ry >= brick.y + brick.h;
            if vertical {
                self.velocity[1] = -self.velocity[1];
            } else {
                self.velocity[0] = -self.velocity[0];
            }
            self.score += 100 * self.combo;
            self.combo = (self.combo + 1).min(9);
            self.hits += 1;
            self.last_hit = Some(brick.label);
            if self.bricks.iter().all(|brick| !brick.alive) {
                self.phase = Phase::Won;
                self.last_tick = None;
            }
        }

        if self.ball[1] - ry > 1.0 {
            self.lives = self.lives.saturating_sub(1);
            if self.lives == 0 {
                self.phase = Phase::Lost;
                self.last_tick = None;
            } else {
                self.reset_ball();
            }
        }
    }
}

pub(crate) fn open(state: &mut State, tx: &mut Tx) {
    crate::terminal_mouse::cancel(state);
    state.prefix_waiting = false;
    state.preedit.clear();
    state.breakout = Some(Game::default());
    tx.blur();
    tx.instant();
}

pub(crate) fn key(state: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(game) = state.breakout.as_mut() else {
        return false;
    };
    match &event.key {
        Key::Escape => {
            state.breakout = None;
            tx.blur();
        }
        Key::Left => game.move_paddle(-0.065),
        Key::Right => game.move_paddle(0.065),
        Key::Enter => {
            if !tx.key_repeat() {
                game.launch();
                tx.wake_after(Duration::ZERO);
            }
        }
        Key::Text(t) if t.eq_ignore_ascii_case("a") => game.move_paddle(-0.065),
        Key::Text(t) if t.eq_ignore_ascii_case("d") => game.move_paddle(0.065),
        Key::Text(t) if t == " " => {
            if !tx.key_repeat() {
                game.launch();
                tx.wake_after(Duration::ZERO);
            }
        }
        Key::Text(t) if t.eq_ignore_ascii_case("r") => {
            if !tx.key_repeat() {
                *game = Game::default();
            }
        }
        _ => {}
    }
    true
}

pub(crate) fn poll(state: &mut State, now: f64, tx: &mut Tx) -> bool {
    let Some(game) = state.breakout.as_mut() else {
        return false;
    };
    if game.phase != Phase::Playing {
        game.last_tick = None;
        return false;
    }
    let dt = game
        .last_tick
        .replace(now)
        .map(|last| (now - last) as f32)
        .unwrap_or(1.0 / 60.0);
    game.step(dt);
    if game.phase == Phase::Playing {
        tx.wake_after(Duration::ZERO);
    }
    true
}

fn rgb(value: [u8; 3]) -> Color {
    Color {
        r: value[0] as f32 / 255.0,
        g: value[1] as f32 / 255.0,
        b: value[2] as f32 / 255.0,
        a: 1.0,
    }
}

fn brick_color(label: usize) -> Color {
    [
        rgb([103, 232, 168]),
        rgb([117, 169, 255]),
        rgb([213, 154, 255]),
        rgb([255, 188, 92]),
        rgb([255, 113, 134]),
        rgb([78, 219, 255]),
    ][label % 6]
}

fn phase_copy(game: &Game) -> (&'static str, &'static str) {
    match game.phase {
        Phase::Ready => (
            "PRESS SPACE TO DEPLOY",
            "Move with ← → or A D · mouse works too",
        ),
        Phase::Playing => (
            "BREAKING THE MODEL LAYER",
            if game.hits == 0 {
                "Aim for the expensive rectangles"
            } else {
                QUIPS[(game.hits - 1) % QUIPS.len()]
            },
        ),
        Phase::Won => ("ALL SYSTEMS BROKEN ✓", "Press Space to rebuild the stack"),
        Phase::Lost => (
            "CONTEXT WINDOW EXHAUSTED",
            "Press Space to retry · R resets",
        ),
    }
}

pub(crate) fn panel(state: &State, p: &Palette, cx: &Cx<State>) -> Option<El> {
    let game = state.breakout.as_ref()?;
    let [window_w, window_h] = cx.layout_size("root").unwrap_or([1100.0, 720.0]);
    let board_x = 24.0;
    let board_y = 88.0;
    let board_w = (window_w - 48.0).max(260.0);
    let board_h = (window_h - board_y - 24.0).max(190.0);
    let (status, hint) = phase_copy(game);

    let mut board = el()
        .key("breakout-board")
        .role("application")
        .label("Griz Breakout game board")
        .absolute(board_x, board_y)
        .size(board_w, board_h)
        .clip_soft(14.0)
        .radius(14.0)
        .fill(rgb([7, 12, 24]))
        .border(1.0, p.accent.alpha(0.35))
        .shadow(12.0, 36.0, Color::BLACK.alpha(0.55))
        .on_layout_tx(|s: &mut State, [_, _, width, height], tx| {
            if let Some(game) = s.breakout.as_mut() {
                game.board_size = [width.max(1.0), height.max(1.0)];
            }
            tx.live_only();
        })
        .on_pointer_move_local(|s: &mut State, pointer, tx| {
            if let Some(game) = s.breakout.as_mut() {
                game.point_paddle(pointer.x / game.board_size[0].max(1.0));
            }
            tx.instant();
        })
        .on_press_tx(|s: &mut State, tx| {
            if let Some(game) = s.breakout.as_mut() {
                game.launch();
                tx.wake_after(Duration::ZERO);
            }
        });

    for col in 1..12 {
        board = board.child(
            el().absolute(board_w * col as f32 / 12.0, 0.0)
                .size(1.0, board_h)
                .fill(p.ink.alpha(0.025))
                .inert(),
        );
    }
    for row in 1..8 {
        board = board.child(
            el().absolute(0.0, board_h * row as f32 / 8.0)
                .size(board_w, 1.0)
                .fill(p.ink.alpha(0.025))
                .inert(),
        );
    }

    for (index, brick) in game
        .bricks
        .iter()
        .enumerate()
        .filter(|(_, brick)| brick.alive)
    {
        let color = brick_color(brick.label);
        board = board.child(
            el().key(format!("breakout-brick-{index}"))
                .absolute(brick.x * board_w, brick.y * board_h)
                .size(brick.w * board_w, brick.h * board_h)
                .radius(5.0)
                .fill(color.alpha(0.20))
                .border(1.0, color.alpha(0.72))
                .shadow(3.0, 10.0, color.alpha(0.16))
                .center()
                .inert()
                .child(
                    text(LABELS[brick.label])
                        .font(p.label)
                        .color(color)
                        .truncate(),
                ),
        );
    }

    let ball_x = game.ball[0] * board_w - BALL_R;
    let ball_y = game.ball[1] * board_h - BALL_R;
    if game.phase == Phase::Playing {
        for trail in 1..=3 {
            let distance = trail as f32 * 0.012;
            board = board.child(
                el().absolute(
                    (game.ball[0] - game.velocity[0] * distance) * board_w - BALL_R,
                    (game.ball[1] - game.velocity[1] * distance) * board_h - BALL_R,
                )
                .size(BALL_R * 2.0, BALL_R * 2.0)
                .radius(BALL_R)
                .fill(p.accent.alpha(0.16 / trail as f32))
                .inert(),
            );
        }
    }
    board = board
        .child(
            el().key("breakout-ball")
                .absolute(ball_x, ball_y)
                .size(BALL_R * 2.0, BALL_R * 2.0)
                .radius(BALL_R)
                .fill(rgb([235, 255, 244]))
                .shadow(5.0, 18.0, p.accent.alpha(0.8))
                .inert(),
        )
        .child(
            el().key("breakout-paddle")
                .absolute(
                    (game.paddle_x - PADDLE_W / 2.0) * board_w,
                    PADDLE_Y * board_h,
                )
                .size(PADDLE_W * board_w, 12.0)
                .radius(6.0)
                .fill(p.accent)
                .shadow(6.0, 22.0, p.accent.alpha(0.55))
                .inert(),
        )
        .child(
            col()
                .key("breakout-message")
                .absolute(board_w * 0.2, board_h * 0.70)
                .w(board_w * 0.6)
                .gap(5.0)
                .center()
                .inert()
                .child(text(status).font(p.section).color(p.accent))
                .child(text(hint).font(p.label).color(p.muted).wrap()),
        );

    if let Some(label) = game.last_hit {
        board = board.child(
            text(format!("{} DEPRECATED", LABELS[label]))
                .key("breakout-last-hit")
                .absolute(18.0, board_h - 30.0)
                .font(p.section)
                .color(brick_color(label).alpha(0.78))
                .inert(),
        );
    }

    Some(
        el().key("breakout-workspace")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .fill(rgb([5, 8, 17]))
            .role("dialog")
            .label("Secret Griz Breakout workspace")
            .modal(true, |s: &mut State| s.breakout = None)
            .child(
                row()
                    .key("breakout-header")
                    .absolute(24.0, 18.0)
                    .size(board_w, 52.0)
                    .gap(12.0)
                    .center_y()
                    .child(
                        col()
                            .gap(1.0)
                            .child(text("GRIZ BREAKOUT").font(p.title).color(p.ink))
                            .child(
                                text("MODEL SMASH // definitely productive")
                                    .font(p.section)
                                    .color(p.accent),
                            ),
                    )
                    .child(el().grow(1.0))
                    .child(
                        col()
                            .center()
                            .child(text("SCORE").font(p.section).color(p.muted))
                            .child(
                                text(format!("{:06}", game.score))
                                    .font(p.title)
                                    .color(p.ink),
                            ),
                    )
                    .child(
                        col()
                            .center()
                            .child(text("LIVES").font(p.section).color(p.muted))
                            .child(
                                text("●".repeat(game.lives as usize))
                                    .font(p.title)
                                    .color(rgb([255, 113, 134])),
                            ),
                    )
                    .child(
                        crate::ui::button("breakout-close", "Back to work", p)
                            .focusable(false)
                            .on_press_tx(|s: &mut State, tx| {
                                s.breakout = None;
                                tx.blur();
                            }),
                    ),
            )
            .child(board),
    )
}
