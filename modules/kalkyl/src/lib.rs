//! Vy example page `kalkyl` ("Kalkylark"): a spreadsheet of 26 columns
//! and 100 rows with formulas.
//!
//! What it shows beyond the reference pages:
//!
//! - **A virtualised grid.** Only the cells in view are drawn, under
//!   column letters and row numbers that stay put while the cells scroll:
//!   a `ui::ScrollView` scrolls the rows (wheel, bar), the page scrolls
//!   the columns (Shift+wheel, its own bar), and both follow the selection.
//! - **Formulas** (`sheet.rs`): `+ - * /`, parentheses, cell references,
//!   ranges and SUMMA, MEDEL, MIN, MAX, ANTAL, recalculated in dependency
//!   order with cycles shown as `#CIRKEL`. Numbers are fixed-point and
//!   formatted by the module, so a result never depends on a platform's
//!   floating point (SPEC-2.1.5).
//! - **A formula bar** that is a `ui::TextBox` editing the selected cell.
//!   Enter commits and moves down, Escape cancels. Tab is the host's key
//!   (SPEC-10.3.1), so "Tab commits and moves right" is said with the
//!   tree: the bar is registered between two more `textbox` nodes, the
//!   bars of the cells to the left and right. Tab moves the host's focus
//!   to the next one; the page commits, selects that cell and lets the
//!   bar take over the node that now has focus (see [`RING`]).
//! - **Keys without focus.** Cells are `cell` nodes, which cannot have the
//!   host's focus (SPEC-10.3.2); after a press in the grid the page takes
//!   the arrows itself and typing edits in the cell.
//! - **The copy gesture.** Ctrl+C writes the shown values of the selection
//!   while the page handles that key, which needs no capability
//!   (SPEC-8.2.4); pasted text arrives as `TEXT_INPUT` in the bar.
//! - **Storage** of every cell's source after each change (SPEC-5.2.9);
//!   without the capability the page says "sparas inte".
#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod sheet;

use alloc::string::String;
use alloc::vec;
use sheet::{
    col_name, col_row, format_decimals, format_number, index, name, push_u64, Sheet, Value, COLS,
    MAX_SRC, ROWS,
};
use vy_sdk::event::{button, key};
use vy_sdk::math::{ceil, floor, snap};
use vy_sdk::prelude::*;
use vy_sdk::ui::{Input, Label, Palette, ScrollView, TextBox, Theme};
use vy_sdk::{clipboard_write, now_ms, state_get, state_put};

const PAGE: Color = Color::rgb(0xF1, 0xF3, 0xF7);
const HEAD: Color = Color::rgb(0xEC, 0xEE, 0xF3);
const HEAD_ON: Color = Color::rgb(0xD5, 0xDE, 0xF7);
const LINE: Color = Color::rgb(0xDD, 0xE0, 0xE7);
const HEAD_LINE: Color = Color::rgb(0xC5, 0xC9, 0xD2);
const RANGE: Color = Color::rgba(0x2F, 0x5B, 0xD6, 0x24);
const FAULT: Color = Color::rgb(0xB4, 0x23, 0x2B);
const IDLE: Color = Color::rgb(0x8A, 0x93, 0xA6);

const HEADER_H: f32 = 44.0;
const BAR_H: f32 = 36.0;
const NAME_W: f32 = 76.0;
const STATUS_H: f32 = 28.0;
const ROW_HEAD_W: f32 = 44.0;
const COL_HEAD_H: f32 = 26.0;
const CELL_W: f32 = 96.0;
const CELL_H: f32 = 26.0;
const CELL_PAD: f32 = 6.0;
/// Room for the scroll bars right of and below the cells.
const GUTTER: f32 = 12.0;
const KEY: &str = "kalkyl";
const F2: i32 = 0x3B;
const DOUBLE_MS: i64 = 400;

/// The three node ids the formula bar moves between. In every frame the
/// tree holds, in this order, the bar of the cell to the left, the bar
/// itself and the bar of the cell to the right, with consecutive ids of
/// this ring. Tab from the bar gives the next id the host's focus, Shift+Tab
/// the previous; the page then selects that cell and the bar takes the
/// focused id, so the same three nodes surround it again (SPEC-10.3.1: a
/// page moves focus by changing the tree).
const RING: [i32; 3] = [-11, -12, -13];

#[derive(Clone, Copy)]
struct Styles {
    title: StyleId,
    name: StyleId,
    cell: StyleId,
    fault: StyleId,
    head: StyleId,
    status: StyleId,
    /// Line height of `cell`.
    line_h: f32,
}

/// Every rectangle on the page for the current window and scroll.
#[derive(Clone, Copy, Debug)]
struct Layout {
    name: Rect,
    bar: Rect,
    /// Headers, cells and scroll bars.
    grid: Rect,
    /// The cells.
    view: Rect,
    /// The cells and the gutter of the vertical scroll bar.
    rows_view: Rect,
    content_h: f32,
    htrack: Rect,
    hthumb: Option<Rect>,
    status: Rect,
    /// How far the cells are scrolled.
    ox: f32,
    oy: f32,
    /// Columns and rows in view: first and one past the last.
    cols: (usize, usize),
    rows: (usize, usize),
}

impl Layout {
    fn cell(&self, col: usize, row: usize) -> Rect {
        Rect::new(
            self.view.x + col as f32 * CELL_W - self.ox,
            self.view.y + row as f32 * CELL_H - self.oy,
            CELL_W,
            CELL_H,
        )
    }

    /// The cell under `(x, y)`, or the nearest one in view.
    fn cell_at(&self, x: f32, y: f32) -> (usize, usize) {
        let x = x.clamp(self.view.x, (self.view.right() - 1.0).max(self.view.x));
        let y = y.clamp(self.view.y, (self.view.bottom() - 1.0).max(self.view.y));
        let col = floor((x - self.view.x + self.ox) / CELL_W).max(0.0) as usize;
        let row = floor((y - self.view.y + self.oy) / CELL_H).max(0.0) as usize;
        (col.min(COLS - 1), row.min(ROWS - 1))
    }
}

struct Page {
    window: Window,
    theme: Theme,
    s: Styles,
    sheet: Sheet,
    /// False when the sheet could not be read or written: "sparas inte".
    stored: bool,
    /// The selected cell: column, row.
    cur: (usize, usize),
    /// The other corner of the selected range.
    anchor: (usize, usize),
    bar: TextBox,
    /// Which id of [`RING`] the bar has.
    ring: usize,
    /// True while the bar's text is an edit in progress.
    editing: bool,
    /// The previous event took the host's focus from the bar by keyboard.
    left_bar: bool,
    /// True while the grid takes the keys: no node has the host's focus.
    grid_active: bool,
    /// The pointer is dragging out a range.
    selecting: bool,
    scroll: ScrollView,
    scroll_x: f32,
    /// Where in the horizontal thumb the pointer grabbed it.
    hgrab: Option<f32>,
    /// The cell last pressed and when, for the double click.
    last_press: Option<((usize, usize), i64)>,
    /// What the last copy did, until the selection moves.
    message: String,
}

impl Page {
    fn layout(&self) -> Layout {
        let (w, h) = (self.window.width, self.window.height);
        let name = Rect::new(8.0, HEADER_H, NAME_W, BAR_H);
        let bar_x = name.right() + 6.0;
        let bar = Rect::new(bar_x, HEADER_H, (w - bar_x - 8.0).max(0.0), BAR_H);
        let top = HEADER_H + BAR_H + 8.0;
        let grid = Rect::new(0.0, top, w, (h - top - STATUS_H).max(0.0));
        let view = Rect::new(
            ROW_HEAD_W,
            grid.y + COL_HEAD_H,
            (grid.w - ROW_HEAD_W - GUTTER).max(0.0),
            (grid.h - COL_HEAD_H - GUTTER).max(0.0),
        );
        let content_w = COLS as f32 * CELL_W;
        let (ox, oy) = (self.scroll_x, self.scroll.offset());
        let htrack = Rect::new(
            view.x + 2.0,
            view.bottom() + 3.0,
            (view.w - 4.0).max(0.0),
            6.0,
        );
        let hthumb = (content_w > view.w && htrack.w > 0.0).then(|| {
            let tw = (htrack.w * view.w / content_w).clamp(24.0f32.min(htrack.w), htrack.w);
            let at = (htrack.w - tw) * ox / (content_w - view.w);
            Rect::new(htrack.x + at, htrack.y, tw, htrack.h)
        });
        let span = |o: f32, size: f32, cell: f32, n: usize| {
            (
                (floor(o / cell).max(0.0) as usize).min(n),
                (ceil((o + size) / cell).max(0.0) as usize).min(n),
            )
        };
        Layout {
            name,
            bar,
            grid,
            view,
            rows_view: Rect::new(view.x, view.y, view.w + GUTTER, view.h),
            content_h: ROWS as f32 * CELL_H,
            htrack,
            hthumb,
            status: Rect::new(0.0, h - STATUS_H, w, STATUS_H),
            ox,
            oy,
            cols: span(ox, view.w, CELL_W, COLS),
            rows: span(oy, view.h, CELL_H, ROWS),
        }
    }

    fn max_x(l: &Layout) -> f32 {
        (COLS as f32 * CELL_W - l.view.w).max(0.0)
    }

    /// The selected range: first column, first row, last column, last row.
    fn range(&self) -> (usize, usize, usize, usize) {
        (
            self.cur.0.min(self.anchor.0),
            self.cur.1.min(self.anchor.1),
            self.cur.0.max(self.anchor.0),
            self.cur.1.max(self.anchor.1),
        )
    }

    /// The selection's name: `B3`, or `B3:D5`.
    fn range_name(&self) -> String {
        let (c0, r0, c1, r1) = self.range();
        let mut s = name(index(c0, r0));
        if (c0, r0) != (c1, r1) {
            s.push(':');
            s.push_str(&name(index(c1, r1)));
        }
        s
    }

    /// Source of the cell `d` columns from the selected one.
    fn beside(&self, d: i32) -> &str {
        let col = self.cur.0 as i32 + d;
        if (0..COLS as i32).contains(&col) {
            self.sheet.src(index(col as usize, self.cur.1))
        } else {
            ""
        }
    }

    /// Show the selected cell's source in the bar, selected, so that
    /// typing replaces it.
    fn load_bar(&mut self) {
        let src = String::from(self.sheet.src(index(self.cur.0, self.cur.1)));
        self.bar.set_text(&src);
        self.bar.select_all();
        self.editing = false;
    }

    /// Scroll the least that brings the selected cell into view.
    fn reveal(&mut self) {
        let l = self.layout();
        let r = l.cell(self.cur.0, self.cur.1);
        if r.x < l.view.x {
            self.scroll_x -= l.view.x - r.x;
        } else if r.right() > l.view.right() {
            // By whole columns, so the left edge stays on a column.
            let need = (r.right() - l.view.right()).min(r.x - l.view.x);
            self.scroll_x = ceil((self.scroll_x + need) / CELL_W) * CELL_W;
        }
        self.scroll_x = self.scroll_x.clamp(0.0, Self::max_x(&l));
        self.scroll.reveal(r, l.view, l.content_h);
    }

    fn select(&mut self, cell: (usize, usize), extend: bool) {
        self.cur = cell;
        if !extend {
            self.anchor = cell;
        }
        self.message.clear();
        self.load_bar();
        self.reveal();
    }

    /// Select the cell `dc` columns and `dr` rows away, if there is one.
    fn step(&mut self, dc: i32, dr: i32) {
        let col = (self.cur.0 as i32 + dc).clamp(0, COLS as i32 - 1) as usize;
        let row = (self.cur.1 as i32 + dr).clamp(0, ROWS as i32 - 1) as usize;
        self.select((col, row), false);
    }

    fn save(&mut self) {
        let result = state_put(KEY, &self.sheet.encode());
        if let Err(e) = result {
            let _ = log(
                Level::Warn,
                &cat(&["kalkyl: state_put refused: ", e.name()]),
            );
        }
        self.stored = result.is_ok();
    }

    /// Put the bar's text into the selected cell and recalculate; true
    /// when the sheet changed.
    fn commit(&mut self) -> bool {
        if !self.editing {
            return false;
        }
        let i = index(self.cur.0, self.cur.1);
        let changed = self.sheet.src(i) != self.bar.text().trim();
        if changed {
            let text = String::from(self.bar.text());
            self.sheet.set(i, &text);
            self.sheet.recalc();
            self.save();
        }
        self.load_bar();
        changed
    }

    /// Empty every cell of the selection.
    fn clear(&mut self) -> bool {
        let (c0, r0, c1, r1) = self.range();
        let mut changed = false;
        for row in r0..=r1 {
            for col in c0..=c1 {
                let i = index(col, row);
                changed |= self.sheet.get(i).is_some();
                self.sheet.set(i, "");
            }
        }
        if changed {
            self.sheet.recalc();
            self.save();
        }
        self.load_bar();
        changed
    }

    /// Ctrl+C: the shown values of the selection, tabs between columns
    /// and line feeds between rows, written during the copy gesture
    /// (SPEC-8.2.4).
    fn copy(&mut self) {
        let (c0, r0, c1, r1) = self.range();
        let mut text = String::new();
        for row in r0..=r1 {
            for col in c0..=c1 {
                if col > c0 {
                    text.push('\t');
                }
                if let Some(cell) = self.sheet.get(index(col, row)) {
                    text.push_str(cell.shown());
                }
            }
            if row < r1 {
                text.push('\n');
            }
        }
        self.message = match clipboard_write(&text) {
            Ok(()) => cat(&[&self.range_name(), " kopierat."]),
            Err(e) => cat(&["Urklippet nekades (", e.name(), ")."]),
        };
    }

    /// The status line: what the last copy did, or the selection's sum,
    /// mean and count.
    fn status(&self) -> String {
        if !self.message.is_empty() {
            return self.message.clone();
        }
        let (c0, r0, c1, r1) = self.range();
        if (c0, r0) == (c1, r1) {
            return String::from("Enter ned · Tab höger · Esc ångrar");
        }
        let (mut sum, mut n) = (0i64, 0i64);
        for row in r0..=r1 {
            for col in c0..=c1 {
                if let Some(Value::Num(v)) = self.sheet.get(index(col, row)).map(|c| c.value) {
                    sum = sum.saturating_add(v);
                    n += 1;
                }
            }
        }
        let mut s = self.range_name();
        if n > 0 {
            let mean = (sum + n / 2 * sum.signum()) / n;
            s.push_str(&cat(&[
                " · Summa ",
                &format_number(sum),
                " · Medel ",
                &format_number(mean),
                " · Antal ",
                &number(n as usize),
            ]));
        }
        s
    }

    fn note(&self) -> String {
        let mut s = cat(&[&number(self.sheet.count()), " celler"]);
        if self.sheet.overloaded {
            s.push_str(" · för många beroenden");
        }
        if !self.stored {
            s.push_str(" · sparas inte");
        }
        s
    }

    /// Arrow and paging keys: move the selection, or with Shift its end.
    fn nav(&mut self, code: i32, extend: bool, l: &Layout) {
        let (c, r) = self.cur;
        let page = ((l.view.h / CELL_H) as usize).max(1);
        let cell = match code {
            key::LEFT => (c.saturating_sub(1), r),
            key::RIGHT => ((c + 1).min(COLS - 1), r),
            key::UP => (c, r.saturating_sub(1)),
            key::DOWN => (c, (r + 1).min(ROWS - 1)),
            key::PAGE_UP => (c, r.saturating_sub(page)),
            key::PAGE_DOWN => (c, (r + page).min(ROWS - 1)),
            key::HOME => (0, r),
            // The last cell of the row with something in it.
            _ => (
                (0..COLS)
                    .rev()
                    .find(|col| self.sheet.get(index(*col, r)).is_some())
                    .unwrap_or(c),
                r,
            ),
        };
        self.select(cell, extend);
    }

    /// Keys while the bar or the grid has the keyboard, before the bar
    /// sees them. True when the sheet changed.
    fn keys(&mut self, ui: &mut Input<'_>, l: &Layout) -> bool {
        let focused = self.bar.focused();
        match ui.unhandled() {
            Some(Event::KeyDown {
                code, modifiers, ..
            }) => {
                if modifiers.shortcut() {
                    // While editing, Ctrl+C is the field's own.
                    if code == key::C && !(self.editing && focused) {
                        self.copy();
                        ui.take();
                        ui.set_redraw();
                    }
                    return false;
                }
                let mut changed = false;
                match code {
                    key::ESCAPE if self.editing => self.load_bar(),
                    // With focus, Enter comes back from the field.
                    key::ENTER if !focused => {
                        changed = self.commit();
                        self.step(0, if modifiers.shift() { -1 } else { 1 });
                    }
                    key::UP | key::DOWN | key::PAGE_UP | key::PAGE_DOWN => {
                        changed = self.commit();
                        self.nav(code, modifiers.shift(), l);
                    }
                    // Editing in the bar, these move its caret.
                    key::LEFT | key::RIGHT | key::HOME | key::END if !(self.editing && focused) => {
                        changed = self.commit();
                        self.nav(code, modifiers.shift(), l);
                    }
                    key::DELETE | key::BACKSPACE if !self.editing => changed = self.clear(),
                    key::BACKSPACE if !focused => {
                        let mut text = String::from(self.bar.text());
                        text.pop();
                        self.bar.set_text(&text);
                    }
                    F2 if !self.editing => {
                        self.editing = true;
                        let end = self.bar.text().len();
                        self.bar.select(end..end);
                    }
                    _ => return false,
                }
                ui.take();
                ui.set_redraw();
                changed
            }
            // Typing with the keyboard on the grid edits in the cell.
            Some(Event::TextInput { text }) if !focused => {
                let mut value = if self.editing {
                    String::from(self.bar.text())
                } else {
                    String::new()
                };
                value.push_str(text);
                self.bar.set_text(&value);
                self.editing = true;
                ui.take();
                ui.set_redraw();
                false
            }
            _ => false,
        }
    }

    /// Pointer and wheel on the grid. True when the sheet changed.
    fn pointer(&mut self, ui: &mut Input<'_>, l: &Layout) -> bool {
        let Some(event) = ui.unhandled() else {
            return false;
        };
        match event {
            Event::PointerDown {
                x,
                y,
                button,
                modifiers,
            } if button == button::PRIMARY => {
                if l.view.contains(x, y) {
                    ui.take();
                    ui.set_redraw();
                    let changed = self.commit();
                    let cell = l.cell_at(x, y);
                    let now = now_ms();
                    let double = self
                        .last_press
                        .is_some_and(|(last, at)| last == cell && now - at <= DOUBLE_MS);
                    self.last_press = (!double).then_some((cell, now));
                    self.select(cell, modifiers.shift());
                    // A double click edits what is there, in the cell.
                    self.editing = double;
                    self.grid_active = true;
                    self.selecting = true;
                    return changed;
                }
                let on_track = x >= l.htrack.x
                    && x < l.htrack.right()
                    && y >= l.view.bottom()
                    && y < l.view.bottom() + GUTTER;
                if let (true, Some(thumb)) = (on_track, l.hthumb) {
                    ui.take();
                    ui.set_redraw();
                    if x >= thumb.x && x < thumb.right() {
                        self.hgrab = Some(x - thumb.x);
                    } else {
                        let page = if x < thumb.x { -l.view.w } else { l.view.w };
                        self.scroll_x = (self.scroll_x + page).clamp(0.0, Self::max_x(l));
                    }
                }
                false
            }
            Event::PointerMove { x, y, buttons, .. } => {
                let held = buttons & button::mask(button::PRIMARY) != 0;
                if !held {
                    self.selecting = false;
                    self.hgrab = None;
                } else if let (Some(grab), Some(thumb)) = (self.hgrab, l.hthumb) {
                    let room = (l.htrack.w - thumb.w).max(1.0);
                    let frac = ((x - grab - l.htrack.x) / room).clamp(0.0, 1.0);
                    self.scroll_x = frac * Self::max_x(l);
                    ui.take();
                    ui.set_redraw();
                } else if self.selecting {
                    let cell = l.cell_at(x, y);
                    if cell != self.cur {
                        self.cur = cell;
                        self.load_bar();
                        ui.set_redraw();
                    }
                    ui.take();
                }
                false
            }
            Event::PointerUp { .. } => {
                self.selecting = false;
                self.hgrab = None;
                false
            }
            // Shift+wheel, or a sideways wheel, scrolls the columns.
            Event::Scroll {
                dx,
                dy,
                lines,
                modifiers,
            } => {
                let across = if modifiers.shift() { dy } else { dx };
                if across != 0.0 {
                    let px = if lines { across * CELL_W } else { across };
                    let next = (self.scroll_x + px).clamp(0.0, Self::max_x(l));
                    if next != self.scroll_x {
                        self.scroll_x = next;
                        ui.set_redraw();
                    }
                    if modifiers.shift() {
                        ui.take();
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// A frame of `t` logical pixels just inside `r`.
    fn outline(f: &mut Frame, r: Rect, t: f32, color: Color) -> Result<(), VyError> {
        f.rect(Rect::new(r.x, r.y, r.w, t), 0.0, color)?;
        f.rect(Rect::new(r.x, r.bottom() - t, r.w, t), 0.0, color)?;
        f.rect(Rect::new(r.x, r.y + t, t, r.h - 2.0 * t), 0.0, color)?;
        f.rect(
            Rect::new(r.right() - t, r.y + t, t, r.h - 2.0 * t),
            0.0,
            color,
        )
    }

    /// The text of one cell in view.
    fn draw_cell(
        &mut self,
        f: &mut Frame,
        l: &Layout,
        col: usize,
        row: usize,
    ) -> Result<(), VyError> {
        let i = index(col, row);
        let style = self.s.cell;
        let room = CELL_W - 2.0 * CELL_PAD;
        match self.sheet.get_mut(i) {
            Some(cell) if cell.width < 0.0 => {
                // A number too wide for its cell loses decimals until it fits.
                let mut w = text_measure(style, 0.0, cell.shown())?.w;
                cell.decimals = 6;
                if let Value::Num(v) = cell.value {
                    while w > room && cell.decimals > 0 {
                        cell.decimals -= 1;
                        let shorter = format_decimals(v, u32::from(cell.decimals));
                        w = text_measure(style, 0.0, &shorter)?.w;
                    }
                }
                cell.width = w;
            }
            Some(_) => {}
            None => return Ok(()),
        }
        let Some(cell) = self.sheet.get(i) else {
            return Ok(());
        };
        let r = l.cell(col, row);
        let y = r.y + (CELL_H - self.s.line_h) / 2.0;
        let x = r.right() - CELL_PAD - cell.width;
        match cell.value {
            Value::Num(_) if cell.width > room => {
                f.text_centred(r, style, "#####")?;
            }
            Value::Num(v) if cell.decimals < 6 => {
                let shorter = format_decimals(v, u32::from(cell.decimals));
                f.text(x, y, 0.0, style, &shorter)?;
            }
            Value::Num(_) => {
                f.text(x, y, 0.0, style, cell.shown())?;
            }
            Value::Fault(_) => {
                f.text_centred(r, self.s.fault, cell.shown())?;
            }
            Value::Text => {
                // A long text runs over the empty cells to its right.
                let needs = ceil((cell.width + 2.0 * CELL_PAD) / CELL_W) as usize;
                let free = (col + 1..(col + needs).min(COLS))
                    .all(|c| self.sheet.get(index(c, row)).is_none());
                if cell.width <= room || free {
                    f.text(r.x + CELL_PAD, y, 0.0, style, cell.shown())?;
                } else {
                    let inside = Rect::new(r.x, r.y, CELL_W - CELL_PAD / 2.0, CELL_H);
                    f.clipped(inside, 0.0, |f| {
                        f.text(r.x + CELL_PAD, y, 0.0, style, cell.shown())
                    })?;
                }
            }
        }
        Ok(())
    }

    fn draw_grid(&mut self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let p = self.theme.palette;
        let scale = self.window.scale;
        let line = snap(1.0, scale);
        let w = self.window.width;
        let (c0, c1) = l.cols;
        let (r0, r1) = l.rows;
        f.rect(
            Rect::new(l.view.x, l.view.y, l.view.w + GUTTER, l.view.h + GUTTER),
            0.0,
            p.surface,
        )?;
        for col in c0..=c1 {
            let x = snap(l.cell(col, 0).x, scale) - line;
            f.rect(Rect::new(x, l.view.y, line, l.view.h), 0.0, LINE)?;
        }
        for row in r0..=r1 {
            let y = snap(l.cell(0, row).y, scale) - line;
            f.rect(Rect::new(l.view.x, y, l.view.w, line), 0.0, LINE)?;
        }
        let (sc0, sr0, sc1, sr1) = self.range();
        let first = l.cell(sc0, sr0);
        let last = l.cell(sc1, sr1);
        let range = Rect::new(
            first.x,
            first.y,
            last.right() - first.x,
            last.bottom() - first.y,
        );
        let many = (sc0, sr0) != (sc1, sr1);
        if many {
            f.rect(range, 0.0, RANGE)?;
        }
        for row in r0..r1 {
            for col in c0..c1 {
                if !(self.editing && (col, row) == self.cur) {
                    self.draw_cell(f, l, col, row)?;
                }
            }
        }
        if many {
            Self::outline(f, range, line, p.accent)?;
        }
        // The selected cell; while it is edited, the text being typed.
        let cur = l.cell(self.cur.0, self.cur.1);
        let keyboard = self.grid_active || self.bar.focused();
        let edge = if keyboard { p.accent } else { IDLE };
        if self.editing {
            let text = self.bar.text();
            let tw = text_measure(self.s.cell, 0.0, text)?.w;
            let wide =
                (tw + 2.0 * CELL_PAD + 2.0).clamp(CELL_W, (l.view.right() - cur.x).max(CELL_W));
            let field = Rect::new(cur.x, cur.y, wide, CELL_H);
            f.rect(field, 0.0, p.surface)?;
            let y = cur.y + (CELL_H - self.s.line_h) / 2.0;
            // A text wider than the field shows its end, where the caret is.
            let x = (cur.x + CELL_PAD).min(field.right() - CELL_PAD - tw - 2.0);
            f.clipped(field, 0.0, |f| {
                f.text(x, y, 0.0, self.s.cell, text)?;
                if !self.bar.focused() {
                    let caret = Rect::new(x + tw, y + 1.0, line, self.s.line_h - 2.0);
                    f.rect(caret, 0.0, p.ink)?;
                }
                Ok(())
            })?;
            Self::outline(f, field, 2.0, edge)?;
        } else {
            Self::outline(f, cur, 2.0, edge)?;
        }

        // The headers stay put: drawn over whatever scrolled under them.
        let top = Rect::new(0.0, l.grid.y, w, COL_HEAD_H);
        f.rect(top, 0.0, HEAD)?;
        for col in c0..c1 {
            let r = Rect::new(l.cell(col, 0).x, top.y, CELL_W, COL_HEAD_H);
            if (sc0..=sc1).contains(&col) {
                f.rect(r, 0.0, HEAD_ON)?;
            }
            let mut letter = [0u8; 4];
            f.text_centred(r, self.s.head, col_name(col).encode_utf8(&mut letter))?;
            let x = snap(r.right(), scale) - line;
            f.rect(
                Rect::new(x, top.y + 5.0, line, COL_HEAD_H - 10.0),
                0.0,
                HEAD_LINE,
            )?;
        }
        let side = Rect::new(0.0, l.view.y, ROW_HEAD_W, l.view.h + GUTTER);
        f.rect(side, 0.0, HEAD)?;
        for row in r0..r1 {
            let r = Rect::new(0.0, l.cell(0, row).y, ROW_HEAD_W, CELL_H);
            if (sr0..=sr1).contains(&row) {
                f.rect(r, 0.0, HEAD_ON)?;
            }
            f.text_centred(r, self.s.head, &number(row + 1))?;
        }
        // The corner, the lines along the headers and the gutters.
        f.rect(Rect::new(0.0, top.y, ROW_HEAD_W, COL_HEAD_H), 0.0, HEAD)?;
        f.rect(Rect::new(0.0, l.view.y - line, w, line), 0.0, HEAD_LINE)?;
        f.rect(
            Rect::new(ROW_HEAD_W - line, top.y, line, l.grid.h),
            0.0,
            HEAD_LINE,
        )?;
        f.rect(Rect::new(0.0, top.y - line, w, line), 0.0, HEAD_LINE)?;
        let right = Rect::new(l.view.right(), top.y, GUTTER, l.grid.h);
        f.rect(right, 0.0, HEAD)?;
        let below = Rect::new(0.0, l.view.bottom(), w, GUTTER);
        f.rect(below, 0.0, HEAD)?;
        if let Some(thumb) = self.scroll.thumb(l.rows_view, l.content_h) {
            let color = if self.scroll.dragging() {
                p.muted
            } else {
                p.track
            };
            f.rect(thumb, 3.0, color)?;
        }
        if let Some(thumb) = l.hthumb {
            let color = if self.hgrab.is_some() {
                p.muted
            } else {
                p.track
            };
            f.rect(thumb, 3.0, color)?;
        }
        Ok(())
    }

    /// The table of the semantic tree: a row for every row with content
    /// and a cell for every cell that is not empty, in view or not
    /// (SPEC-10.1.1). Empty cells have no text and no node.
    fn register_table(&self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        f.sem
            .begin(Role::Table, l.grid, "Blad med 26 kolumner och 100 rader")?;
        let mut open = None;
        for i in self.sheet.filled() {
            let (col, row) = col_row(i);
            if open != Some(row) {
                if open.is_some() {
                    f.sem.end();
                }
                let r = l.cell(0, row);
                let all = Rect::new(0.0, r.y, self.window.width, CELL_H);
                f.sem
                    .begin(Role::Row, all, &cat(&["Rad ", &number(row + 1)]))?;
                open = Some(row);
            }
            if let Some(cell) = self.sheet.get(i) {
                f.sem.node(Role::Cell, l.cell(col, row), &cell.label)?;
            }
        }
        if open.is_some() {
            f.sem.end();
        }
        f.sem.end();
        Ok(())
    }

    fn draw_page(&mut self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let t = self.theme;
        let p = t.palette;
        f.text(12.0, 10.0, 0.0, self.s.title, "Kalkylark")?;
        f.sem.node(
            Role::Heading,
            Rect::new(12.0, 10.0, 120.0, 24.0),
            "Kalkylark",
        )?;

        // The name of the selection, then the bar between its neighbours.
        let selected = self.range_name();
        f.rect(l.name, 6.0, p.border)?;
        f.rect(l.name.inset(1.0), 5.0, p.surface)?;
        f.text_centred(l.name, self.s.name, &selected)?;
        f.sem
            .node(Role::Text, l.name, &cat(&["Markerat: ", &selected]))?;
        // Each of the three fields has its name as the `text` node before
        // it: the bar's is the cell it edits.
        let here = Rect::new(l.bar.x, l.bar.y, 0.0, 0.0);
        let (prev, next) = ((self.ring + 2) % 3, (self.ring + 1) % 3);
        f.sem.node(Role::Text, here, "Föregående cell")?;
        f.sem
            .node_with_id(RING[prev], Role::Textbox, here, self.beside(-1))?;
        let cell = cat(&["Innehåll i ", &name(index(self.cur.0, self.cur.1))]);
        self.bar.draw(
            f,
            l.bar,
            &t,
            Label::hidden(&cell),
            "Värde eller formel, till exempel =SUMMA(B4:D4)",
        )?;
        f.sem.node(Role::Text, here, "Nästa cell")?;
        f.sem
            .node_with_id(RING[next], Role::Textbox, here, self.beside(1))?;

        self.draw_grid(f, l)?;
        self.register_table(f, l)?;

        // The status line: the selection to the left, the sheet to the right.
        let line = snap(1.0, self.window.scale);
        f.rect(l.status, 0.0, PAGE)?;
        f.rect(Rect::new(0.0, l.status.y, l.status.w, line), 0.0, HEAD_LINE)?;
        let note = self.note();
        let size = text_measure(self.s.status, 0.0, &note)?;
        let y = l.status.y + (STATUS_H - size.h) / 2.0;
        let note_x = (l.status.w - 12.0 - size.w).max(12.0);
        f.text(note_x, y, 0.0, self.s.status, &note)?;
        let status = self.status();
        let room = Rect::new(12.0, l.status.y, (note_x - 24.0).max(0.0), STATUS_H);
        f.clipped(room, 0.0, |f| f.text(12.0, y, 0.0, self.s.status, &status))?;
        f.sem.node(Role::Text, room, &status)?;
        f.sem
            .node(Role::Text, Rect::new(note_x, y, size.w, size.h), &note)?;
        Ok(())
    }
}

/// The parts joined. The page builds its few strings by hand: nothing
/// here needs `core::fmt`, which keeps the module small.
fn cat(parts: &[&str]) -> String {
    let mut s = String::new();
    for part in parts {
        s.push_str(part);
    }
    s
}

/// `n` in decimal.
fn number(n: usize) -> String {
    let mut s = String::new();
    push_u64(&mut s, n as u64);
    s
}

/// The stored sheet: `Ok(None)` when there is none or it is not a sheet.
fn load() -> Result<Option<Sheet>, VyError> {
    let len = match state_get(KEY, &mut []) {
        Ok(len) => len,
        Err(VyError::Resource) => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut bytes = vec![0u8; len];
    let n = state_get(KEY, &mut bytes)?;
    Ok(bytes.get(..n).and_then(Sheet::decode))
}

impl App for Page {
    fn init(window: Window, _abi_minor: i32) -> Result<Self, VyError> {
        let p = Palette::LIGHT;
        let theme = Theme::custom(p, Font::Sans, 14.0)?;
        let (sheet, stored) = match load() {
            Ok(sheet) => (sheet.unwrap_or_else(Sheet::sample), true),
            Err(e) => {
                let _ = log(Level::Info, &cat(&["kalkyl: no storage: ", e.name()]));
                (Sheet::sample(), false)
            }
        };
        let cell = text_style(Font::Sans, 13.0, 400, p.ink)?;
        let mut page = Self {
            window,
            theme,
            s: Styles {
                title: text_style(Font::Sans, 18.0, 700, p.ink)?,
                name: text_style(Font::Sans, 13.0, 600, p.ink)?,
                cell,
                fault: text_style(Font::Sans, 13.0, 400, FAULT)?,
                head: text_style(Font::Sans, 12.0, 600, p.muted)?,
                status: text_style(Font::Sans, 12.0, 400, p.muted)?,
                line_h: text_measure(cell, 0.0, "Ag")?.h,
            },
            sheet,
            stored,
            cur: (0, 0),
            anchor: (0, 0),
            bar: TextBox::new(RING[0]).with_max_len(MAX_SRC),
            ring: 0,
            editing: false,
            left_bar: false,
            grid_active: true,
            selecting: false,
            scroll: ScrollView::new(),
            scroll_x: 0.0,
            hgrab: None,
            last_press: None,
            message: String::new(),
        };
        page.load_bar();
        Ok(page)
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let l = self.layout();
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        f.rect(window, 0.0, PAGE)?;
        f.group(Role::Main, window, "", |f| self.draw_page(f, &l))
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        let t = self.theme;
        let mut ui = Input::new(event);
        let mut changed = false;
        match event {
            Event::Focus {
                node,
                gained: true,
                keyboard,
            } => {
                self.grid_active = false;
                let to = RING.iter().position(|id| *id == node.0);
                if let Some(to) = to.filter(|to| *to != self.ring) {
                    // A neighbour of the bar has the focus now. From the
                    // bar by Tab or Shift+Tab, that is the cell beside;
                    // from anywhere else it only means "the bar".
                    let forward = to == (self.ring + 1) % 3;
                    self.ring = to;
                    if keyboard && self.left_bar {
                        self.step(if forward { 1 } else { -1 }, 0);
                    }
                    self.bar = TextBox::new(RING[to]).with_max_len(MAX_SRC);
                    self.load_bar();
                }
                ui.set_redraw();
            }
            Event::Focus {
                node,
                gained: false,
                ..
            } => {
                if node.0 == RING[self.ring] {
                    changed |= self.commit();
                }
                ui.set_redraw();
            }
            _ => {}
        }
        self.left_bar = matches!(
            event,
            Event::Focus { node, gained: false, keyboard: true } if node.0 == RING[self.ring]
        );

        let l = self.layout();
        if self.bar.focused() || self.grid_active {
            changed |= self.keys(&mut ui, &l);
        }
        let into_bar = matches!(
            ui.unhandled(),
            Some(Event::PointerDown { x, y, .. }) if l.bar.contains(x, y)
        );
        let up = matches!(event, Event::KeyDown { modifiers, .. } if modifiers.shift());
        let field = self.bar.event(&mut ui, l.bar, &t);
        if field.activated() {
            changed |= self.commit();
            self.step(0, if up { -1 } else { 1 });
        } else if field.changed() || into_bar {
            self.editing = true;
        }
        changed |= self.pointer(&mut ui, &l);
        self.scroll.event(&mut ui, l.rows_view, &t, l.content_h);
        ui.redraw() || changed
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
        let l = self.layout();
        self.scroll.clamp(l.view, l.content_h);
        self.scroll_x = self.scroll_x.clamp(0.0, Self::max_x(&l));
    }
}

vy_sdk::export!(Page);

#[cfg(test)]
mod tests;
