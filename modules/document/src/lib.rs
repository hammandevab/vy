//! # document
//!
//! Vy reference module 5 (`document`): a report with title and byline, a
//! body that flows into two columns on wide windows and one below 640 px,
//! a data table, footnotes, and keyboard focus that travels across links.
//!
//! What the page exercises in the ABI:
//! - `vy_resize` (SPEC-5.1.4) re-measures every block with `text_measure`
//!   (SPEC-5.2.5, SPEC-6.4.3) and re-flows the body: two columns balanced
//!   by measured height, or one column under 640 logical pixels.
//! - A semantic tree with `table`, `row` and `cell` nodes for the data
//!   table, `nav` for the table of contents and `list`/`listitem` for the
//!   footnotes (SPEC-10.1.1, SPEC-A.3.1). The whole tree is registered every
//!   frame, on-screen or not, so node ids are stable and a host can build its
//!   focus order from it (SPEC-10.3.1); only visible blocks are drawn.
//! - `KIND_FOCUS` (SPEC-10.3.1): the host owns focus and turns Tab,
//!   Shift+Tab and pointer presses into focus events; the page keeps no
//!   focus of its own, draws the ring where the host says focus is
//!   (SPEC-10.3.3) and scrolls a link into view when the change came from
//!   the keyboard. Enter follows the link, which is a `navigate`
//!   (SPEC-5.2.10) for external links and a scroll for the sections.
//! - `link_register` (SPEC-5.2.10) for every link each frame, since scrolling
//!   moves them; `SCROLL`, Page Up/Down, Home/End in a clipped page
//!   (SPEC-6.1.4). No capabilities.
#![no_std]

extern crate alloc;

mod content;

use alloc::format;
use alloc::vec::Vec;
use content::{Block, BODY, NOTES, SECTIONS, TABLE_HEAD, TABLE_ROWS};
use vy_sdk::event::key;
use vy_sdk::math::snap;
use vy_sdk::prelude::*;
use vy_sdk::{link_register, navigate};

const PAPER: Color = Color::rgb(0xFC, 0xFB, 0xF8);
const INK: Color = Color::rgb(0x1F, 0x1F, 0x24);
const MUTED: Color = Color::rgb(0x62, 0x62, 0x6A);
const RULE: Color = Color::rgb(0xD4, 0xD0, 0xC6);
const ACCENT: Color = Color::rgb(0x8A, 0x2B, 0x2B);
const RING: Color = Color::rgb(0x2F, 0x5B, 0xD6);
const HEAD_BG: Color = Color::rgb(0xEE, 0xEB, 0xE3);
const STRIPE: Color = Color::rgb(0xF5, 0xF3, 0xEE);
const BAR: Color = Color::rgba(0x1F, 0x1F, 0x24, 0x50);

const TOP: f32 = 40.0;
const BOTTOM: f32 = 80.0;
const MARGIN: f32 = 24.0;
const MAX_COLUMN: f32 = 880.0;
const COL_GAP: f32 = 36.0;
/// Windows at least this wide get two body columns.
const TWO_COLUMNS_MIN: f32 = 640.0;
const ROW_H: f32 = 26.0;
/// Height of a 13 px line.
const LINE_13: f32 = 17.0;
const LINE_PX: f32 = 24.0;
const ARROW_PX: f32 = 48.0;
/// Space kept around a link scrolled into view.
const VIEW_PAD: f32 = 24.0;

/// Links in document order (the host's Tab order, SPEC-10.3.2): author,
/// three sections, the CSV, three sources.
const LINKS: usize = 1 + SECTIONS.len() + 1 + NOTES.len();

struct LinkInfo {
    label: &'static str,
    url: &'static str,
    /// Body block a section link scrolls to; `None` navigates.
    section: Option<usize>,
}

fn link_info(i: usize) -> LinkInfo {
    if i == 0 {
        return LinkInfo {
            label: content::AUTHOR,
            url: content::AUTHOR_URL,
            section: None,
        };
    }
    if let Some((label, url, block)) = SECTIONS.get(i - 1) {
        return LinkInfo {
            label,
            url,
            section: Some(*block),
        };
    }
    if i == 1 + SECTIONS.len() {
        return LinkInfo {
            label: content::CSV_LABEL,
            url: content::CSV_URL,
            section: None,
        };
    }
    let (_, label, url) = NOTES[i - 2 - SECTIONS.len()];
    LinkInfo {
        label,
        url,
        section: None,
    }
}

/// Everything laid out in document coordinates (y from the top of the
/// page, before scrolling). Rebuilt on init and resize.
#[derive(Default)]
struct Layout {
    column: Rect,
    title: Rect,
    byline_y: f32,
    nav_y: f32,
    rule_y: f32,
    lead: Rect,
    body: Vec<Rect>,
    table: Rect,
    caption_h: f32,
    notes_heading_y: f32,
    notes: Vec<Rect>,
    links: [Rect; LINKS],
    content_h: f32,
}

struct DocumentPage {
    window: Window,
    title: StyleId,
    byline: StyleId,
    link13: StyleId,
    link14: StyleId,
    lead: StyleId,
    h2: StyleId,
    body: StyleId,
    caption: StyleId,
    cell: StyleId,
    cell_small: StyleId,
    head: StyleId,
    head_small: StyleId,
    note: StyleId,
    layout: Layout,
    scroll: f32,
    focus: Option<usize>,
    /// Node id each link got in the last frame, for `KIND_FOCUS`.
    link_ids: [i32; LINKS],
}

impl DocumentPage {
    fn two_columns(&self) -> bool {
        self.window.width >= TWO_COLUMNS_MIN
    }

    fn cell_style(&self) -> (StyleId, StyleId) {
        if self.two_columns() {
            (self.cell, self.head)
        } else {
            (self.cell_small, self.head_small)
        }
    }

    fn measure_h(&self, style: StyleId, w: f32, text: &str) -> f32 {
        text_measure(style, w, text).map_or(0.0, |s| s.h)
    }

    fn measure_w(&self, style: StyleId, text: &str) -> f32 {
        text_measure(style, 0.0, text).map_or(0.0, |s| s.w)
    }

    /// Spacing above and below a body block.
    fn spacing(block: &Block, first: bool) -> (f32, f32) {
        match block {
            Block::Heading(_) => (if first { 0.0 } else { 18.0 }, 8.0),
            Block::Para(_) => (0.0, 14.0),
        }
    }

    fn relayout(&mut self) {
        let mut l = Layout::default();
        let w = self.window.width;
        let col_w = (w - 2.0 * MARGIN).clamp(0.0, MAX_COLUMN);
        let col_x = ((w - col_w) / 2.0).max(MARGIN);
        l.column = Rect::new(col_x, 0.0, col_w, 0.0);
        let mut y = TOP;

        let th = self.measure_h(self.title, col_w, content::TITLE);
        l.title = Rect::new(col_x, y, col_w, th);
        y += th + 10.0;

        l.byline_y = y;
        let av = self.measure_w(self.byline, "Av ");
        l.links[0] = Rect::new(
            col_x + av,
            y,
            self.measure_w(self.link13, content::AUTHOR),
            LINE_13,
        );
        y += LINE_13 + 8.0;

        l.nav_y = y;
        let mut x = col_x + self.measure_w(self.byline, content::NAV_LABEL) + 8.0;
        let sep = self.measure_w(self.byline, " · ");
        for (i, (label, _, _)) in SECTIONS.iter().enumerate() {
            let lw = self.measure_w(self.link13, label);
            l.links[1 + i] = Rect::new(x, y, lw, LINE_13);
            x += lw + sep;
        }
        y += LINE_13 + 18.0;
        l.rule_y = y;
        y += 1.0 + 22.0;

        let lh = self.measure_h(self.lead, col_w, content::LEAD);
        l.lead = Rect::new(col_x, y, col_w, lh);
        y += lh + 28.0;

        // Body: measure every block at the column width, then split into
        // two columns at the block boundary that balances them best.
        let two = self.two_columns();
        let bw = if two { (col_w - COL_GAP) / 2.0 } else { col_w };
        let heights: Vec<f32> = BODY
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let (above, below) = Self::spacing(b, i == 0);
                let text = match b {
                    Block::Heading(t) => (self.h2, *t),
                    Block::Para(t) => (self.body, *t),
                };
                above + self.measure_h(text.0, bw, text.1) + below
            })
            .collect();
        let total: f32 = heights.iter().sum();
        let split = if two {
            let mut best = (BODY.len(), f32::MAX);
            let mut left = 0.0;
            for (k, h) in heights.iter().enumerate() {
                left += h;
                let diff = (left - (total - left)).abs();
                // A heading must not end the left column.
                let ends_with_heading = matches!(BODY[k], Block::Heading(_));
                if diff < best.1 && !ends_with_heading {
                    best = (k + 1, diff);
                }
            }
            best.0
        } else {
            BODY.len()
        };
        let body_top = y;
        let mut ly = body_top;
        let mut ry = body_top;
        for (i, b) in BODY.iter().enumerate() {
            let (above, below) = Self::spacing(b, i == 0);
            let text_h = heights[i] - above - below;
            let (x, cursor, above) = if i < split {
                (col_x, &mut ly, above)
            } else {
                // The right column starts flush with the top, like the left.
                let above = if i == split { 0.0 } else { above };
                (col_x + bw + COL_GAP, &mut ry, above)
            };
            let r = Rect::new(x, *cursor + above, bw, text_h);
            *cursor += above + text_h + below;
            l.body.push(r);
        }
        y = ly.max(ry) + 16.0;

        l.caption_h = self.measure_h(self.caption, col_w, content::TABLE_CAPTION);
        let table_h = l.caption_h + 8.0 + (1.0 + TABLE_ROWS.len() as f32) * ROW_H;
        l.table = Rect::new(col_x, y, col_w, table_h);
        y += table_h + 12.0;

        let csv_i = 1 + SECTIONS.len();
        l.links[csv_i] = Rect::new(
            col_x,
            y,
            self.measure_w(self.link14, content::CSV_LABEL),
            18.0,
        );
        y += 18.0 + 36.0;

        l.notes_heading_y = y;
        y += 24.0 + 10.0;
        for (i, (text, label, _)) in NOTES.iter().enumerate() {
            let h = self.measure_h(self.note, col_w, text);
            l.notes.push(Rect::new(col_x, y, col_w, h));
            y += h + 4.0;
            l.links[csv_i + 1 + i] =
                Rect::new(col_x + 18.0, y, self.measure_w(self.link13, label), LINE_13);
            y += LINE_13 + 12.0;
        }
        l.content_h = y + BOTTOM;
        self.layout = l;
        self.scroll_to(self.scroll);
    }

    fn max_scroll(&self) -> f32 {
        (self.layout.content_h - self.window.height).max(0.0)
    }

    fn scroll_to(&mut self, y: f32) -> bool {
        let clamped = y.clamp(0.0, self.max_scroll());
        let changed = clamped != self.scroll;
        self.scroll = clamped;
        changed
    }

    fn scroll_by(&mut self, dy: f32) -> bool {
        self.scroll_to(self.scroll + dy)
    }

    /// Scroll so that `r` (document coordinates) is on screen with padding.
    fn reveal(&mut self, r: Rect) -> bool {
        let top = r.y - VIEW_PAD;
        let bottom = r.bottom() + VIEW_PAD;
        if top < self.scroll {
            self.scroll_to(top)
        } else if bottom > self.scroll + self.window.height {
            self.scroll_to(bottom - self.window.height)
        } else {
            false
        }
    }

    fn set_focus(&mut self, next: Option<usize>, keyboard: bool) -> bool {
        let changed = next != self.focus;
        self.focus = next;
        if let (Some(i), true) = (next, keyboard) {
            self.reveal(self.layout.links[i]);
        }
        changed
    }

    /// Enter on the focused link.
    fn activate(&mut self) -> bool {
        let Some(i) = self.focus else {
            return false;
        };
        let info = link_info(i);
        match info.section {
            Some(block) => {
                let y = self.layout.body[block].y - VIEW_PAD;
                self.scroll_to(y)
            }
            None => {
                match navigate(info.url, false) {
                    Ok(()) => {
                        let _ = log(Level::Info, &format!("document: navigate {}", info.url));
                    }
                    Err(e) => {
                        let _ = log(Level::Warn, &format!("document: navigate refused: {e}"));
                    }
                }
                false
            }
        }
    }

    /// A 1 px rule's top and height, snapped to device pixels (SPEC-6.3.1).
    fn hairline(&self, y: f32) -> (f32, f32) {
        let scale = self.window.scale;
        (snap(y, scale), snap(1.0, scale))
    }

    fn visible(&self, r: Rect) -> bool {
        let y = r.y - self.scroll;
        y + r.h > 0.0 && y < self.window.height
    }

    /// Draw link `i` (ring, text, underline) and register it with the host
    /// and in the tree. `style` is the text style of its context.
    fn draw_link(&mut self, f: &mut Frame, i: usize, style: StyleId) -> Result<(), VyError> {
        let info = link_info(i);
        let r = self.layout.links[i].offset(0.0, -self.scroll);
        link_register(i as i32 + 1, r, info.url)?;
        if self.visible(self.layout.links[i]) {
            if self.focus == Some(i) {
                f.rect(r.inset(-4.0), 5.0, RING)?;
                f.rect(r.inset(-2.0), 3.0, PAPER)?;
            }
            f.text(r.x, r.y, 0.0, style, info.label)?;
            let (uy, uh) = self.hairline(r.bottom() - 1.0);
            f.rect(Rect::new(r.x, uy, r.w, uh), 0.0, ACCENT)?;
        }
        let id = f.sem.node(Role::Link, r, info.label)?;
        self.link_ids[i] = id.0;
        Ok(())
    }

    fn draw_table(&self, f: &mut Frame) -> Result<(), VyError> {
        let t = self.layout.table.offset(0.0, -self.scroll);
        let (cell, head) = self.cell_style();
        // The name column gets more of a narrow window; numbers need less.
        let (fractions, head_labels) = if self.two_columns() {
            ([0.46, 0.16, 0.18, 0.20], TABLE_HEAD)
        } else {
            ([0.54, 0.17, 0.12, 0.17], content::TABLE_HEAD_NARROW)
        };
        let widths: [f32; 4] = [
            t.w * fractions[0],
            t.w * fractions[1],
            t.w * fractions[2],
            t.w * fractions[3],
        ];
        let text_dy = (ROW_H - LINE_13) / 2.0;
        let table_on = self.visible(self.layout.table);
        if table_on {
            f.text(t.x, t.y, t.w, self.caption, content::TABLE_CAPTION)?;
        }
        f.group(Role::Table, t, content::TABLE_CAPTION, |f| {
            let mut y = t.y + self.layout.caption_h + 8.0;
            let draw_row = |f: &mut Frame,
                            y: f32,
                            cells: &[&str; 4],
                            style: StyleId,
                            fill: Option<Color>|
             -> Result<(), VyError> {
                let row = Rect::new(t.x, y, t.w, ROW_H);
                let on_screen = y + ROW_H > 0.0 && y < self.window.height;
                if on_screen {
                    if let Some(c) = fill {
                        f.rect(row, 0.0, c)?;
                    }
                }
                f.group(Role::Row, row, "", |f| {
                    let mut x = t.x;
                    for (c, text) in cells.iter().enumerate() {
                        let cw = widths[c];
                        let r = Rect::new(x, y, cw, ROW_H);
                        if on_screen {
                            if c == 0 {
                                f.clipped(Rect::new(x, y, cw - 8.0, ROW_H), 0.0, |f| {
                                    f.text(x + 8.0, y + text_dy, 0.0, style, text).map(|_| ())
                                })?;
                            } else {
                                let tw = text_measure(style, 0.0, text)?.w;
                                f.text(x + cw - 8.0 - tw, y + text_dy, 0.0, style, text)?;
                            }
                        }
                        f.sem.node(Role::Cell, r, text)?;
                        x += cw;
                    }
                    Ok(())
                })
            };
            draw_row(f, y, &head_labels, head, Some(HEAD_BG))?;
            y += ROW_H;
            if table_on {
                let (ry, rh) = self.hairline(y - 1.0);
                f.rect(Rect::new(t.x, ry, t.w, rh), 0.0, RULE)?;
            }
            for (i, row) in TABLE_ROWS.iter().enumerate() {
                let fill = if i % 2 == 1 { Some(STRIPE) } else { None };
                draw_row(f, y, row, cell, fill)?;
                y += ROW_H;
            }
            if table_on {
                let (ry, rh) = self.hairline(y);
                f.rect(Rect::new(t.x, ry, t.w, rh), 0.0, RULE)?;
            }
            Ok(())
        })
    }
}

impl App for DocumentPage {
    fn init(window: Window, _abi_minor: i32) -> Result<Self, VyError> {
        let style = |font, size, weight, color| text_style(font, size, weight, color);
        let _ = log(Level::Debug, "document: init");
        let mut page = Self {
            window,
            title: style(Font::Sans, 30.0, 700, INK)?,
            byline: style(Font::Sans, 13.0, 400, MUTED)?,
            link13: style(Font::Sans, 13.0, 600, ACCENT)?,
            link14: style(Font::Sans, 14.0, 600, ACCENT)?,
            lead: style(Font::Serif, 18.0, 400, INK)?,
            h2: style(Font::Sans, 19.0, 700, INK)?,
            body: style(Font::Serif, 15.0, 400, INK)?,
            caption: style(Font::Sans, 13.0, 600, MUTED)?,
            cell: style(Font::Sans, 13.0, 400, INK)?,
            cell_small: style(Font::Sans, 12.0, 400, INK)?,
            head: style(Font::Sans, 13.0, 700, INK)?,
            head_small: style(Font::Sans, 12.0, 700, INK)?,
            note: style(Font::Sans, 13.0, 400, MUTED)?,
            layout: Layout::default(),
            scroll: 0.0,
            focus: None,
            link_ids: [0; LINKS],
        };
        page.relayout();
        Ok(page)
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        f.rect(window, 0.0, PAPER)?;
        let scroll = self.scroll;
        let col = Rect::new(
            self.layout.column.x,
            -scroll,
            self.layout.column.w,
            self.layout.content_h,
        );
        let col_x = col.x;
        let col_w = col.w;
        f.clipped(window, 0.0, |f| {
            f.group(Role::Main, col, "", |f| {
                // Title.
                let t = self.layout.title.offset(0.0, -scroll);
                if self.visible(self.layout.title) {
                    f.text(t.x, t.y, t.w, self.title, content::TITLE)?;
                }
                f.sem.node(Role::Heading, t, content::TITLE)?;

                // Byline with the author as a link.
                let by = self.layout.byline_y - scroll;
                let author = self.layout.links[0];
                let byline_r = Rect::new(col_x, by, col_w, LINE_13);
                if self.visible(Rect::new(col_x, self.layout.byline_y, col_w, LINE_13)) {
                    f.text(col_x, by, 0.0, self.byline, "Av ")?;
                    f.text(author.right(), by, 0.0, self.byline, content::BYLINE_TAIL)?;
                }
                f.sem.node(
                    Role::Text,
                    byline_r,
                    &format!("Av {}{}", content::AUTHOR, content::BYLINE_TAIL),
                )?;
                self.draw_link(f, 0, self.link13)?;

                // Table of contents.
                let ny = self.layout.nav_y - scroll;
                let nav_r = Rect::new(col_x, ny, col_w, LINE_13);
                if self.visible(Rect::new(col_x, self.layout.nav_y, col_w, LINE_13)) {
                    f.text(col_x, ny, 0.0, self.byline, content::NAV_LABEL)?;
                    for i in 1..SECTIONS.len() {
                        let prev = self.layout.links[i];
                        f.text(prev.right(), ny, 0.0, self.byline, " · ")?;
                    }
                }
                f.group(Role::Nav, nav_r, content::NAV_LABEL, |f| {
                    for i in 0..SECTIONS.len() {
                        self.draw_link(f, 1 + i, self.link13)?;
                    }
                    Ok(())
                })?;
                let ry = self.layout.rule_y - scroll;
                if ry > -1.0 && ry < self.window.height {
                    let (ry, rh) = self.hairline(ry);
                    f.rect(Rect::new(col_x, ry, col_w, rh), 0.0, RULE)?;
                }

                // Lead.
                let lead = self.layout.lead.offset(0.0, -scroll);
                if self.visible(self.layout.lead) {
                    f.text(lead.x, lead.y, lead.w, self.lead, content::LEAD)?;
                }
                f.sem.node(Role::Text, lead, content::LEAD)?;

                // Body blocks, already placed in one or two columns.
                for (i, b) in BODY.iter().enumerate() {
                    let r = self.layout.body[i].offset(0.0, -scroll);
                    let on = self.visible(self.layout.body[i]);
                    match b {
                        Block::Heading(t) => {
                            if on {
                                f.text(r.x, r.y, r.w, self.h2, t)?;
                            }
                            f.sem.node(Role::Heading, r, t)?;
                        }
                        Block::Para(t) => {
                            if on {
                                f.text(r.x, r.y, r.w, self.body, t)?;
                            }
                            f.sem.node(Role::Text, r, t)?;
                        }
                    }
                }

                self.draw_table(f)?;
                self.draw_link(f, 1 + SECTIONS.len(), self.link14)?;

                // Footnotes.
                let hy = self.layout.notes_heading_y - scroll;
                let hr = Rect::new(col_x, hy, col_w, 24.0);
                if self.visible(Rect::new(col_x, self.layout.notes_heading_y, col_w, 24.0)) {
                    f.text(col_x, hy, col_w, self.h2, content::NOTES_HEADING)?;
                }
                f.sem.node(Role::Heading, hr, content::NOTES_HEADING)?;
                let first = self.layout.notes[0].y;
                let last = self.layout.links[LINKS - 1].bottom();
                let list = Rect::new(col_x, first - scroll, col_w, last - first);
                f.group(Role::List, list, "", |f| {
                    for (i, (text, _, _)) in NOTES.iter().enumerate() {
                        let r = self.layout.notes[i].offset(0.0, -scroll);
                        if self.visible(self.layout.notes[i]) {
                            f.text(r.x, r.y, r.w, self.note, text)?;
                        }
                        f.sem.node(Role::ListItem, r, text)?;
                        self.draw_link(f, 2 + SECTIONS.len() + i, self.link13)?;
                    }
                    Ok(())
                })
            })?;
            // Scroll indicator.
            if self.max_scroll() > 0.0 {
                let track = self.window.height - 16.0;
                let thumb_h = (track * self.window.height / self.layout.content_h).max(24.0);
                let thumb_y = 8.0 + (track - thumb_h) * (scroll / self.max_scroll());
                f.rect(
                    Rect::new(self.window.width - 10.0, thumb_y, 4.0, thumb_h),
                    2.0,
                    BAR,
                )?;
            }
            Ok(())
        })
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        match event {
            Event::Scroll { dy, lines, .. } => {
                let px = if lines { dy * LINE_PX } else { dy };
                self.scroll_by(px)
            }
            Event::KeyDown { code, .. } => {
                let page = (self.window.height - ARROW_PX).max(ARROW_PX);
                match code {
                    key::ENTER => self.activate(),
                    key::DOWN => self.scroll_by(ARROW_PX),
                    key::UP => self.scroll_by(-ARROW_PX),
                    key::PAGE_DOWN | key::SPACE => self.scroll_by(page),
                    key::PAGE_UP => self.scroll_by(-page),
                    key::HOME => self.scroll_to(0.0),
                    key::END => self.scroll_to(f32::MAX),
                    _ => false,
                }
            }
            // A press focuses the link under it through the host's `FOCUS`
            // (SPEC-10.3.1); the page keeps no pointer focus of its own.
            Event::Focus {
                node,
                gained,
                keyboard,
            } => {
                let index = self.link_ids.iter().position(|id| *id == node.0);
                match (index, gained) {
                    (Some(i), true) => self.set_focus(Some(i), keyboard),
                    (Some(i), false) if self.focus == Some(i) => self.set_focus(None, false),
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
        self.relayout();
    }
}

vy_sdk::export!(DocumentPage);
