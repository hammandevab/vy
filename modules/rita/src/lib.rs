//! Vy example page `rita` ("Ritbord"): a freehand drawing canvas.
//!
//! What it shows beyond the reference pages:
//!
//! - **Strokes as `PATH` ops.** A pointer drag collects points; on release
//!   the stroke is simplified (Ramer–Douglas–Peucker, `ink.rs`) and from
//!   then on drawn as one stroked path: quadratic curves where its points
//!   bend, straight lines to the corners, and a dot at each end for round
//!   caps (`dl_path` has no cap style).
//! - **A drawing that stays inside the budgets.** Every frame sends every
//!   stroke again, so the drawing is capped at 2 500 points and at a sum
//!   of length × width (SPEC-5.3.3, SPEC-9.3.2: far below 65 535 commands
//!   a path and 65 536 ops a frame), and a stroke's path is built once
//!   and kept until the drawing or the window changes. At the cap the
//!   page refuses new strokes with a message on the paper; nothing fails.
//! - **Tools as buttons with their state in the text** (a node has a role
//!   and a text, SPEC-5.2.13): eight colours, three pens, an eraser that
//!   removes the strokes it touches, undo and redo (also Ctrl+Z and
//!   Ctrl+Y), and clear, which asks once in a `dialog` with two
//!   `ui::Button`s. While the dialog is open the tools leave the tree, so
//!   Tab stays in the dialog (SPEC-10.3.1).
//! - **A paper of fixed size** scaled to the window, so a drawing made in
//!   a wide window is whole in a narrow one.
//! - **Storage** of the strokes after each change, a few bytes a point
//!   (SPEC-5.2.9); without the capability the page says "sparas inte".
#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod ink;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use ink::{
    curve, decode, distance, encode, Point, Stroke, MAX_INK, MAX_POINTS, PAPER_H, PAPER_W,
    TOLERANCE_PX, WIDTHS,
};
use vy_sdk::event::{button, key};
use vy_sdk::math::snap;
use vy_sdk::prelude::*;
use vy_sdk::ui::{self, Button, Input, Palette, Theme};
use vy_sdk::{state_get, state_put};

const PAGE: Color = Color::rgb(0xE9, 0xEB, 0xF0);
const PAPER: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const PAPER_EDGE: Color = Color::rgb(0xC9, 0xCD, 0xD6);
const SCRIM: Color = Color::rgba(0x10, 0x18, 0x30, 0x70);
const RUBBER: Color = Color::rgba(0x10, 0x18, 0x30, 0x30);
const NOTICE: Color = Color::rgba(0x1C, 0x1E, 0x26, 0xE6);

/// The eight colours and what the tree calls them.
const COLORS: [(Color, &str); 8] = [
    (Color::rgb(0x1C, 0x1E, 0x26), "svart"),
    (Color::rgb(0xD9, 0x37, 0x2B), "röd"),
    (Color::rgb(0xE8, 0x87, 0x2B), "orange"),
    (Color::rgb(0xE9, 0xC2, 0x2F), "gul"),
    (Color::rgb(0x2E, 0x9E, 0x5B), "grön"),
    (Color::rgb(0x2F, 0x5B, 0xD6), "blå"),
    (Color::rgb(0x7B, 0x4B, 0xC9), "lila"),
    (Color::rgb(0xE0, 0x5F, 0xA0), "rosa"),
];
const PENS: [&str; 3] = ["tunn", "mellan", "bred"];

/// The toolbar's keys, in Tab order: colours, pens, eraser, undo, redo,
/// clear. Key `i` has node id `ID_KEY - i`.
const KEYS: usize = 15;
const K_PEN: usize = 8;
const K_ERASER: usize = 11;
const K_UNDO: usize = 12;
const K_REDO: usize = 13;
const K_CLEAR: usize = 14;
const ID_KEY: i32 = -10;
const ID_YES: i32 = -40;
const ID_NO: i32 = -41;

const MARGIN: f32 = 16.0;
const HEADER_H: f32 = 48.0;
const KEY_H: f32 = 32.0;
const GAP: f32 = 6.0;
/// One row of tools from this width, two below.
const WIDE: f32 = 760.0;
/// A stroke that has collected this many points is ended and continued
/// in a new one: as many as a stroke may keep, so none is ever dropped.
const RAW_MAX: usize = ink::MAX_STROKE_POINTS;
/// The eraser's reach in window pixels.
const RUBBER_R: f32 = 10.0;
/// Steps kept for undo.
const UNDO_MAX: usize = 200;
const KEY_Z: i32 = 0x1D;
const KEY_Y: i32 = 0x1C;
const STORE: &str = "rita";

/// One step of the history, in the form that takes it back (on the undo
/// stack) or brings it again (on the redo stack).
#[derive(Clone, Debug, PartialEq)]
enum Step {
    /// Undo: the last stroke was drawn; take it away.
    Drawn,
    /// Redo: draw this stroke again.
    Draw(Stroke),
    /// Undo: these strokes were erased from these places; put them back.
    Erased(Vec<(usize, Stroke)>),
    /// Redo: erase the strokes at these places again.
    Erase(Vec<usize>),
    /// Undo: the drawing was cleared; this is what it was.
    Cleared(Vec<Stroke>),
    /// Redo: clear again.
    Clear,
}

#[derive(Clone, Copy)]
struct Styles {
    title: StyleId,
    status: StyleId,
    key: StyleId,
    key_on: StyleId,
    glyph: StyleId,
    glyph_off: StyleId,
    notice: StyleId,
    question: StyleId,
    body: StyleId,
}

/// Every rectangle on the page for the current window.
#[derive(Clone, Copy, Debug)]
struct Layout {
    keys: [Rect; KEYS],
    /// The toolbar's rows.
    tools: Rect,
    /// The paper, scaled to fit below the toolbar.
    paper: Rect,
    /// Window pixels per paper unit.
    k: f32,
    dialog: Rect,
    yes: Rect,
    no: Rect,
}

impl Layout {
    /// A paper point in window coordinates.
    fn on_window(&self, p: Point) -> Point {
        (self.paper.x + p.0 * self.k, self.paper.y + p.1 * self.k)
    }

    /// A window point on the paper, moved inside it when it is not.
    fn on_paper(&self, x: f32, y: f32) -> Point {
        (
            ((x - self.paper.x) / self.k).clamp(0.0, PAPER_W),
            ((y - self.paper.y) / self.k).clamp(0.0, PAPER_H),
        )
    }
}

struct Page {
    window: Window,
    theme: Theme,
    s: Styles,
    strokes: Vec<Stroke>,
    /// The path of each stroke for the paper as `paths_for` has it;
    /// empty when the drawing changed. Built in `frame`.
    paths: Vec<Path>,
    paths_for: Option<(f32, f32, f32)>,
    /// Points and ink of `strokes`, kept in step with it.
    points: usize,
    ink: f32,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// False when the drawing could not be read or written: "sparas inte".
    stored: bool,
    color: usize,
    pen: usize,
    eraser: bool,
    /// The points of the stroke being drawn, in paper units.
    raw: Vec<Point>,
    drawing: bool,
    /// While the eraser is down: the strokes it has touched, ascending,
    /// and where it was last.
    erasing: Option<(Vec<usize>, Option<Point>)>,
    /// A press on the paper found the drawing full.
    refused: bool,
    /// The clear dialog is open.
    confirm: bool,
    yes: Button,
    no: Button,
    /// The node the host's focus is on.
    focus: Option<i32>,
    /// Where the pointer is, for the eraser's mark.
    pointer: Option<(f32, f32)>,
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
fn number(mut n: usize) -> String {
    let mut digits = [0u8; 20];
    let mut len = 0;
    loop {
        digits[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    digits[..len].iter().rev().map(|d| *d as char).collect()
}

impl Page {
    fn layout(&self) -> Layout {
        let (w, h) = (self.window.width, self.window.height);
        let wide = w >= WIDE;
        let mut keys = [Rect::default(); KEYS];
        let (mut x, mut y) = (MARGIN, HEADER_H);
        for key in &mut keys[..K_PEN] {
            *key = Rect::new(x, y, KEY_H, KEY_H);
            x += KEY_H + GAP;
        }
        if wide {
            x += 10.0;
        } else {
            x = MARGIN;
            y += KEY_H + 8.0;
        }
        for key in &mut keys[K_PEN..K_ERASER] {
            *key = Rect::new(x, y, 38.0, KEY_H);
            x += 38.0 + GAP;
        }
        x += 6.0;
        keys[K_ERASER] = Rect::new(x, y, 58.0, KEY_H);
        x += 58.0 + GAP + 6.0;
        if wide {
            x = x.max(w - MARGIN - 150.0);
        }
        keys[K_UNDO] = Rect::new(x, y, 36.0, KEY_H);
        keys[K_REDO] = Rect::new(x + 42.0, y, 36.0, KEY_H);
        keys[K_CLEAR] = Rect::new(x + 84.0, y, 66.0, KEY_H);
        let tools = Rect::new(
            MARGIN,
            HEADER_H,
            (w - 2.0 * MARGIN).max(0.0),
            y + KEY_H - HEADER_H,
        );

        let top = tools.bottom() + 12.0;
        let room = Rect::new(
            MARGIN,
            top,
            (w - 2.0 * MARGIN).max(1.0),
            (h - top - MARGIN).max(1.0),
        );
        let k = (room.w / PAPER_W).min(room.h / PAPER_H);
        let scale = self.window.scale;
        let paper = Rect::new(
            snap(room.x + (room.w - PAPER_W * k) / 2.0, scale),
            snap(room.y, scale),
            PAPER_W * k,
            PAPER_H * k,
        );

        let dw = (w - 2.0 * MARGIN).clamp(0.0, 340.0);
        let dialog = Rect::new((w - dw) / 2.0, ((h - 150.0) / 2.0).max(0.0), dw, 150.0);
        let no = Rect::new(
            dialog.right() - 20.0 - 100.0,
            dialog.bottom() - 56.0,
            100.0,
            36.0,
        );
        let yes = Rect::new(no.x - 10.0 - 100.0, no.y, 100.0, 36.0);
        Layout {
            keys,
            tools,
            paper,
            k,
            dialog,
            yes,
            no,
        }
    }

    /// After every change to `strokes`: the totals, and the paths are
    /// built again in the next frame.
    fn recount(&mut self) {
        self.points = self.strokes.iter().map(|s| s.pts.len()).sum();
        self.ink = self.strokes.iter().map(Stroke::ink).sum();
        self.paths.clear();
    }

    /// Build the strokes' paths for the paper of `l`, unless they are
    /// already: sending them costs enough without building them anew in
    /// every frame of a pen stroke.
    fn build_paths(&mut self, l: &Layout) {
        let key = Some((l.paper.x, l.paper.y, l.k));
        if self.paths_for == key && self.paths.len() == self.strokes.len() {
            return;
        }
        self.paths_for = key;
        self.paths = self
            .strokes
            .iter()
            .map(|s| s.path(|p| l.on_window(p)))
            .collect();
    }

    /// True when the drawing takes no more strokes.
    fn full(&self) -> bool {
        self.points + 2 > MAX_POINTS || self.ink >= MAX_INK
    }

    fn save(&mut self) {
        let result = state_put(STORE, &encode(&self.strokes));
        if let Err(e) = result {
            let _ = log(Level::Warn, &cat(&["rita: state_put refused: ", e.name()]));
        }
        self.stored = result.is_ok();
    }

    /// A new step: it goes on the undo stack and nothing can be redone.
    fn did(&mut self, step: Step) {
        if self.undo.len() >= UNDO_MAX {
            self.undo.remove(0);
        }
        self.undo.push(step);
        self.redo.clear();
        self.recount();
        self.save();
    }

    /// Turn the collected points into a stroke; true when one was added.
    fn commit_stroke(&mut self) -> bool {
        let raw = core::mem::take(&mut self.raw);
        let tolerance = TOLERANCE_PX / self.layout().k;
        let stroke = Stroke::new(self.color as u8, self.pen as u8, &raw, tolerance);
        let Some(mut stroke) = stroke else {
            return false;
        };
        stroke.pts.truncate(MAX_POINTS.saturating_sub(self.points));
        if stroke.pts.is_empty() {
            return false;
        }
        self.strokes.push(stroke);
        self.did(Step::Drawn);
        true
    }

    /// Move the eraser to `p`, marking the strokes within its reach on
    /// the way there from where it was: a fast drag reports few points.
    fn erase_at(&mut self, p: Point, l: &Layout) {
        let Some((marked, from)) = self.erasing.as_mut() else {
            return;
        };
        let reach = RUBBER_R / l.k;
        let start = from.unwrap_or(p);
        *from = Some(p);
        let steps = ((distance(start, p) / reach) as usize).clamp(1, 32);
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let q = (start.0 + (p.0 - start.0) * t, start.1 + (p.1 - start.1) * t);
            for (i, stroke) in self.strokes.iter().enumerate() {
                if !marked.contains(&i) && stroke.touches(q, reach) {
                    marked.push(i);
                }
            }
        }
        marked.sort_unstable();
    }

    /// Lift the eraser: the marked strokes go, as one step.
    fn finish_erase(&mut self) -> bool {
        let Some((marked, _)) = self.erasing.take().filter(|m| !m.0.is_empty()) else {
            return false;
        };
        let mut gone: Vec<(usize, Stroke)> = marked
            .iter()
            .rev()
            .map(|&i| (i, self.strokes.remove(i)))
            .collect();
        gone.reverse();
        self.did(Step::Erased(gone));
        true
    }

    fn clear(&mut self) -> bool {
        if self.strokes.is_empty() {
            return false;
        }
        let all = core::mem::take(&mut self.strokes);
        self.did(Step::Cleared(all));
        self.refused = false;
        true
    }

    fn step_back(&mut self) -> bool {
        let Some(step) = self.undo.pop() else {
            return false;
        };
        let again = match step {
            Step::Drawn => match self.strokes.pop() {
                Some(stroke) => Step::Draw(stroke),
                None => return false,
            },
            Step::Erased(gone) => {
                let places = gone.iter().map(|g| g.0).collect();
                for (i, stroke) in gone {
                    self.strokes.insert(i.min(self.strokes.len()), stroke);
                }
                Step::Erase(places)
            }
            Step::Cleared(all) => {
                self.strokes = all;
                Step::Clear
            }
            other => other,
        };
        self.redo.push(again);
        self.after_history();
        true
    }

    fn step_again(&mut self) -> bool {
        let Some(step) = self.redo.pop() else {
            return false;
        };
        let back = match step {
            Step::Draw(stroke) => {
                self.strokes.push(stroke);
                Step::Drawn
            }
            Step::Erase(places) => {
                let mut gone = Vec::with_capacity(places.len());
                for &i in places.iter().rev() {
                    if i < self.strokes.len() {
                        gone.push((i, self.strokes.remove(i)));
                    }
                }
                gone.reverse();
                Step::Erased(gone)
            }
            Step::Clear => Step::Cleared(core::mem::take(&mut self.strokes)),
            other => other,
        };
        self.undo.push(back);
        self.after_history();
        true
    }

    fn after_history(&mut self) {
        self.refused = false;
        self.recount();
        self.save();
    }

    /// True when the event presses the control `id` in `r`: a primary
    /// press inside it, or Enter or Space while it has the host's focus.
    fn pressed(&self, ui: &Input<'_>, id: i32, r: Rect) -> bool {
        match ui.unhandled() {
            Some(Event::PointerDown { x, y, button, .. }) => {
                button == button::PRIMARY && r.contains(x, y)
            }
            Some(Event::KeyDown {
                code: key::ENTER | key::SPACE,
                repeat: false,
                ..
            }) => self.focus == Some(id),
            _ => false,
        }
    }

    /// What toolbar key `i` does. True when the drawing changed.
    fn key(&mut self, i: usize) -> bool {
        match i {
            K_ERASER => self.eraser = !self.eraser,
            K_UNDO => return self.step_back(),
            K_REDO => return self.step_again(),
            K_CLEAR => self.confirm = !self.strokes.is_empty(),
            i if i >= K_PEN => {
                self.pen = i - K_PEN;
                self.eraser = false;
            }
            i => {
                self.color = i;
                self.eraser = false;
            }
        }
        false
    }

    /// Pen, eraser and the history keys. True when the drawing changed.
    fn draw_event(&mut self, ui: &mut Input<'_>, l: &Layout) -> bool {
        let Some(event) = ui.unhandled() else {
            return false;
        };
        match event {
            Event::PointerDown { x, y, button, .. }
                if button == button::PRIMARY && l.paper.contains(x, y) =>
            {
                ui.take();
                ui.set_redraw();
                self.pointer = Some((x, y));
                let p = l.on_paper(x, y);
                if self.eraser {
                    self.erasing = Some((Vec::new(), None));
                    self.erase_at(p, l);
                } else if self.full() {
                    self.refused = true;
                } else {
                    self.raw.clear();
                    self.raw.push(p);
                    self.drawing = true;
                }
                false
            }
            Event::PointerMove { x, y, buttons, .. } => {
                self.pointer = Some((x, y));
                let held = buttons & button::mask(button::PRIMARY) != 0;
                let p = l.on_paper(x, y);
                if self.drawing && held {
                    ui.take();
                    // A point for every pixel and a half of movement.
                    let moved = self.raw.last().is_none_or(|q| distance(*q, p) * l.k >= 1.5);
                    if !moved {
                        return false;
                    }
                    self.raw.push(p);
                    ui.set_redraw();
                    if self.raw.len() < RAW_MAX {
                        return false;
                    }
                    // A very long stroke goes on in a new one.
                    let changed = self.commit_stroke();
                    self.drawing = !self.full();
                    self.refused = !self.drawing;
                    if self.drawing {
                        self.raw.push(p);
                    }
                    changed
                } else if self.erasing.is_some() && held {
                    ui.take();
                    ui.set_redraw();
                    self.erase_at(p, l);
                    false
                } else if self.drawing || self.erasing.is_some() {
                    // Released outside the window.
                    ui.set_redraw();
                    self.drawing = false;
                    self.commit_stroke() | self.finish_erase()
                } else {
                    if self.eraser {
                        ui.set_redraw();
                    }
                    false
                }
            }
            Event::PointerUp { button, .. } if button == button::PRIMARY => {
                if !self.drawing && self.erasing.is_none() {
                    return false;
                }
                ui.take();
                ui.set_redraw();
                self.drawing = false;
                self.commit_stroke() | self.finish_erase()
            }
            Event::PointerLeave => {
                self.pointer = None;
                if self.eraser {
                    ui.set_redraw();
                }
                false
            }
            Event::KeyDown {
                code, modifiers, ..
            } if modifiers.shortcut() && (code == KEY_Z || code == KEY_Y) => {
                ui.take();
                ui.set_redraw();
                if code == KEY_Y || modifiers.shift() {
                    self.step_again()
                } else {
                    self.step_back()
                }
            }
            _ => false,
        }
    }

    /// Strokes on the paper: those the eraser holds are already gone.
    fn shown(&self) -> usize {
        self.strokes.len() - self.erasing.as_ref().map_or(0, |e| e.0.len())
    }

    fn status(&self) -> String {
        let mut s = cat(&[&number(self.shown()), " streck"]);
        if self.full() {
            s.push_str(" · fullt");
        }
        if !self.stored {
            s.push_str(" · sparas inte");
        }
        s
    }

    /// The text of toolbar key `i`'s node: what it is and its state.
    fn key_label(&self, i: usize) -> String {
        let chosen = self.theme.words.selected;
        match i {
            K_ERASER => cat(&["Sudd", if self.eraser { " (på)" } else { " (av)" }]),
            K_UNDO | K_REDO => {
                let (name, steps) = if i == K_UNDO {
                    ("Ångra", self.undo.len())
                } else {
                    ("Gör om", self.redo.len())
                };
                if steps == 0 {
                    cat(&[name, " (inget)"])
                } else {
                    cat(&[name, " (", &number(steps), " steg)"])
                }
            }
            K_CLEAR => String::from("Rensa ritningen"),
            i if i >= K_PEN => {
                let on = self.pen == i - K_PEN;
                cat(&["Penna: ", PENS[i - K_PEN], if on { chosen } else { "" }])
            }
            i => {
                let on = self.color == i && !self.eraser;
                cat(&["Färg: ", COLORS[i].1, if on { chosen } else { "" }])
            }
        }
    }

    /// `text` centred in `r`.
    fn centred(f: &mut Frame, r: Rect, style: StyleId, text: &str) -> Result<(), VyError> {
        let size = text_measure(style, 0.0, text)?;
        f.text(
            r.x + (r.w - size.w) / 2.0,
            r.y + (r.h - size.h) / 2.0,
            0.0,
            style,
            text,
        )?;
        Ok(())
    }

    /// A filled circle around `c`.
    fn dot(f: &mut Frame, c: Point, radius: f32, color: Color) -> Result<(), VyError> {
        let r = Rect::new(c.0 - radius, c.1 - radius, 2.0 * radius, 2.0 * radius);
        f.rect(r, radius, color)
    }

    /// Toolbar key `i`; with `node` also its node (not under the dialog).
    fn draw_key(&self, f: &mut Frame, l: &Layout, i: usize, node: bool) -> Result<(), VyError> {
        let p = self.theme.palette;
        let r = l.keys[i];
        let id = ID_KEY - i as i32;
        if node && self.focus == Some(id) {
            f.rect(r.inset(-ui::RING), ui::RADIUS + ui::RING, p.ring)?;
        }
        let ink = COLORS[self.color].0;
        // A key is a surface with a border; the chosen one has a heavier,
        // darker border, the eraser when on is filled.
        let plate = |f: &mut Frame, on: bool, fill: Color| -> Result<(), VyError> {
            let (edge, t) = if on { (p.ink, 2.0) } else { (p.border, 1.0) };
            f.rect(r, ui::RADIUS, edge)?;
            f.rect(r.inset(t), ui::RADIUS - t, fill)
        };
        let middle = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        match i {
            K_ERASER => {
                let (fill, style) = if self.eraser {
                    (p.accent, self.s.key_on)
                } else {
                    (p.surface, self.s.key)
                };
                plate(f, false, fill)?;
                Self::centred(f, r, style, "Sudd")?;
            }
            K_UNDO | K_REDO => {
                let (glyph, steps) = if i == K_UNDO {
                    ("↶", self.undo.len())
                } else {
                    ("↷", self.redo.len())
                };
                plate(f, false, p.surface)?;
                let style = if steps == 0 {
                    self.s.glyph_off
                } else {
                    self.s.glyph
                };
                Self::centred(f, r, style, glyph)?;
            }
            K_CLEAR => {
                plate(f, false, p.surface)?;
                Self::centred(f, r, self.s.key, "Rensa")?;
            }
            i if i >= K_PEN => {
                plate(f, self.pen == i - K_PEN, p.surface)?;
                Self::dot(f, middle, [2.0, 4.0, 7.0][i - K_PEN], ink)?;
            }
            i => {
                plate(f, self.color == i && !self.eraser, p.surface)?;
                Self::dot(f, middle, 9.0, COLORS[i].0)?;
            }
        }
        if node {
            f.sem
                .node_with_id(id, Role::Button, r, &self.key_label(i))?;
        }
        Ok(())
    }

    /// Stroke `i`: its path, and a dot at each end as round caps.
    fn draw_stroke(&self, f: &mut Frame, l: &Layout, i: usize) -> Result<(), VyError> {
        let stroke = &self.strokes[i];
        let color = COLORS[usize::from(stroke.color) % COLORS.len()].0;
        let w = (stroke.pen() * l.k).max(1.0);
        let n = stroke.pts.len();
        Self::dot(f, l.on_window(stroke.point(0)), w / 2.0, color)?;
        if let Some(path) = self.paths.get(i).filter(|_| n > 1) {
            f.path(path, color, w)?;
            Self::dot(f, l.on_window(stroke.point(n - 1)), w / 2.0, color)?;
        }
        Ok(())
    }

    fn draw_paper(&self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let line = snap(1.0, self.window.scale);
        f.rect(l.paper.inset(-line), 0.0, PAPER_EDGE)?;
        f.rect(l.paper, 0.0, PAPER)?;
        let marked = self.erasing.as_ref().map_or(&[][..], |e| &e.0);
        f.clipped(l.paper, 0.0, |f| {
            for i in 0..self.strokes.len() {
                if !marked.contains(&i) {
                    self.draw_stroke(f, l, i)?;
                }
            }
            // The stroke being drawn, through every point collected.
            if self.drawing && !self.raw.is_empty() {
                let color = COLORS[self.color].0;
                let w = (WIDTHS[self.pen] * l.k).max(1.0);
                let at = |i: usize| l.on_window(self.raw[i]);
                Self::dot(f, at(0), w / 2.0, color)?;
                if self.raw.len() > 1 {
                    f.path(&curve(self.raw.len(), at), color, w)?;
                    Self::dot(f, at(self.raw.len() - 1), w / 2.0, color)?;
                }
            }
            match self.pointer {
                Some((x, y)) if self.eraser && !self.confirm => {
                    Self::dot(f, (x, y), RUBBER_R, RUBBER)
                }
                _ => Ok(()),
            }
        })?;
        let text = cat(&["Ritning med ", &number(self.shown()), " streck"]);
        f.sem.node(Role::Image, l.paper, &text)?;

        // The drawing is full: say so where the pen is.
        if self.refused || self.full() {
            let text = "Ritningen är full – ångra eller rensa för att rita mer.";
            let w = (l.paper.w - 24.0).max(0.0);
            let size = text_measure(self.s.notice, w - 24.0, text)?;
            let note = Rect::new(
                l.paper.x + (l.paper.w - size.w - 24.0) / 2.0,
                l.paper.bottom() - size.h - 28.0,
                size.w + 24.0,
                size.h + 16.0,
            );
            f.rect(note, 8.0, NOTICE)?;
            f.text(note.x + 12.0, note.y + 8.0, w - 24.0, self.s.notice, text)?;
            f.sem.node(Role::Text, note, text)?;
        }
        Ok(())
    }

    fn draw_dialog(&self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let t = self.theme;
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        f.rect(window, 0.0, SCRIM)?;
        f.rect(l.dialog, 12.0, t.palette.surface)?;
        let question = "Rensa hela ritningen?";
        f.group(Role::Dialog, l.dialog, question, |f| {
            let x = l.dialog.x + 20.0;
            let w = l.dialog.w - 40.0;
            let h = f
                .text(x, l.dialog.y + 18.0, w, self.s.question, question)?
                .h;
            f.sem.node(
                Role::Heading,
                Rect::new(x, l.dialog.y + 18.0, w, h),
                question,
            )?;
            let body = "Det går att ångra efteråt.";
            let y = l.dialog.y + 18.0 + h + 6.0;
            let h = f.text(x, y, w, self.s.body, body)?.h;
            f.sem.node(Role::Text, Rect::new(x, y, w, h), body)?;
            self.yes.draw(f, l.yes, &t, "Rensa")?;
            self.no.draw(f, l.no, &t, "Avbryt")
        })
    }
}

impl App for Page {
    fn init(window: Window, _abi_minor: i32) -> Result<Self, VyError> {
        let p = Palette::LIGHT;
        let theme = Theme::custom(p, Font::Sans, 14.0)?;
        // The stored drawing: its length first, then the bytes.
        let loaded = state_get(STORE, &mut []).and_then(|len| {
            let mut bytes = vec![0u8; len];
            let n = state_get(STORE, &mut bytes)?;
            Ok(bytes.get(..n).and_then(decode))
        });
        let (strokes, stored) = match loaded {
            Ok(strokes) => (strokes.unwrap_or_default(), true),
            Err(VyError::Resource) => (Vec::new(), true),
            Err(e) => {
                let _ = log(Level::Info, &cat(&["rita: no storage: ", e.name()]));
                (Vec::new(), false)
            }
        };
        let mut page = Self {
            window,
            theme,
            s: Styles {
                title: text_style(Font::Sans, 20.0, 700, p.ink)?,
                status: text_style(Font::Sans, 13.0, 400, p.muted)?,
                key: text_style(Font::Sans, 13.0, 600, p.ink)?,
                key_on: text_style(Font::Sans, 13.0, 600, p.on_accent)?,
                glyph: text_style(Font::Sans, 20.0, 400, p.ink)?,
                glyph_off: text_style(Font::Sans, 20.0, 400, p.border)?,
                notice: text_style(Font::Sans, 13.0, 600, PAPER)?,
                question: text_style(Font::Sans, 17.0, 700, p.ink)?,
                body: text_style(Font::Sans, 14.0, 400, p.muted)?,
            },
            strokes,
            paths: Vec::new(),
            paths_for: None,
            points: 0,
            ink: 0.0,
            undo: Vec::new(),
            redo: Vec::new(),
            stored,
            color: 0,
            pen: 1,
            eraser: false,
            raw: Vec::new(),
            drawing: false,
            erasing: None,
            refused: false,
            confirm: false,
            yes: Button::new(ID_YES),
            no: Button::secondary(ID_NO),
            focus: None,
            pointer: None,
        };
        page.recount();
        Ok(page)
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let l = self.layout();
        self.build_paths(&l);
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        f.rect(window, 0.0, PAGE)?;
        f.group(Role::Main, window, "", |f| {
            let h = f.text(MARGIN, 11.0, 0.0, self.s.title, "Ritbord")?.h;
            f.sem
                .node(Role::Heading, Rect::new(MARGIN, 11.0, 100.0, h), "Ritbord")?;
            let status = self.status();
            let size = text_measure(self.s.status, 0.0, &status)?;
            let x = self.window.width - MARGIN - size.w;
            f.text(x, 17.0, 0.0, self.s.status, &status)?;
            f.sem
                .node(Role::Text, Rect::new(x, 17.0, size.w, size.h), &status)?;
            if self.confirm {
                // Under the dialog the tools are drawn but not in the tree.
                for i in 0..KEYS {
                    self.draw_key(f, &l, i, false)?;
                }
            } else {
                f.group(Role::Region, l.tools, "Verktyg", |f| {
                    (0..KEYS).try_for_each(|i| self.draw_key(f, &l, i, true))
                })?;
            }
            self.draw_paper(f, &l)
        })?;
        if self.confirm {
            self.draw_dialog(f, &l)?;
        }
        Ok(())
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        let l = self.layout();
        let t = self.theme;
        let mut ui = Input::new(event);
        if let Event::Focus { node, gained, .. } = event {
            if gained {
                self.focus = Some(node.0);
            } else if self.focus == Some(node.0) {
                self.focus = None;
            }
            ui.set_redraw();
        }
        // The dialog's buttons follow the host's focus always, and take
        // presses only while the dialog is open.
        let dialog = self.confirm || matches!(event, Event::Focus { .. });
        let yes = dialog && self.yes.event(&mut ui, l.yes, &t).activated();
        let no = dialog && self.no.event(&mut ui, l.no, &t).activated();
        let mut changed = false;
        if self.confirm {
            // Everything else is behind the dialog.
            let escape = matches!(
                ui.unhandled(),
                Some(Event::KeyDown {
                    code: key::ESCAPE,
                    ..
                })
            );
            if yes || no || escape {
                self.confirm = false;
                changed = yes && self.clear();
                ui.take();
                ui.set_redraw();
            }
            return ui.redraw() || changed;
        }
        for i in 0..KEYS {
            if self.pressed(&ui, ID_KEY - i as i32, l.keys[i]) {
                ui.take();
                ui.set_redraw();
                changed |= self.key(i);
            }
        }
        changed |= self.draw_event(&mut ui, &l);
        ui.redraw() || changed
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
    }
}

vy_sdk::export!(Page);

#[cfg(test)]
mod tests;
