//! # list
//!
//! Vy reference module 3: a virtualised list of 200 rows (title, subtitle
//! and a right-aligned value) with single selection, keyboard navigation
//! and a detail dialog.
//!
//! What it exercises in the ABI: `SCROLL` in pixels and in lines and
//! `KEY_DOWN` (arrows, Page Up/Down, Home, End, Enter, Escape) driving one
//! scroll offset (SPEC-A.2.1); `POINTER_DOWN` hit-testing against row
//! rectangles the frame derives from the same arithmetic; `dl_clip_push` /
//! `dl_clip_pop` around the viewport so rows half outside it are cut
//! (SPEC-6.1.4); `text_measure` to right-align the value column
//! (SPEC-5.2.5); only the rows on screen are drawn and registered, as `list`
//! / `listitem` nodes, and the open dialog is a `dialog` node with a
//! `button` of fixed id (SPEC-10.1.1, SPEC-10.3.1). Row data is generated
//! from the row index, so the module carries no table. No capabilities.
#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use vy_sdk::event::key;
use vy_sdk::prelude::*;

const PAPER: Color = Color::rgb(0xF7, 0xF7, 0xF5);
const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const INK: Color = Color::rgb(0x1F, 0x23, 0x28);
const MUTED: Color = Color::rgb(0x64, 0x6B, 0x74);
const LINE: Color = Color::rgb(0xE3, 0xE5, 0xE8);
const ACCENT: Color = Color::rgb(0x25, 0x63, 0xEB);
const SELECT_BG: Color = Color::rgb(0xE8, 0xF0, 0xFE);
const BACKDROP: Color = Color::rgba(0x10, 0x14, 0x1A, 0x66);
const BAR: Color = Color::rgba(0x1F, 0x23, 0x28, 0x50);

/// Rows in the list.
pub const ROWS: usize = 200;
const HEADER_H: f32 = 64.0;
const ROW_H: f32 = 56.0;
const PAD: f32 = 16.0;
/// Pixels per line for `SCROLL` events that count lines: one row.
const LINE_PX: f32 = ROW_H;
const DIALOG_W: f32 = 480.0;
const DIALOG_PAD: f32 = 24.0;
const BUTTON_H: f32 = 40.0;
/// Fixed semantic id of the dialog's close button (SPEC-10.3.1).
const ID_CLOSE: i32 = 900;

const CUSTOMERS: [&str; 12] = [
    "Nordlund Bygg",
    "Sjöberg & Söner",
    "Ekdalens Trä",
    "Lindqvist Måleri",
    "Berglunds Rör",
    "Hagström El",
    "Åkerlunds Åkeri",
    "Fjällvind Design",
    "Granits Grus",
    "Holmgrens Glas",
    "Vikens Varv",
    "Örnbergs Plåt",
];
const KINDS: [&str; 5] = ["Faktura", "Kreditnota", "Offert", "Påminnelse", "Avtal"];
const STATUS: [&str; 4] = ["Betald", "Förfallen", "Skickad", "Utkast"];
const DESCRIPTION: &str = "Raden öppnades med Enter eller ett klick på en redan markerad rad. Escape, Enter eller ett klick utanför kortet stänger.";

/// One row, derived from its index so the module carries no table.
struct Row {
    number: u32,
    customer: &'static str,
    kind: &'static str,
    status: &'static str,
    date: String,
    amount: u32,
}

impl Row {
    fn new(i: usize) -> Self {
        let day = 1 + (i * 13) % 28;
        let month = 1 + (i / 17) % 12;
        // Multiplicative hash so the amounts look unsorted.
        let hash = (i as u32).wrapping_mul(2_654_435_761);
        Self {
            number: 1001 + i as u32,
            customer: CUSTOMERS[(i * 7) % CUSTOMERS.len()],
            kind: KINDS[(i * 3) % KINDS.len()],
            status: STATUS[(i * 5 + i / 4) % STATUS.len()],
            date: format!("2026-{month:02}-{day:02}"),
            amount: 500 + (hash ^ (hash >> 16)) % 24_000,
        }
    }

    fn title(&self) -> &'static str {
        self.customer
    }

    fn subtitle(&self) -> String {
        format!("{} {} · {} · {}", self.kind, self.number, self.status, self.date)
    }

    fn value(&self) -> String {
        kronor(self.amount)
    }
}

/// `12 340 kr`: thousands separated by a space.
fn kronor(n: u32) -> String {
    let digits = format!("{n}");
    let len = digits.len();
    let mut s = String::with_capacity(len + len / 3 + 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            s.push(' ');
        }
        s.push(c);
    }
    s.push_str(" kr");
    s
}

struct ListPage {
    window: Window,
    header: StyleId,
    count: StyleId,
    title: StyleId,
    subtitle: StyleId,
    value: StyleId,
    dialog_title: StyleId,
    label: StyleId,
    body: StyleId,
    button: StyleId,
    /// Scroll offset of the list in logical pixels, 0 at the top.
    scroll: f32,
    /// The one selected row.
    selected: usize,
    /// True while the detail dialog of `selected` is open.
    open: bool,
}

impl ListPage {
    /// The viewport below the header.
    fn list_area(&self) -> Rect {
        Rect::new(
            0.0,
            HEADER_H,
            self.window.width,
            (self.window.height - HEADER_H).max(0.0),
        )
    }

    fn max_scroll(&self) -> f32 {
        (ROWS as f32 * ROW_H - self.list_area().h).max(0.0)
    }

    /// Clamp and apply; true when the offset changed.
    fn scroll_to(&mut self, y: f32) -> bool {
        let clamped = y.clamp(0.0, self.max_scroll());
        let changed = clamped != self.scroll;
        self.scroll = clamped;
        changed
    }

    fn scroll_by(&mut self, dy: f32) -> bool {
        self.scroll_to(self.scroll + dy)
    }

    /// Scroll the least that brings row `i` fully into the viewport.
    fn ensure_visible(&mut self, i: usize) {
        let top = i as f32 * ROW_H;
        let bottom = top + ROW_H;
        let view_h = self.list_area().h;
        if top < self.scroll {
            self.scroll_to(top);
        } else if bottom > self.scroll + view_h {
            self.scroll_to(bottom - view_h);
        }
    }

    /// Move the selection to `i` (clamped); true when it changed.
    fn select(&mut self, i: usize) -> bool {
        let i = i.min(ROWS - 1);
        let changed = i != self.selected;
        self.selected = i;
        self.ensure_visible(i);
        changed
    }

    fn select_by(&mut self, delta: isize) -> bool {
        let i = (self.selected as isize + delta).clamp(0, ROWS as isize - 1);
        self.select(i as usize)
    }

    /// Rows that fit the viewport, for Page Up/Down.
    fn page_rows(&self) -> isize {
        ((self.list_area().h / ROW_H) as isize - 1).max(1)
    }

    /// The row under `(x, y)`, if any.
    fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        let area = self.list_area();
        if !area.contains(x, y) {
            return None;
        }
        let i = ((y - area.y + self.scroll) / ROW_H) as usize;
        (i < ROWS).then_some(i)
    }

    /// Rectangle of row `i` in window coordinates.
    fn row_rect(&self, i: usize) -> Rect {
        let area = self.list_area();
        Rect::new(
            area.x,
            area.y + i as f32 * ROW_H - self.scroll,
            area.w,
            ROW_H,
        )
    }

    /// Visible rows: the half-open index range that intersects the viewport.
    fn visible_rows(&self) -> core::ops::Range<usize> {
        let first = (self.scroll / ROW_H) as usize;
        let last = ((self.scroll + self.list_area().h) / ROW_H) as usize + 1;
        first.min(ROWS)..last.min(ROWS)
    }

    /// The dialog card and its close button; measured so a wrapped
    /// description grows the card.
    fn dialog_layout(&self) -> (Rect, Rect) {
        let w = (self.window.width - 48.0).clamp(0.0, DIALOG_W);
        let inner = w - 2.0 * DIALOG_PAD;
        let row = Row::new(self.selected);
        let heading = format!("{} {}", row.kind, row.number);
        let title_h = text_measure(self.dialog_title, inner, &heading).map_or(25.0, |s| s.h);
        let desc_h = text_measure(self.body, inner, DESCRIPTION).map_or(38.0, |s| s.h);
        // Title, four label/value lines, description, button.
        let h = DIALOG_PAD + title_h + 16.0 + 4.0 * 24.0 + 12.0 + desc_h + 20.0 + BUTTON_H + DIALOG_PAD;
        let x = (self.window.width - w) / 2.0;
        let y = ((self.window.height - h) / 2.0).max(16.0);
        let card = Rect::new(x, y, w, h);
        let button = Rect::new(
            card.right() - DIALOG_PAD - 120.0,
            card.bottom() - DIALOG_PAD - BUTTON_H,
            120.0,
            BUTTON_H,
        );
        (card, button)
    }

    fn set_open(&mut self, open: bool) -> bool {
        let changed = open != self.open;
        self.open = open;
        changed
    }

    fn draw_row(&self, f: &mut Frame, i: usize) -> Result<(), VyError> {
        let r = self.row_rect(i);
        let row = Row::new(i);
        if i == self.selected {
            f.rect(r, 0.0, SELECT_BG)?;
            f.rect(Rect::new(r.x, r.y, 4.0, r.h), 0.0, ACCENT)?;
        }
        let subtitle = row.subtitle();
        let value = row.value();
        let value_w = text_measure(self.value, 0.0, &value)?.w;
        let value_x = r.right() - PAD - 12.0 - value_w;
        let text_w = (value_x - 12.0 - (r.x + PAD)).max(0.0);
        // Title and subtitle on one line each, cut by a nested clip rather
        // than wrapped, so a narrow window never grows the row.
        f.clipped(Rect::new(r.x + PAD, r.y, text_w, r.h), |f| {
            f.text(r.x + PAD, r.y + 9.0, 0.0, self.title, row.title())?;
            f.text(r.x + PAD, r.y + 30.0, 0.0, self.subtitle, &subtitle)?;
            Ok(())
        })?;
        f.text(value_x, r.y + 18.0, 0.0, self.value, &value)?;
        f.rect(Rect::new(r.x + PAD, r.bottom() - 1.0, r.w - 2.0 * PAD, 1.0), 0.0, LINE)?;
        let name = format!("{}, {}, {}", row.title(), subtitle, value);
        f.sem.node(Role::ListItem, r, &name)?;
        Ok(())
    }

    fn draw_dialog(&self, f: &mut Frame) -> Result<(), VyError> {
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        f.rect(window, 0.0, BACKDROP)?;
        let (card, button) = self.dialog_layout();
        let row = Row::new(self.selected);
        let heading = format!("{} {}", row.kind, row.number);
        f.rect(card, 12.0, WHITE)?;
        f.group(Role::Dialog, card, &heading, |f| {
            let x = card.x + DIALOG_PAD;
            let w = card.w - 2.0 * DIALOG_PAD;
            let mut y = card.y + DIALOG_PAD;
            let h = f.text(x, y, w, self.dialog_title, &heading)?;
            f.sem.heading(Rect::new(x, y, w, h), &heading)?;
            y += h + 16.0;
            let value = row.value();
            let lines: [(&str, &str); 4] = [
                ("Kund", row.customer),
                ("Status", row.status),
                ("Datum", &row.date),
                ("Belopp", &value),
            ];
            for (label, text) in lines {
                f.text(x, y + 2.0, 90.0, self.label, label)?;
                f.text(x + 100.0, y, w - 100.0, self.body, text)?;
                let line = format!("{label}: {text}");
                f.sem.text(Rect::new(x, y, w, 24.0), &line)?;
                y += 24.0;
            }
            y += 12.0;
            let h = f.text(x, y, w, self.body, DESCRIPTION)?;
            f.sem.text(Rect::new(x, y, w, h), DESCRIPTION)?;
            f.rect(button, 6.0, ACCENT)?;
            let tw = text_measure(self.button, 0.0, "Stäng")?.w;
            f.text(
                button.x + (button.w - tw) / 2.0,
                button.y + (BUTTON_H - 19.0) / 2.0,
                0.0,
                self.button,
                "Stäng",
            )?;
            f.sem.node_with_id(ID_CLOSE, Role::Button, button, "Stäng")?;
            Ok(())
        })
    }
}

impl App for ListPage {
    fn init(window: Window, _abi_minor: i32) -> Self {
        let style =
            |font, size, weight, color| text_style(font, size, weight, color).unwrap_or(StyleId(0));
        let _ = log(Level::Debug, "list: init");
        Self {
            window,
            header: style(Font::Sans, 22.0, 700, INK),
            count: style(Font::Sans, 13.0, 400, MUTED),
            title: style(Font::Sans, 15.0, 600, INK),
            subtitle: style(Font::Sans, 13.0, 400, MUTED),
            value: style(Font::Mono, 15.0, 400, INK),
            dialog_title: style(Font::Sans, 20.0, 700, INK),
            label: style(Font::Sans, 13.0, 600, MUTED),
            body: style(Font::Sans, 15.0, 400, INK),
            button: style(Font::Sans, 15.0, 600, WHITE),
            scroll: 0.0,
            selected: 0,
            open: false,
        }
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        let area = self.list_area();
        f.rect(window, 0.0, PAPER)?;
        f.group(Role::Main, window, "", |f| {
            // Header with the title and the position of the selection.
            let header = Rect::new(0.0, 0.0, self.window.width, HEADER_H);
            f.rect(header, 0.0, WHITE)?;
            f.rect(Rect::new(0.0, HEADER_H - 1.0, self.window.width, 1.0), 0.0, LINE)?;
            let h = f.text(PAD, 18.0, self.window.width - 2.0 * PAD, self.header, "Fakturor")?;
            f.sem.heading(Rect::new(PAD, 18.0, 200.0, h), "Fakturor")?;
            let count = format!("{ROWS} poster · rad {}", self.selected + 1);
            let cw = text_measure(self.count, 0.0, &count)?.w;
            let cx = self.window.width - PAD - cw;
            let ch = f.text(cx, 25.0, 0.0, self.count, &count)?;
            f.sem.text(Rect::new(cx, 25.0, cw, ch), &count)?;

            // The rows on screen, clipped to the viewport (SPEC-6.1.4).
            f.clipped(area, |f| {
                f.group(Role::List, area, "Fakturor", |f| {
                    for i in self.visible_rows() {
                        self.draw_row(f, i)?;
                    }
                    Ok(())
                })
            })?;

            // Scroll indicator along the right edge.
            if self.max_scroll() > 0.0 {
                let track = area.h - 16.0;
                let content = ROWS as f32 * ROW_H;
                let thumb_h = (track * area.h / content).max(24.0);
                let thumb_y = area.y + 8.0 + (track - thumb_h) * (self.scroll / self.max_scroll());
                f.rect(
                    Rect::new(self.window.width - 10.0, thumb_y, 4.0, thumb_h),
                    2.0,
                    BAR,
                )?;
            }
            if self.open {
                self.draw_dialog(f)?;
            }
            Ok(())
        })
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        match event {
            Event::Scroll { dy, lines, .. } if !self.open => {
                let px = if lines { dy * LINE_PX } else { dy };
                self.scroll_by(px)
            }
            Event::KeyDown { code, .. } if self.open => match code {
                key::ESCAPE | key::ENTER => self.set_open(false),
                _ => false,
            },
            Event::KeyDown { code, .. } => match code {
                key::DOWN => self.select_by(1),
                key::UP => self.select_by(-1),
                key::PAGE_DOWN => self.select_by(self.page_rows()),
                key::PAGE_UP => self.select_by(-self.page_rows()),
                key::HOME => self.select(0),
                key::END => self.select(ROWS - 1),
                key::ENTER | key::SPACE => self.set_open(true),
                _ => false,
            },
            Event::PointerDown { x, y, .. } if self.open => {
                let (card, button) = self.dialog_layout();
                if button.contains(x, y) || !card.contains(x, y) {
                    self.set_open(false)
                } else {
                    false
                }
            }
            Event::PointerDown { x, y, .. } => match self.row_at(x, y) {
                Some(i) if i == self.selected => self.set_open(true),
                Some(i) => self.select(i),
                None => false,
            },
            _ => false,
        }
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
        self.scroll_to(self.scroll);
    }
}

vy_sdk::export!(ListPage);

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use vy_sdk::mock;

    fn page() -> ListPage {
        mock::reset();
        ListPage::init(
            Window {
                width: 800.0,
                height: 600.0,
                scale: 1.0,
            },
            0,
        )
    }

    fn down(page: &mut ListPage, code: i32) -> bool {
        page.event(Event::KeyDown {
            code,
            modifiers: Modifiers(0),
            repeat: false,
        })
    }

    #[test]
    fn arrows_move_selection_and_scroll_it_into_view() {
        let mut page = page();
        assert_eq!(page.selected, 0);
        for _ in 0..12 {
            assert!(down(&mut page, key::DOWN));
        }
        assert_eq!(page.selected, 12);
        // Row 12 ends at 728 px of content; the viewport is 536 px tall.
        assert_eq!(page.scroll, 13.0 * ROW_H - page.list_area().h);
        assert!(!down(&mut page, 0x04), "an unknown key changes nothing");
        assert!(down(&mut page, key::ENTER));
        assert!(page.open);
        assert!(down(&mut page, key::ESCAPE));
        assert!(!page.open);
        assert!(down(&mut page, key::END));
        assert_eq!(page.selected, ROWS - 1);
        assert_eq!(page.scroll, page.max_scroll());
        assert!(!down(&mut page, key::DOWN), "already at the last row");
    }

    #[test]
    fn frame_draws_only_visible_rows() {
        let mut page = page();
        let mut f = Frame::begin();
        page.frame(&mut f, 16).unwrap();
        f.end().unwrap();
        mock::with(|h| {
            let items = h
                .sem
                .iter()
                .filter(|n| n.role == Role::ListItem as i32)
                .count();
            assert_eq!(items, 10, "536 px of 56 px rows plus the cut one");
            assert!(h.refusals.is_empty(), "{:?}", h.refusals);
        });
    }

    #[test]
    fn click_selects_then_opens() {
        let mut page = page();
        let click = |page: &mut ListPage, y: f32| {
            page.event(Event::PointerDown {
                x: 100.0,
                y,
                button: 1,
                modifiers: Modifiers(0),
            })
        };
        assert!(click(&mut page, HEADER_H + 2.5 * ROW_H));
        assert_eq!(page.selected, 2);
        assert!(!page.open);
        assert!(click(&mut page, HEADER_H + 2.5 * ROW_H));
        assert!(page.open);
        assert!(click(&mut page, 1.0), "outside the card closes");
        assert!(!page.open);
    }

    #[test]
    fn kronor_groups_thousands() {
        assert_eq!(kronor(500), "500 kr");
        assert_eq!(kronor(12340), "12 340 kr");
        assert_eq!(kronor(1234567), "1 234 567 kr");
    }
}
