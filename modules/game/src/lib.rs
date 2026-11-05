//! # game
//!
//! Vy reference module 5: a small paddle-and-ball game. Space serves the
//! ball, the arrow keys move the paddle, every paddle hit scores a point,
//! a miss ends the round.
//!
//! What it exercises in the ABI: `request_frame` on every frame while the
//! ball is in play and never otherwise (SPEC-5.2.2, SPEC-6.2.2), so a page
//! at rest costs nothing; motion integrated from `dt_ms` (SPEC-5.1.2) so
//! the same frame times give the same pixels; `random` from the host for
//! the serve angle (SPEC-5.2.11), the page's only source of chance; the
//! `key_down` import polled each frame for held arrows next to `KEY_DOWN`
//! events for single steps (SPEC-5.2.6); `VISIBILITY` hidden and
//! `vy_suspend` pause the round, visible redraws the pause overlay
//! (SPEC-A.2.1, SPEC-5.1.5); `BUDGET` drops the starfield and the ball's
//! trail so the next frame does less (SPEC-5.3.3). Everything is `f32`
//! arithmetic on wasm primitives; there is no trigonometry, the serve
//! directions are a table. No capabilities.
#![no_std]

extern crate alloc;

use alloc::format;
use vy_sdk::event::key;
use vy_sdk::prelude::*;
use vy_sdk::{key_down, random};

const PAGE: Color = Color::rgb(0x02, 0x06, 0x17);
const COURT: Color = Color::rgb(0x0F, 0x17, 0x2A);
const BORDER: Color = Color::rgb(0x33, 0x41, 0x55);
const STAR: Color = Color::rgb(0x47, 0x55, 0x69);
const BALL: Color = Color::rgb(0xF8, 0xFA, 0xFC);
const PADDLE: Color = Color::rgb(0x38, 0xBD, 0xF8);
const HUD: Color = Color::rgb(0xE2, 0xE8, 0xF0);
const HUD_MUTED: Color = Color::rgb(0x94, 0xA3, 0xB8);
const OVERLAY: Color = Color::rgba(0x02, 0x06, 0x17, 0x99);

const HUD_H: f32 = 48.0;
const MARGIN: f32 = 16.0;
const BALL_R: f32 = 8.0;
const PADDLE_W: f32 = 96.0;
const PADDLE_H: f32 = 12.0;
/// Gap between the paddle's bottom edge and the court's.
const PADDLE_LIFT: f32 = 24.0;
/// Pixels per second.
const BALL_SPEED: f32 = 480.0;
const PADDLE_SPEED: f32 = 420.0;
/// One `KEY_DOWN` while the ball waits moves the paddle this far.
const PADDLE_STEP: f32 = 32.0;
/// Longest frame the physics will integrate; a longer gap is clamped so a
/// stalled tab cannot tunnel the ball through the paddle.
const MAX_DT_MS: i32 = 50;
const STARS: usize = 48;
const TRAIL: usize = 8;

/// Serve directions, unit vectors pointing up: no trigonometry in the module.
const SERVES: [(f32, f32); 8] = [
    (-0.766, -0.643),
    (-0.643, -0.766),
    (-0.5, -0.866),
    (-0.342, -0.940),
    (0.342, -0.940),
    (0.5, -0.866),
    (0.643, -0.766),
    (0.766, -0.643),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Ball on the paddle, waiting for Space.
    Waiting,
    /// Ball in play; a frame is requested every frame.
    Running,
    /// Hidden or suspended mid-round; Space resumes.
    Paused,
    /// The ball got past the paddle; Space serves again.
    Lost,
}

struct Game {
    window: Window,
    title: StyleId,
    hud: StyleId,
    hint: StyleId,
    big: StyleId,
    phase: Phase,
    /// Paddle centre x in window coordinates.
    paddle_x: f32,
    ball: (f32, f32),
    velocity: (f32, f32),
    /// Last positions of the ball, newest last.
    trail: [(f32, f32); TRAIL],
    trail_len: usize,
    score: u32,
    best: u32,
    /// Milliseconds of play in the current round, from `dt_ms`.
    elapsed_ms: u32,
    /// Set by `BUDGET`: skip the decorations.
    low_detail: bool,
}

impl Game {
    fn court(&self) -> Rect {
        Rect::new(
            MARGIN,
            HUD_H + MARGIN,
            (self.window.width - 2.0 * MARGIN).max(0.0),
            (self.window.height - HUD_H - 2.0 * MARGIN).max(0.0),
        )
    }

    fn paddle(&self) -> Rect {
        let court = self.court();
        Rect::new(
            self.paddle_x - PADDLE_W / 2.0,
            court.bottom() - PADDLE_LIFT - PADDLE_H,
            PADDLE_W,
            PADDLE_H,
        )
    }

    fn clamp_paddle(&mut self) {
        let court = self.court();
        let half = PADDLE_W / 2.0;
        self.paddle_x = self
            .paddle_x
            .clamp(court.x + half, (court.right() - half).max(court.x + half));
    }

    /// Put the ball on the paddle.
    fn rest_ball(&mut self) {
        self.ball = (self.paddle_x, self.paddle().y - BALL_R);
        self.velocity = (0.0, 0.0);
        self.trail_len = 0;
    }

    fn move_paddle(&mut self, dx: f32) -> bool {
        let before = self.paddle_x;
        self.paddle_x += dx;
        self.clamp_paddle();
        if self.phase != Phase::Running {
            self.rest_ball();
        }
        before != self.paddle_x
    }

    /// Serve: one byte from the host picks the direction (SPEC-5.2.11).
    fn serve(&mut self) {
        let mut byte = [0u8; 1];
        let _ = random(&mut byte);
        let (dx, dy) = SERVES[usize::from(byte[0]) % SERVES.len()];
        self.velocity = (dx * BALL_SPEED, dy * BALL_SPEED);
        self.score = 0;
        self.elapsed_ms = 0;
        self.trail_len = 0;
        self.phase = Phase::Running;
        let _ = log(Level::Info, "game: serve");
    }

    fn push_trail(&mut self) {
        if self.trail_len == TRAIL {
            self.trail.copy_within(1.., 0);
            self.trail_len -= 1;
        }
        self.trail[self.trail_len] = self.ball;
        self.trail_len += 1;
    }

    /// Advance the round by `dt_ms` (SPEC-5.1.2).
    fn step(&mut self, dt_ms: i32) {
        let dt_ms = dt_ms.clamp(0, MAX_DT_MS);
        let dt = dt_ms as f32 / 1000.0;
        self.elapsed_ms += dt_ms as u32;

        // Held arrows move the paddle continuously (polled, SPEC-5.2.6).
        let dir = f32::from(key_down(key::RIGHT)) - f32::from(key_down(key::LEFT));
        if dir != 0.0 {
            self.paddle_x += dir * PADDLE_SPEED * dt;
            self.clamp_paddle();
        }

        if !self.low_detail {
            self.push_trail();
        }
        let court = self.court();
        let (mut x, mut y) = self.ball;
        let (mut vx, mut vy) = self.velocity;
        x += vx * dt;
        y += vy * dt;
        if x - BALL_R < court.x {
            x = court.x + BALL_R;
            vx = vx.abs();
        } else if x + BALL_R > court.right() {
            x = court.right() - BALL_R;
            vx = -vx.abs();
        }
        if y - BALL_R < court.y {
            y = court.y + BALL_R;
            vy = vy.abs();
        }
        let paddle = self.paddle();
        if vy > 0.0
            && y + BALL_R >= paddle.y
            && y - BALL_R <= paddle.bottom()
            && x >= paddle.x - BALL_R
            && x <= paddle.right() + BALL_R
        {
            // Bounce; where the paddle was hit steers the ball.
            y = paddle.y - BALL_R;
            vy = -vy.abs();
            let offset = (x - self.paddle_x) / (PADDLE_W / 2.0);
            vx += offset * 120.0;
            self.score += 1;
            if self.score > self.best {
                self.best = self.score;
            }
        }
        if y - BALL_R > court.bottom() {
            self.phase = Phase::Lost;
            let _ = log(Level::Info, "game: lost");
        }
        self.ball = (x, y);
        self.velocity = (vx, vy);
    }

    fn message(&self) -> Option<(&'static str, &'static str)> {
        match self.phase {
            Phase::Waiting => Some(("Studs", "Mellanslag serverar · pil vänster/höger flyttar")),
            Phase::Paused => Some(("Pausad", "Mellanslag fortsätter")),
            Phase::Lost => Some(("Missat", "Mellanslag serverar igen")),
            Phase::Running => None,
        }
    }

    fn draw_stars(&self, f: &mut Frame) -> Result<(), VyError> {
        let court = self.court();
        // A fixed linear congruential sequence: part of the module, not of
        // the host's randomness, so the sky is the same on every machine.
        let mut seed: u32 = 0x9E37_79B9;
        for _ in 0..STARS {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let fx = (seed >> 16) as f32 / 65_536.0;
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let fy = (seed >> 16) as f32 / 65_536.0;
            let size = 1.0 + (seed & 1) as f32;
            f.rect(
                Rect::new(
                    court.x + 2.0 + fx * (court.w - 6.0),
                    court.y + 2.0 + fy * (court.h - 6.0),
                    size,
                    size,
                ),
                0.0,
                STAR,
            )?;
        }
        Ok(())
    }
}

impl App for Game {
    fn init(window: Window, _abi_minor: i32) -> Result<Self, VyError> {
        let style = |font, size, weight, color| text_style(font, size, weight, color);
        let _ = log(Level::Debug, "game: init");
        let mut game = Self {
            window,
            title: style(Font::Sans, 18.0, 700, HUD)?,
            hud: style(Font::Mono, 15.0, 400, HUD)?,
            hint: style(Font::Sans, 14.0, 400, HUD_MUTED)?,
            big: style(Font::Sans, 32.0, 700, HUD)?,
            phase: Phase::Waiting,
            paddle_x: window.width / 2.0,
            ball: (0.0, 0.0),
            velocity: (0.0, 0.0),
            trail: [(0.0, 0.0); TRAIL],
            trail_len: 0,
            score: 0,
            best: 0,
            elapsed_ms: 0,
            low_detail: false,
        };
        game.clamp_paddle();
        game.rest_ball();
        Ok(game)
    }

    fn frame(&mut self, f: &mut Frame, dt_ms: i32) -> Result<(), VyError> {
        if self.phase == Phase::Running {
            self.step(dt_ms);
        }
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        let court = self.court();
        f.rect(window, 0.0, PAGE)?;
        f.group(Role::Main, window, "", |f| {
            // HUD: title left, score and clock right.
            let h = f.text(MARGIN, 14.0, 200.0, self.title, "Studs")?.h;
            f.sem
                .node(Role::Heading, Rect::new(MARGIN, 14.0, 200.0, h), "Studs")?;
            let hud = format!(
                "Poäng {}  Bästa {}  {},{} s",
                self.score,
                self.best,
                self.elapsed_ms / 1000,
                (self.elapsed_ms % 1000) / 100
            );
            let hw = text_measure(self.hud, 0.0, &hud)?.w;
            let hx = self.window.width - MARGIN - hw;
            let hh = f.text(hx, 15.0, 0.0, self.hud, &hud)?.h;
            f.sem.node(Role::Text, Rect::new(hx, 15.0, hw, hh), &hud)?;

            f.rect(court, 6.0, BORDER)?;
            f.rect(court.inset(2.0), 4.0, COURT)?;
            f.clipped(court.inset(2.0), 0.0, |f| {
                if !self.low_detail {
                    self.draw_stars(f)?;
                    for (i, &(x, y)) in self.trail[..self.trail_len].iter().enumerate() {
                        let a = (20 + i * 10) as u8;
                        let r = BALL_R * (0.4 + 0.5 * i as f32 / TRAIL as f32);
                        f.rect(
                            Rect::new(x - r, y - r, 2.0 * r, 2.0 * r),
                            r,
                            BALL.with_alpha(a),
                        )?;
                    }
                }
                let paddle = self.paddle();
                f.rect(paddle, PADDLE_H / 2.0, PADDLE)?;
                let (x, y) = self.ball;
                f.rect(
                    Rect::new(x - BALL_R, y - BALL_R, 2.0 * BALL_R, 2.0 * BALL_R),
                    BALL_R,
                    BALL,
                )?;
                Ok(())
            })?;

            if let Some((title, hint)) = self.message() {
                f.rect(court.inset(2.0), 4.0, OVERLAY)?;
                let tw = text_measure(self.big, 0.0, title)?.w;
                let ty = court.y + court.h * 0.4;
                let th = f
                    .text(court.x + (court.w - tw) / 2.0, ty, 0.0, self.big, title)?
                    .h;
                let hw = text_measure(self.hint, 0.0, hint)?.w;
                let hy = ty + th + 8.0;
                let hh = f
                    .text(court.x + (court.w - hw) / 2.0, hy, 0.0, self.hint, hint)?
                    .h;
                let status = format!("{title}. {hint}");
                f.sem.node(
                    Role::Text,
                    Rect::new(
                        court.x + (court.w - tw.max(hw)) / 2.0,
                        ty,
                        tw.max(hw),
                        th + 8.0 + hh,
                    ),
                    &status,
                )?;
            }
            Ok(())
        })?;
        if self.phase == Phase::Running {
            request_frame();
        }
        Ok(())
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        match event {
            Event::KeyDown { code, repeat, .. } => match (code, self.phase) {
                (key::SPACE, Phase::Waiting | Phase::Lost) if !repeat => {
                    self.serve();
                    true
                }
                (key::SPACE, Phase::Paused) if !repeat => {
                    self.phase = Phase::Running;
                    true
                }
                (key::ESCAPE, Phase::Running) => {
                    self.phase = Phase::Paused;
                    true
                }
                // While the ball is in play the frame polls the held keys.
                (key::LEFT, Phase::Running) | (key::RIGHT, Phase::Running) => false,
                (key::LEFT, _) => self.move_paddle(-PADDLE_STEP),
                (key::RIGHT, _) => self.move_paddle(PADDLE_STEP),
                _ => false,
            },
            Event::Visibility { visible } => {
                if !visible && self.phase == Phase::Running {
                    self.phase = Phase::Paused;
                    let _ = log(Level::Info, "game: hidden, paused");
                    false
                } else {
                    visible && self.phase == Phase::Paused
                }
            }
            Event::Budget { used_ms, .. } => {
                if !self.low_detail {
                    self.low_detail = true;
                    let _ = log(
                        Level::Warn,
                        &format!("game: {used_ms} ms frame, dropping stars and trail"),
                    );
                }
                true
            }
            _ => false,
        }
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
        self.clamp_paddle();
        if self.phase != Phase::Running {
            self.rest_ball();
        }
    }

    /// Hidden by the host (SPEC-5.1.5): the round waits for Space.
    fn suspend(&mut self) -> i32 {
        if self.phase == Phase::Running {
            self.phase = Phase::Paused;
        }
        0
    }
}

vy_sdk::export!(Game);

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use vy_sdk::mock;
    use vy_sdk::mock::Op;

    fn game() -> Game {
        mock::reset();
        Game::init(
            Window {
                width: 800.0,
                height: 600.0,
                scale: 1.0,
            },
            0,
        )
        .unwrap()
    }

    fn press(g: &mut Game, code: i32) -> bool {
        g.event(Event::KeyDown {
            code,
            modifiers: Modifiers(0),
            repeat: false,
        })
    }

    fn run_frame(g: &mut Game) -> usize {
        mock::run_frame(g, 16).unwrap();
        mock::with(|h| h.frame_ops().len())
    }

    #[test]
    fn waiting_costs_no_frames_running_requests_one_per_frame() {
        let mut g = game();
        run_frame(&mut g);
        assert_eq!(mock::with(|h| h.frame_requests), 0);
        assert!(press(&mut g, key::SPACE));
        assert_eq!(g.phase, Phase::Running);
        for _ in 0..5 {
            run_frame(&mut g);
        }
        assert_eq!(mock::with(|h| h.frame_requests), 5);
        assert_eq!(g.elapsed_ms, 80);
        assert!(g.ball.1 < g.paddle().y - BALL_R, "the ball left the paddle");
    }

    #[test]
    fn same_seed_same_positions() {
        let a = {
            let mut g = game();
            press(&mut g, key::SPACE);
            for _ in 0..30 {
                run_frame(&mut g);
            }
            (g.ball, g.velocity)
        };
        let b = {
            let mut g = game();
            press(&mut g, key::SPACE);
            for _ in 0..30 {
                run_frame(&mut g);
            }
            (g.ball, g.velocity)
        };
        assert_eq!(a, b);
    }

    #[test]
    fn held_arrow_moves_paddle_while_running() {
        let mut g = game();
        let start = g.paddle_x;
        assert!(press(&mut g, key::LEFT), "a step while waiting redraws");
        assert_eq!(g.paddle_x, start - PADDLE_STEP);
        press(&mut g, key::SPACE);
        assert!(!press(&mut g, key::LEFT), "in play the frame polls instead");
        mock::with(|h| h.keys_down.push(key::LEFT));
        run_frame(&mut g);
        let expected = start - PADDLE_STEP - PADDLE_SPEED * 0.016;
        assert!((g.paddle_x - expected).abs() < 0.01, "{}", g.paddle_x);
    }

    #[test]
    fn visibility_and_suspend_pause_budget_reduces_work() {
        let mut g = game();
        press(&mut g, key::SPACE);
        assert!(
            !g.event(Event::Visibility { visible: false }),
            "no redraw hidden"
        );
        assert_eq!(g.phase, Phase::Paused);
        assert!(g.event(Event::Visibility { visible: true }), "overlay");
        let ops_paused = run_frame(&mut g);
        assert_eq!(mock::with(|h| h.frame_requests), 0);
        assert!(press(&mut g, key::SPACE));
        assert_eq!(g.phase, Phase::Running);
        assert_eq!(g.suspend(), 0);
        assert_eq!(g.phase, Phase::Paused);
        press(&mut g, key::SPACE);
        let full = run_frame(&mut g);
        assert!(g.event(Event::Budget {
            used_ms: 9,
            allowed_ms: 8,
            last_warning: false,
        }));
        let reduced = run_frame(&mut g);
        assert!(reduced + STARS <= full, "{full} -> {reduced}");
        assert!(ops_paused > 0);
        mock::with(|h| {
            assert!(h.logs.iter().any(|(_, m)| m.contains("dropping")));
            assert!(h.refusals.is_empty(), "{:?}", h.refusals);
            assert!(h
                .frame_ops()
                .iter()
                .all(|op| !matches!(op, Op::Rect { color, .. } if *color == STAR.0)));
        });
    }

    #[test]
    fn a_miss_ends_the_round() {
        let mut g = game();
        press(&mut g, key::SPACE);
        // Park the paddle at the far left; the serve goes elsewhere or
        // bounces until it does not.
        mock::with(|h| h.keys_down.push(key::LEFT));
        for _ in 0..600 {
            if g.phase != Phase::Running {
                break;
            }
            run_frame(&mut g);
        }
        assert_eq!(g.phase, Phase::Lost);
        assert!(press(&mut g, key::SPACE));
        assert_eq!(g.phase, Phase::Running);
        assert_eq!(g.score, 0);
    }
}
