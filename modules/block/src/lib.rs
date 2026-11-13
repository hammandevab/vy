//! Vy example page `block` ("Block"): a falling-blocks puzzle game.
//!
//! What it shows beyond the reference pages:
//!
//! - **A game loop in fixed steps.** While the game is on, every frame
//!   asks for the next with `request_frame` (SPEC-5.2.2) and adds its
//!   `dt_ms` (SPEC-5.1.2) to a balance that pays for ticks of 16 ms; the
//!   rules (`game.rs`) only ever see whole ticks, so the same events at
//!   the same ticks give the same game at any frame rate. Paused, over or
//!   not started, the page asks for nothing (SPEC-6.2.2).
//! - **Chance from the host.** The order of the pieces comes from
//!   `random` (SPEC-5.2.11) through a seven-piece bag.
//! - **Pause when hidden.** `VISIBILITY` hidden and `vy_suspend` pause the
//!   game (SPEC-A.2.1, SPEC-5.1.5).
//! - **Keys and pointer alike.** Arrows, Space, Z, X, C and P play it; so
//!   do the on-screen buttons, which repeat while they are held and are
//!   `button` nodes the keyboard can reach and press with Enter.
//! - **The game in the tree.** Score, level, lines and the state are text
//!   nodes, the well an `image` node that says what is in it.
//! - **A stored high score** (SPEC-5.2.9); without the capability the
//!   page says "sparas inte".
#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod game;

use alloc::string::String;
use game::{Game, Piece, Shape, State, CLEAR_TICKS, COLS, ROWS, TICK_MS};
use vy_sdk::event::{button, key};
use vy_sdk::math::{floor, snap};
use vy_sdk::prelude::*;
use vy_sdk::{state_get, state_put};

const PAGE: Color = Color::rgb(0x0B, 0x10, 0x1C);
const WELL: Color = Color::rgb(0x11, 0x18, 0x29);
const EDGE: Color = Color::rgb(0x33, 0x41, 0x55);
const GRID: Color = Color::rgb(0x1A, 0x24, 0x3A);
const PANEL: Color = Color::rgb(0x17, 0x20, 0x33);
const INK: Color = Color::rgb(0xE6, 0xEA, 0xF2);
const MUTED: Color = Color::rgb(0x94, 0xA3, 0xB8);
const ACCENT: Color = Color::rgb(0x4F, 0x7C, 0xFF);
const RING: Color = Color::rgb(0xF2, 0x9F, 0x05);
const SCRIM: Color = Color::rgba(0x0B, 0x10, 0x1C, 0xC8);
const FLASH: Color = Color::rgb(0xFF, 0xFF, 0xFF);

/// Each shape's colour and the lighter one of its top edge.
const TILES: [(Color, Color); 7] = [
    (Color::rgb(0x22, 0xB8, 0xCF), Color::rgb(0x7D, 0xDD, 0xEA)),
    (Color::rgb(0xE5, 0xB1, 0x0A), Color::rgb(0xF5, 0xD6, 0x6B)),
    (Color::rgb(0xA4, 0x58, 0xF0), Color::rgb(0xCD, 0xA2, 0xF8)),
    (Color::rgb(0x2D, 0xB8, 0x5C), Color::rgb(0x86, 0xDD, 0xA3)),
    (Color::rgb(0xE5, 0x48, 0x4D), Color::rgb(0xF3, 0x99, 0x9C)),
    (Color::rgb(0x3F, 0x7F, 0xF2), Color::rgb(0x93, 0xB7, 0xF8)),
    (Color::rgb(0xF0, 0x7F, 0x23), Color::rgb(0xF8, 0xB8, 0x82)),
];

/// The on-screen keys, in Tab order. Key `i` has node id `ID_KEY - i`.
const KEYS: usize = 7;
const K_LEFT: usize = 0;
const K_RIGHT: usize = 1;
const K_TURN: usize = 2;
const K_DOWN: usize = 3;
const K_DROP: usize = 4;
const K_HOLD: usize = 5;
const K_MAIN: usize = 6;
const ID_KEY: i32 = -10;
const NAMES: [&str; KEYS] = ["Vänster", "Höger", "Rotera", "Ned", "Släpp", "Håll", ""];
const GLYPHS: [&str; KEYS] = ["←", "→", "↻", "↓", "Släpp", "Håll", ""];

const MARGIN: f32 = 16.0;
const HEADER_H: f32 = 52.0;
/// Below this width the keys are a row under the well.
const NARROW: f32 = 620.0;
/// Ticks a held key waits before it repeats, and between repeats.
const REPEAT_AFTER: u32 = 12;
const REPEAT_EVERY: u32 = 3;
/// Most ticks one frame catches up.
const MAX_TICKS: u32 = 8;
const KEY_C: i32 = 0x06;
const KEY_H: i32 = 0x0B;
const KEY_P: i32 = 0x13;
const KEY_X: i32 = 0x1B;
const KEY_Z: i32 = 0x1D;
const STORE: &str = "block";

#[derive(Clone, Copy)]
struct Styles {
    title: StyleId,
    label: StyleId,
    value: StyleId,
    key: StyleId,
    glyph: StyleId,
    main: StyleId,
    big: StyleId,
    hint: StyleId,
}

/// Every rectangle on the page for the current window.
#[derive(Clone, Copy, Debug)]
struct Layout {
    narrow: bool,
    /// The well and the side of one of its cells.
    well: Rect,
    cell: f32,
    /// The column beside the well.
    side: Rect,
    next: Rect,
    hold: Rect,
    stats: Rect,
    keys: [Rect; KEYS],
}

struct Page {
    window: Window,
    s: Styles,
    game: Game,
    /// Milliseconds of frame time not yet spent on ticks.
    balance: i32,
    /// The game just started or went on: the next frame's `dt_ms` is
    /// time it stood still, not time to play.
    resumed: bool,
    /// The best score, and whether it could be read and written.
    best: u32,
    stored: bool,
    /// The on-screen key the pointer holds, and for how many ticks.
    held: Option<(usize, u32)>,
    /// The node the host's focus is on.
    focus: Option<i32>,
}

/// The parts joined; the page builds its few strings without `core::fmt`.
fn cat(parts: &[&str]) -> String {
    let mut s = String::new();
    for part in parts {
        s.push_str(part);
    }
    s
}

/// `n` in decimal.
fn number(n: u32) -> String {
    let mut s = String::new();
    vy_sdk::num::push_int(&mut s, n as i64);
    s
}

impl Page {
    fn layout(&self) -> Layout {
        let (w, h) = (self.window.width, self.window.height);
        let narrow = w < NARROW;
        let mut keys = [Rect::default(); KEYS];
        let (side_w, gap, below) = if narrow {
            (112.0, 12.0, 48.0 + 12.0)
        } else {
            (220.0, 16.0, 0.0)
        };
        let room_h = h - HEADER_H - MARGIN - below;
        let room_w = w - 2.0 * MARGIN - gap - side_w;
        let cell = floor((room_h / ROWS as f32).min(room_w / COLS as f32)).clamp(8.0, 44.0);
        let well_w = COLS as f32 * cell;
        let well_h = ROWS as f32 * cell;
        let scale = self.window.scale;
        let x0 = snap(((w - (well_w + gap + side_w)) / 2.0).max(MARGIN), scale);
        let well = Rect::new(x0, HEADER_H, well_w, well_h);
        let side = Rect::new(well.right() + gap, HEADER_H, side_w, well_h);

        let box_w = side_w.min(132.0);
        let box_h = if narrow { 58.0 } else { 72.0 };
        let next = Rect::new(side.x, side.y + 20.0, box_w, box_h);
        let hold = Rect::new(side.x, next.bottom() + 30.0, box_w, box_h);
        let stats = Rect::new(side.x, hold.bottom() + 14.0, side_w, 4.0 * 24.0);
        if narrow {
            let y = well.bottom() + 12.0;
            let all = (w - 2.0 * MARGIN).max(0.0);
            // The arrows narrower than the keys with words, the main
            // key widest: 4 + 2 × 1.55 + 2.3 units.
            let unit = (all - 6.0 * 6.0) / 9.4;
            let mut x = MARGIN;
            for (i, key) in keys.iter_mut().enumerate() {
                let kw = match i {
                    K_MAIN => unit * 2.3,
                    i if i >= K_DROP => unit * 1.55,
                    _ => unit,
                };
                *key = Rect::new(x, y, kw, 48.0);
                x += kw + 6.0;
            }
        } else {
            let third = (side_w - 16.0) / 3.0;
            let half = (side_w - 8.0) / 2.0;
            let y = side.bottom() - 3.0 * 40.0 - 2.0 * 8.0;
            keys[K_LEFT] = Rect::new(side.x, y, third, 40.0);
            keys[K_TURN] = Rect::new(side.x + third + 8.0, y, third, 40.0);
            keys[K_RIGHT] = Rect::new(side.x + 2.0 * (third + 8.0), y, third, 40.0);
            keys[K_DOWN] = Rect::new(side.x, y + 48.0, half, 40.0);
            keys[K_DROP] = Rect::new(side.x + half + 8.0, y + 48.0, half, 40.0);
            keys[K_HOLD] = Rect::new(side.x, y + 96.0, half, 40.0);
            keys[K_MAIN] = Rect::new(side.x + half + 8.0, y + 96.0, half, 40.0);
        }
        Layout {
            narrow,
            well,
            cell,
            side,
            next,
            hold,
            stats,
            keys,
        }
    }

    /// What the main key does now, which is also its label.
    fn main_label(&self) -> &'static str {
        match self.game.state {
            State::Ready => "Starta",
            State::Playing => "Paus",
            State::Paused => "Fortsätt",
            State::Over => "Börja om",
        }
    }

    fn state_text(&self) -> &'static str {
        match self.game.state {
            State::Ready => "Inte startat. Tryck Starta eller Enter.",
            State::Playing => "Spelet pågår.",
            State::Paused => "Pausat. Tryck Fortsätt eller P.",
            State::Over => "Spelet är slut. Tryck Börja om eller Enter.",
        }
    }

    /// Keep the best score: a finished game that beat it is stored.
    fn settle(&mut self) {
        if self.game.state == State::Over && self.game.score > self.best {
            self.best = self.game.score;
            self.save();
        }
    }

    fn save(&mut self) {
        let result = state_put(STORE, &self.best.to_le_bytes());
        if let Err(e) = result {
            let _ = log(Level::Warn, &cat(&["block: state_put refused: ", e.name()]));
        }
        self.stored = result.is_ok();
    }

    /// What on-screen key `k` does; true when something changed.
    fn act(&mut self, k: usize) -> bool {
        match k {
            K_LEFT => self.game.shift(-1),
            K_RIGHT => self.game.shift(1),
            K_TURN => self.game.rotate(true),
            K_DOWN => {
                self.game.soft = self.game.state == State::Playing;
                self.game.soft
            }
            K_DROP => self.game.hard_drop(),
            K_HOLD => self.game.hold(),
            _ => {
                match self.game.state {
                    State::Ready | State::Over => self.game.start(),
                    State::Playing => self.game.pause(true),
                    State::Paused => self.game.pause(false),
                }
                self.rested();
                true
            }
        }
    }

    /// The game stood still: time starts over with the next frame.
    fn rested(&mut self) {
        self.balance = 0;
        self.resumed = true;
    }

    /// The pointer let go of the key it held.
    fn release(&mut self) -> bool {
        match self.held.take() {
            Some((K_DOWN, _)) => {
                self.game.soft = false;
                false
            }
            Some(_) => true,
            None => false,
        }
    }

    /// One tick: a held arrow repeats, then the game moves on.
    fn tick(&mut self) {
        let again = self.held.as_mut().and_then(|(k, ticks)| {
            *ticks += 1;
            let due = *ticks > REPEAT_AFTER && (*ticks - REPEAT_AFTER).is_multiple_of(REPEAT_EVERY);
            (due && (*k == K_LEFT || *k == K_RIGHT)).then_some(*k)
        });
        if let Some(k) = again {
            self.act(k);
        }
        self.game.tick();
    }

    fn key_event(&mut self, code: i32, repeat: bool) -> bool {
        let playing = self.game.state == State::Playing;
        let on_key = self
            .focus
            .map(|id| (ID_KEY - id) as usize)
            .filter(|k| *k < KEYS);
        match code {
            // Enter presses the focused key; otherwise it starts a game.
            key::ENTER if repeat => false,
            key::ENTER => match on_key {
                Some(k) => self.act(k),
                None if !playing && self.game.state != State::Paused => self.act(K_MAIN),
                None => false,
            },
            // Space is the drop while the game is on, whatever has focus.
            key::SPACE if playing => !repeat && self.game.hard_drop(),
            key::SPACE => !repeat && on_key.is_some_and(|k| self.act(k)),
            key::LEFT => self.game.shift(-1),
            key::RIGHT => self.game.shift(1),
            key::UP | KEY_X => !repeat && self.game.rotate(true),
            KEY_Z => !repeat && self.game.rotate(false),
            key::DOWN => {
                self.game.soft = playing;
                false
            }
            KEY_C | KEY_H => !repeat && self.game.hold(),
            KEY_P | key::ESCAPE if !repeat && self.game.state != State::Ready => {
                self.game.pause(playing);
                self.rested();
                self.game.state != State::Over
            }
            _ => false,
        }
    }

    /// One cell of a piece at `r`: the tile and its lighter top edge.
    fn tile(f: &mut Frame, r: Rect, shape: usize) -> Result<(), VyError> {
        let (fill, light) = TILES[shape % TILES.len()];
        let inner = r.inset(1.0);
        let radius = (r.w / 8.0).min(4.0);
        f.rect(inner, radius, fill)?;
        let edge = (r.w / 7.0).max(2.0);
        f.rect(
            Rect::new(inner.x + edge, inner.y + edge, inner.w - 2.0 * edge, edge),
            edge / 2.0,
            light,
        )
    }

    /// A shape drawn small in the middle of `r`.
    fn mini(f: &mut Frame, r: Rect, shape: Shape) -> Result<(), VyError> {
        let cells = shape.cells(0);
        let (x0, x1) = cells
            .iter()
            .fold((9, 0), |(a, b), c| (a.min(c.0), b.max(c.0)));
        let (y0, y1) = cells
            .iter()
            .fold((9, 0), |(a, b), c| (a.min(c.1), b.max(c.1)));
        let size = floor((r.w - 16.0) / 4.0)
            .min(floor((r.h - 16.0) / 2.0))
            .max(4.0);
        let w = f32::from(x1 - x0 + 1) * size;
        let h = f32::from(y1 - y0 + 1) * size;
        let (ox, oy) = (r.x + (r.w - w) / 2.0, r.y + (r.h - h) / 2.0);
        for (cx, cy) in cells {
            let cell = Rect::new(
                ox + f32::from(cx - x0) * size,
                oy + f32::from(cy - y0) * size,
                size,
                size,
            );
            Self::tile(f, cell, shape as usize)?;
        }
        Ok(())
    }

    fn draw_well(&self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let g = &self.game;
        let c = l.cell;
        let line = snap(1.0, self.window.scale);
        f.rect(l.well.inset(-2.0), 6.0, EDGE)?;
        f.rect(l.well, 4.0, WELL)?;
        for col in 1..COLS {
            let x = snap(l.well.x + col as f32 * c, self.window.scale);
            f.rect(Rect::new(x, l.well.y, line, l.well.h), 0.0, GRID)?;
        }
        for row in 1..ROWS {
            let y = snap(l.well.y + row as f32 * c, self.window.scale);
            f.rect(Rect::new(l.well.x, y, l.well.w, line), 0.0, GRID)?;
        }
        let at = |x: i8, y: i8| {
            Rect::new(
                l.well.x + f32::from(x) * c,
                l.well.y + f32::from(y) * c,
                c,
                c,
            )
        };
        for (row, cells) in g.well.iter().enumerate() {
            for (col, cell) in cells.iter().enumerate() {
                if *cell != 0 {
                    Self::tile(f, at(col as i8, row as i8), usize::from(*cell) - 1)?;
                }
            }
        }
        // Where the piece will land, then the piece.
        if let (Some(p), Some(ghost)) = (g.piece, g.landing()) {
            let (fill, _) = TILES[p.shape as usize];
            if ghost != p {
                for (x, y) in ghost.cells() {
                    f.rect(at(x, y).inset(1.0), 3.0, fill.with_alpha(0x38))?;
                }
            }
            for (x, y) in p.cells().into_iter().filter(|c| c.1 >= 0) {
                Self::tile(f, at(x, y), p.shape as usize)?;
            }
        }
        // Full rows flash before they go.
        if let Some((full, left)) = g.clearing {
            let alpha = (left * 220 / CLEAR_TICKS) as u8;
            for row in (0..ROWS).filter(|r| full & 1 << r != 0) {
                let r = Rect::new(l.well.x, l.well.y + row as f32 * c, l.well.w, c);
                f.rect(r, 0.0, FLASH.with_alpha(alpha))?;
            }
        }
        let falling = g.piece.map_or(String::new(), |p: Piece| {
            cat(&[", faller: ", p.shape.name()])
        });
        let text = cat(&[
            "Spelplan 10 × 20, ",
            &number(g.filled() as u32),
            " rutor fyllda",
            &falling,
        ]);
        f.sem.node(Role::Image, l.well, &text)?;

        // What the game is waiting for, over the well.
        let (big, hint) = match g.state {
            State::Playing => return Ok(()),
            State::Ready => ("Block", "Enter eller Starta"),
            State::Paused => ("Pausat", "P eller Fortsätt"),
            State::Over => ("Slut", "Enter eller Börja om"),
        };
        f.rect(l.well, 4.0, SCRIM)?;
        let size = text_measure(self.s.big, 0.0, big)?;
        let y = l.well.y + l.well.h * 0.38;
        f.text(
            l.well.x + (l.well.w - size.w) / 2.0,
            y,
            0.0,
            self.s.big,
            big,
        )?;
        let small = text_measure(self.s.hint, l.well.w - 16.0, hint)?;
        let hint_x = l.well.x + (l.well.w - small.w) / 2.0;
        f.text(hint_x, y + size.h + 6.0, l.well.w - 16.0, self.s.hint, hint)?;
        Ok(())
    }

    fn draw_side(&self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let g = &self.game;
        let boxes = [("Nästa", l.next, Some(g.next)), ("Håll", l.hold, g.hold)];
        for (label, r, shape) in boxes {
            f.text(r.x, r.y - 19.0, 0.0, self.s.label, label)?;
            f.rect(r, 8.0, PANEL)?;
            if let Some(shape) = shape {
                Self::mini(f, r, shape)?;
            }
            let what = shape.map_or("tom", Shape::name);
            let area = Rect::new(r.x, r.y - 19.0, r.w, r.h + 19.0);
            f.sem.node(Role::Text, area, &cat(&[label, ": ", what]))?;
        }
        let best = if self.stored { "" } else { " (sparas inte)" };
        let rows = [
            ("Poäng", number(g.score), ""),
            ("Nivå", number(g.level()), ""),
            ("Rader", number(g.lines), ""),
            ("Bästa", number(self.best.max(g.score)), best),
        ];
        for (i, (label, value, note)) in rows.iter().enumerate() {
            let (label, note) = (*label, *note);
            let y = l.stats.y + i as f32 * 24.0;
            f.text(l.stats.x, y + 3.0, 0.0, self.s.label, label)?;
            let w = text_measure(self.s.value, 0.0, value)?.w;
            let right = l.stats.x + l.next.w;
            f.text(right - w, y, 0.0, self.s.value, value)?;
            if !note.is_empty() && !l.narrow {
                f.text(right + 6.0, y + 3.0, 0.0, self.s.label, note)?;
            }
            let line = Rect::new(l.stats.x, y, l.stats.w, 24.0);
            let text = cat(&[label, " ", value.as_str(), note]);
            f.sem.node(Role::Text, line, &text)?;
        }
        Ok(())
    }

    fn draw_keys(&self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        for (i, r) in l.keys.iter().enumerate() {
            let id = ID_KEY - i as i32;
            if self.focus == Some(id) {
                f.rect(r.inset(-3.0), 11.0, RING)?;
            }
            let down = self.held.is_some_and(|h| h.0 == i);
            let main = i == K_MAIN;
            f.rect(*r, 8.0, if main { ACCENT } else { EDGE })?;
            if !main {
                f.rect(r.inset(1.0), 7.0, if down { EDGE } else { PANEL })?;
            }
            let (label, name) = if main {
                (self.main_label(), self.main_label())
            } else {
                (GLYPHS[i], NAMES[i])
            };
            let style = match i {
                K_MAIN => self.s.main,
                i if i < K_DROP => self.s.glyph,
                _ => self.s.key,
            };
            f.text_centred(*r, style, label)?;
            f.sem.node_with_id(id, Role::Button, *r, name)?;
        }
        Ok(())
    }
}

impl App for Page {
    fn init(window: Window, _abi_minor: i32) -> Result<Self, VyError> {
        let mut bytes = [0u8; 4];
        let (best, stored) = match state_get(STORE, &mut bytes) {
            Ok(4) => (u32::from_le_bytes(bytes), true),
            Ok(_) | Err(VyError::Resource) => (0, true),
            Err(e) => {
                let _ = log(Level::Info, &cat(&["block: no storage: ", e.name()]));
                (0, false)
            }
        };
        Ok(Self {
            window,
            s: Styles {
                title: text_style(Font::Sans, 20.0, 700, INK)?,
                label: text_style(Font::Sans, 13.0, 400, MUTED)?,
                value: text_style(Font::Mono, 17.0, 400, INK)?,
                key: text_style(Font::Sans, 14.0, 600, INK)?,
                glyph: text_style(Font::Sans, 22.0, 400, INK)?,
                main: text_style(Font::Sans, 14.0, 700, FLASH)?,
                big: text_style(Font::Sans, 28.0, 700, INK)?,
                hint: text_style(Font::Sans, 13.0, 400, MUTED)?,
            },
            game: Game::new(),
            balance: 0,
            resumed: true,
            best,
            stored,
            held: None,
            focus: None,
        })
    }

    fn frame(&mut self, f: &mut Frame, dt_ms: i32) -> Result<(), VyError> {
        if self.game.state == State::Playing {
            // The frame's time pays for whole ticks; a long gap is not
            // caught up beyond a few.
            if !core::mem::take(&mut self.resumed) {
                self.balance += dt_ms.max(0);
            }
            let mut ticks = 0;
            while self.balance >= TICK_MS && ticks < MAX_TICKS {
                self.balance -= TICK_MS;
                ticks += 1;
                self.tick();
            }
            if ticks == MAX_TICKS {
                self.balance = 0;
            }
            self.settle();
        }
        if self.game.state == State::Playing {
            request_frame();
        }
        let l = self.layout();
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        f.rect(window, 0.0, PAGE)?;
        f.group(Role::Main, window, "", |f| {
            let h = f.text(l.well.x, 14.0, 0.0, self.s.title, "Block")?.h;
            let title = Rect::new(l.well.x, 14.0, 80.0, h);
            f.sem.node(Role::Heading, title, "Block")?;
            // The state in words, right-aligned in the header.
            let state = self.state_text();
            let room = (l.side.right() - l.well.x - 90.0).max(60.0);
            let size = text_measure(self.s.label, room, state)?;
            let x = l.side.right() - size.w;
            f.text(x, 12.0, room, self.s.label, state)?;
            f.sem
                .node(Role::Text, Rect::new(x, 12.0, size.w, size.h), state)?;
            self.draw_well(f, &l)?;
            self.draw_side(f, &l)?;
            self.draw_keys(f, &l)
        })
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        let l = self.layout();
        let redraw = match event {
            Event::Focus { node, gained, .. } => {
                if gained {
                    self.focus = Some(node.0);
                } else if self.focus == Some(node.0) {
                    self.focus = None;
                }
                true
            }
            Event::KeyDown {
                code,
                modifiers,
                repeat,
            } if !modifiers.shortcut() => self.key_event(code, repeat),
            Event::KeyUp {
                code: key::DOWN, ..
            } => {
                self.game.soft = false;
                false
            }
            Event::PointerDown { x, y, button, .. } if button == button::PRIMARY => {
                match l.keys.iter().position(|r| r.contains(x, y)) {
                    Some(k) => {
                        self.act(k);
                        self.held = Some((k, 0));
                        true
                    }
                    None => false,
                }
            }
            Event::PointerMove { buttons, .. } if buttons & button::mask(button::PRIMARY) == 0 => {
                self.release()
            }
            Event::PointerUp { .. } | Event::PointerLeave => self.release(),
            // A hidden page plays no game; shown again it stays paused.
            Event::Visibility { visible } => {
                if !visible {
                    self.game.pause(true);
                    self.held = None;
                }
                visible
            }
            _ => false,
        };
        self.settle();
        redraw
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
    }

    /// Hidden by the host (SPEC-5.1.5): the game waits.
    fn suspend(&mut self) -> i32 {
        self.game.pause(true);
        self.held = None;
        0
    }
}

vy_sdk::export!(Page);

#[cfg(test)]
mod tests;
