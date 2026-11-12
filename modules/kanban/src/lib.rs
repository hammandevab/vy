//! Vy example page `kanban` ("Tavlan"): a board with three columns whose
//! cards are dragged with the pointer or moved with the arrow keys.
//!
//! What it shows beyond the reference pages:
//!
//! - **Drag and drop on raw pointer events.** A press on a card that moves
//!   more than a few pixels lifts it; `layout()` takes the card out of its
//!   column, finds the column and slot under it and opens a gap there (the
//!   drop indicator), so `frame` and `event` agree on where it will land.
//!   On release the card eases into its slot over [`SETTLE_MS`], driven by
//!   `request_frame` and `dt_ms` (SPEC-5.2.2, SPEC-5.1.2), and the page is
//!   at rest again afterwards (SPEC-6.2.2).
//! - **Focus that follows a card.** Every card is a `button` node whose id
//!   is derived from the card, not from its place, so the host's focus
//!   (SPEC-10.3.2) stays on a card while the arrow keys move it between
//!   and within columns. Enter or a double click turns the same node into
//!   a `textbox`: a `ui::TextBox` constructed with the card's node id,
//!   which already has the host's focus, edits the title in place.
//! - **Storage.** The board is written with `state_put` after every change
//!   (SPEC-5.2.9) in a compact form, a few bytes more than the titles.
//!   Without the capability the page keeps its state in memory and says
//!   "sparas inte".
//! - **Three independent `ui::ScrollView`s**, and below 640 px one column
//!   at a time behind `tab` nodes; a card dropped on a tab moves there.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use vy_sdk::event::{button, key};
use vy_sdk::math::snap;
use vy_sdk::prelude::*;
use vy_sdk::ui::{self, Input, Palette, ScrollView, TextBox, Theme};
use vy_sdk::{now_ms, state_get, state_put};

const PAGE: Color = Color::rgb(0xF1, 0xF3, 0xF7);
const LANE: Color = Color::rgb(0xE3, 0xE6, 0xED);
const CARD: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const CARD_EDGE: Color = Color::rgb(0xD0, 0xD5, 0xDF);
const SLOT: Color = Color::rgba(0x2F, 0x5B, 0xD6, 0x2E);
const SHADOW: Color = Color::rgba(0x10, 0x18, 0x30, 0x38);
const DOTS: [Color; COLS] = [
    Color::rgb(0x8A, 0x93, 0xA6),
    Color::rgb(0x2F, 0x5B, 0xD6),
    Color::rgb(0x2E, 0x9E, 0x5B),
];

const COLS: usize = 3;
const NAMES: [&str; COLS] = ["Att göra", "Pågår", "Klart"];
/// Most cards on the whole board.
const MAX_CARDS: usize = 60;
/// Longest title, in bytes.
const MAX_TITLE: usize = 80;
const KEY: &str = "tavlan";
const FORMAT: u8 = 1;

/// Below this width one column is shown at a time, behind tabs.
const NARROW: f32 = 640.0;
const MARGIN: f32 = 16.0;
const HEADER_H: f32 = 56.0;
const TAB_H: f32 = 40.0;
const GAP: f32 = 12.0;
const MAX_LANE: f32 = 380.0;
const LANE_PAD: f32 = 10.0;
const LANE_HEAD: f32 = 42.0;
const FOOT_H: f32 = 36.0;
const CARD_PAD: f32 = 10.0;
const CARD_GAP: f32 = 8.0;
const CARD_TOP: f32 = 4.0;
/// Side of a card's delete button.
const DEL: f32 = 24.0;
/// How long a dropped or moved card takes to settle, in milliseconds.
const SETTLE_MS: f32 = 160.0;
/// A press becomes a drag after this many pixels.
const DRAG_START: f32 = 4.0;
/// Two presses on a card within this many milliseconds are a double click.
const DOUBLE_MS: i64 = 400;
const F2: i32 = 0x3B;

/// Fixed ids of the focusable nodes (SPEC-5.2.13); a card's own two ids
/// come from [`node_card`] and [`node_del`].
const ID_TAB: i32 = -10; // -10, -11, -12
const ID_NEW: i32 = -20; // -20, -21, -22
const ID_ADD: i32 = -30; // -30, -31, -32

/// The node id of card `id`: a button, or the title field while edited.
fn node_card(id: u32) -> i32 {
    -(1000 + 2 * id as i32)
}

/// The node id of card `id`'s delete button.
fn node_del(id: u32) -> i32 {
    node_card(id) - 1
}

/// The card a node id belongs to, and whether it is its delete button.
fn card_of(node: i32) -> Option<(u32, bool)> {
    let n = -1000 - node;
    (n >= 0).then_some(((n / 2) as u32, n % 2 == 1))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Card {
    /// Stable while the page runs: the card's node ids come from it.
    id: u32,
    title: String,
}

/// The cards of the three columns, top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Board {
    cols: [Vec<Card>; COLS],
    next_id: u32,
}

impl Board {
    fn empty() -> Self {
        Self {
            cols: [Vec::new(), Vec::new(), Vec::new()],
            next_id: 0,
        }
    }

    /// What a first visit shows, so the page is not empty.
    fn sample() -> Self {
        let mut b = Self::empty();
        for (c, title) in [
            (0, "Skriv utkast till nyhetsbrevet"),
            (0, "Boka lokal för höstmötet"),
            (0, "Beställ nya visitkort"),
            (1, "Bygg exempelsidorna"),
            (1, "Granska budgeten för 2027"),
            (2, "Publicera version 0.2"),
        ] {
            b.add(c, title);
        }
        b
    }

    fn count(&self) -> usize {
        self.cols.iter().map(Vec::len).sum()
    }

    /// Column and index of card `id`.
    fn find(&self, id: u32) -> Option<(usize, usize)> {
        self.cols
            .iter()
            .enumerate()
            .find_map(|(c, col)| col.iter().position(|k| k.id == id).map(|i| (c, i)))
    }

    /// Add a card at the bottom of column `c`; `None` for an empty title
    /// or a full board.
    fn add(&mut self, c: usize, title: &str) -> Option<u32> {
        let title = title.trim();
        if title.is_empty() || self.count() >= MAX_CARDS {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.cols[c].push(Card {
            id,
            title: String::from(&title[..title.floor_char_boundary(MAX_TITLE)]),
        });
        Some(id)
    }

    fn remove(&mut self, id: u32) -> Option<Card> {
        let (c, i) = self.find(id)?;
        Some(self.cols[c].remove(i))
    }

    /// Move card `id` to column `c` at `index`, counted among the cards
    /// that are there once the card itself is taken out. True when the
    /// board changed.
    fn move_to(&mut self, id: u32, c: usize, index: usize) -> bool {
        let Some((from, i)) = self.find(id) else {
            return false;
        };
        if from == c && i == index.min(self.cols[c].len() - 1) {
            return false;
        }
        let card = self.cols[from].remove(i);
        let index = index.min(self.cols[c].len());
        self.cols[c].insert(index, card);
        true
    }

    /// The stored form: a format byte, then per column a count and per
    /// card a length byte and the title.
    fn encode(&self) -> Vec<u8> {
        let mut out = vec![FORMAT];
        for col in &self.cols {
            out.push(col.len() as u8);
            for card in col {
                out.push(card.title.len() as u8);
                out.extend_from_slice(card.title.as_bytes());
            }
        }
        out
    }

    /// The board a stored value describes; `None` for anything else.
    fn decode(bytes: &[u8]) -> Option<Self> {
        let (&format, mut rest) = bytes.split_first()?;
        if format != FORMAT {
            return None;
        }
        let mut b = Self::empty();
        for c in 0..COLS {
            let (&n, tail) = rest.split_first()?;
            rest = tail;
            for _ in 0..n {
                let (&len, tail) = rest.split_first()?;
                let (title, tail) = tail.split_at_checked(usize::from(len))?;
                rest = tail;
                b.add(c, core::str::from_utf8(title).ok()?)?;
            }
        }
        rest.is_empty().then_some(b)
    }
}

/// What the pointer is doing with a card.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Press {
    Idle,
    /// Pressed at `at`, not moved far enough to lift the card yet.
    Down {
        id: u32,
        at: (f32, f32),
        grab: (f32, f32),
        size: (f32, f32),
    },
    /// The card floats under the pointer, held at `grab` inside it.
    Drag {
        id: u32,
        grab: (f32, f32),
        size: (f32, f32),
    },
}

/// A card on its way from where it was let go to its slot.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Settle {
    id: u32,
    from: (f32, f32),
    ms: f32,
    /// False until the first frame: its `dt_ms` is the time the page was
    /// at rest, not animation time.
    started: bool,
}

#[derive(Clone, Copy)]
struct Styles {
    title: StyleId,
    status: StyleId,
    lane: StyleId,
    count: StyleId,
    card: StyleId,
    glyph: StyleId,
    plus: StyleId,
    tab: StyleId,
    tab_on: StyleId,
}

/// One column for the current window, scroll and drag.
#[derive(Clone, Debug, Default)]
struct Lane {
    /// False for the columns behind the tabs of a narrow window.
    visible: bool,
    panel: Rect,
    head: Rect,
    /// The scroll viewport of the cards.
    view: Rect,
    content_h: f32,
    /// One rectangle per card, in order; zero for the card being dragged.
    cards: Vec<Rect>,
    /// The gap a dragged card would drop into.
    slot: Option<Rect>,
    field: Rect,
    add: Rect,
}

/// Every rectangle on the page. `frame` draws them, `event` hit-tests them.
#[derive(Clone, Debug, Default)]
struct Layout {
    narrow: bool,
    title: Rect,
    /// Right edge of the header's status text.
    right: f32,
    tabs: [Rect; COLS],
    lanes: [Lane; COLS],
    /// The dragged card under the pointer.
    float: Option<Rect>,
    /// Where the dragged card would land: column and index.
    drop: Option<(usize, usize)>,
}

impl Layout {
    /// The card under `(x, y)`: column and index. A card scrolled out of
    /// its viewport is not there.
    fn card_at(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        self.lanes.iter().enumerate().find_map(|(c, lane)| {
            if !lane.visible || !lane.view.contains(x, y) {
                return None;
            }
            lane.cards
                .iter()
                .position(|r| r.contains(x, y))
                .map(|i| (c, i))
        })
    }
}

/// The delete button in the corner of a card.
fn del_rect(card: Rect) -> Rect {
    Rect::new(card.right() - DEL - 4.0, card.y + 4.0, DEL, DEL)
}

/// The title field inside a card that is being edited.
fn edit_rect(card: Rect) -> Rect {
    card.inset(4.0)
}

/// The part of `r` inside `view`; zero-sized at the nearest edge when
/// nothing is, so the host's pointer focus cannot land on a hidden card.
fn visible_part(r: Rect, view: Rect) -> Rect {
    let x0 = r.x.clamp(view.x, view.right());
    let y0 = r.y.clamp(view.y, view.bottom());
    let x1 = r.right().clamp(view.x, view.right());
    let y1 = r.bottom().clamp(view.y, view.bottom());
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// Ease-out cubic: fast at first, gentle at the end.
fn ease(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

struct Page {
    window: Window,
    theme: Theme,
    s: Styles,
    board: Board,
    /// False when the board could not be read or written: "sparas inte".
    stored: bool,
    /// The column a narrow window shows.
    active: usize,
    scroll: [ScrollView; COLS],
    new: [TextBox; COLS],
    /// The card whose title is being edited, and its field.
    edit: Option<(u32, TextBox)>,
    /// The node the host's focus is on.
    focus: Option<i32>,
    press: Press,
    pointer: (f32, f32),
    settle: Option<Settle>,
    /// The card last pressed and when, for the double click.
    last_press: Option<(u32, i64)>,
}

impl Page {
    fn card_h(&self, card: &Card, w: f32) -> f32 {
        let text_w = (w - 2.0 * CARD_PAD - DEL).max(24.0);
        let text_h = text_measure(self.s.card, text_w, &card.title).map_or(18.0, |s| s.h);
        let h = text_h + 2.0 * CARD_PAD;
        if self.edit.as_ref().is_some_and(|e| e.0 == card.id) {
            h.max(FOOT_H + 8.0)
        } else {
            h
        }
    }

    fn layout(&self) -> Layout {
        let (w, h) = (self.window.width, self.window.height);
        let narrow = w < NARROW;
        let mut top = HEADER_H;
        let mut tabs = [Rect::default(); COLS];
        if narrow {
            let row = Rect::new(MARGIN, top, (w - 2.0 * MARGIN).max(0.0), TAB_H);
            tabs = ui::split::<COLS>(row, 6.0);
            top += TAB_H + 8.0;
        }
        let area = Rect::new(
            MARGIN,
            top,
            (w - 2.0 * MARGIN).max(0.0),
            (h - top - MARGIN).max(0.0),
        );
        let lane_w = if narrow {
            area.w
        } else {
            ((area.w - 2.0 * GAP) / 3.0).clamp(0.0, MAX_LANE)
        };
        let x0 = area.x + (area.w - (3.0 * lane_w + 2.0 * GAP)) / 2.0;
        let panel = |c: usize| {
            if narrow {
                area
            } else {
                Rect::new(x0 + c as f32 * (lane_w + GAP), area.y, lane_w, area.h)
            }
        };

        let (dragged, float) = match self.press {
            Press::Drag { id, grab, size } => (
                Some(id),
                Some(Rect::new(
                    self.pointer.0 - grab.0,
                    self.pointer.1 - grab.1,
                    size.0,
                    size.1,
                )),
            ),
            _ => (None, None),
        };
        // The column the dragged card is over: the one under its centre,
        // or the nearest; in a narrow window the one shown, or a tab.
        let over_tab = float.and_then(|_| {
            tabs.iter()
                .position(|t| t.contains(self.pointer.0, self.pointer.1))
        });
        // Over a tab the card hangs below the tab row, so the tab shows.
        let float = float.map(|fl| match over_tab {
            Some(t) => Rect::new(
                fl.x.clamp(MARGIN, (w - MARGIN - fl.w).max(MARGIN)),
                tabs[t].bottom() + 6.0,
                fl.w,
                fl.h,
            ),
            None => fl,
        });
        let target = float.map(|fl| {
            let cx = fl.x + fl.w / 2.0;
            if narrow {
                over_tab.unwrap_or(self.active)
            } else {
                let mid = |c: usize| panel(c).x + lane_w / 2.0;
                (0..COLS).fold(0, |best, c| {
                    if (mid(c) - cx).abs() < (mid(best) - cx).abs() {
                        c
                    } else {
                        best
                    }
                })
            }
        });

        let mut drop = None;
        let lanes: [Lane; COLS] = core::array::from_fn(|c| {
            let panel = panel(c);
            let foot_y = panel.bottom() - LANE_PAD - FOOT_H;
            let add = Rect::new(panel.right() - LANE_PAD - FOOT_H, foot_y, FOOT_H, FOOT_H);
            let field = Rect::new(
                panel.x + LANE_PAD,
                foot_y,
                (add.x - 8.0 - panel.x - LANE_PAD).max(0.0),
                FOOT_H,
            );
            let view = Rect::new(
                panel.x,
                panel.y + LANE_HEAD,
                panel.w,
                (foot_y - 8.0 - panel.y - LANE_HEAD).max(0.0),
            );
            let card_x = view.x + LANE_PAD;
            let card_w = (view.w - 2.0 * LANE_PAD).max(0.0);
            let cards = &self.board.cols[c];
            let heights: Vec<f32> = cards.iter().map(|k| self.card_h(k, card_w)).collect();
            let start = view.y + CARD_TOP - self.scroll[c].offset();
            // The slot: before the first card whose middle is below the
            // dragged card's middle, in the column as it is without it.
            let slot_at = match (float, target) {
                (Some(fl), Some(t)) if t == c => {
                    let cy = fl.y + fl.h / 2.0;
                    let mut y = start;
                    let mut index = 0;
                    for (k, h) in cards.iter().zip(&heights) {
                        if Some(k.id) == dragged {
                            continue;
                        }
                        if y + h / 2.0 < cy && over_tab.is_none() {
                            index += 1;
                        }
                        y += h + CARD_GAP;
                    }
                    if over_tab.is_some() {
                        index = cards.iter().filter(|k| Some(k.id) != dragged).count();
                    }
                    drop = Some((c, index));
                    Some(index)
                }
                _ => None,
            };
            let mut rects = Vec::with_capacity(cards.len());
            let mut slot = None;
            let mut y = start;
            let mut placed = 0;
            let mut open_slot = |y: &mut f32, placed: usize| {
                if slot_at == Some(placed) && slot.is_none() {
                    let h = float.map_or(0.0, |fl| fl.h);
                    slot = Some(Rect::new(card_x, *y, card_w, h));
                    *y += h + CARD_GAP;
                }
            };
            for (k, h) in cards.iter().zip(&heights) {
                if Some(k.id) == dragged {
                    rects.push(Rect::default());
                    continue;
                }
                open_slot(&mut y, placed);
                rects.push(Rect::new(card_x, y, card_w, *h));
                y += h + CARD_GAP;
                placed += 1;
            }
            open_slot(&mut y, placed);
            Lane {
                visible: !narrow || c == self.active,
                panel,
                head: Rect::new(
                    panel.x + LANE_PAD + 4.0,
                    panel.y + 12.0,
                    (panel.w - 2.0 * LANE_PAD - 8.0).max(0.0),
                    20.0,
                ),
                view,
                content_h: y - start + CARD_TOP,
                cards: rects,
                slot,
                field,
                add,
            }
        });
        // The header lines up with the columns.
        let (left, right) = (lanes[0].panel.x, lanes[COLS - 1].panel.right());
        Layout {
            narrow,
            title: Rect::new(left, 14.0, 120.0, 28.0),
            right,
            tabs,
            lanes,
            float,
            drop,
        }
    }

    /// Keep every column's scroll inside its content.
    fn clamp_scrolls(&mut self) {
        let l = self.layout();
        for (scroll, lane) in self.scroll.iter_mut().zip(&l.lanes) {
            scroll.clamp(lane.view, lane.content_h);
        }
    }

    /// The rectangle of card `id` in the current layout.
    fn card_rect(&self, id: u32) -> Option<(usize, Rect)> {
        let (c, i) = self.board.find(id)?;
        Some((c, self.layout().lanes[c].cards[i]))
    }

    /// Scroll card `id`'s column so the card is in view.
    fn reveal(&mut self, id: u32) {
        let Some((c, i)) = self.board.find(id) else {
            return;
        };
        let l = self.layout();
        let lane = &l.lanes[c];
        self.scroll[c].reveal(lane.cards[i].inset(-6.0), lane.view, lane.content_h);
    }

    fn save(&mut self) {
        let result = state_put(KEY, &self.board.encode());
        if let Err(e) = result {
            let _ = log(Level::Warn, &format!("kanban: state_put refused: {e}"));
        }
        self.stored = result.is_ok();
    }

    /// Add the text of column `c`'s field as a card; true when added.
    fn add_card(&mut self, c: usize) -> bool {
        let Some(id) = self.board.add(c, self.new[c].text()) else {
            return false;
        };
        self.new[c].set_text("");
        self.reveal(id);
        true
    }

    fn delete_card(&mut self, id: u32) -> bool {
        if self.edit.as_ref().is_some_and(|e| e.0 == id) {
            self.finish_edit(false);
        }
        self.board.remove(id).is_some()
    }

    /// Turn card `id` into its title field. The host's focus is on the
    /// card's node already, and the field takes that node's id, so focus
    /// stays where it is (SPEC-10.3.2) and the field is told it has it.
    fn start_edit(&mut self, id: u32) {
        let Some((c, i)) = self.board.find(id) else {
            return;
        };
        let node = node_card(id);
        let mut field = TextBox::new(node)
            .with_max_len(MAX_TITLE)
            .with_text(&self.board.cols[c][i].title);
        field.select_all();
        self.edit = Some((id, field));
        self.press = Press::Idle;
        if self.focus == Some(node) {
            self.tell_field(true);
        }
    }

    /// Hand the title field a `FOCUS` event for its node.
    fn tell_field(&mut self, gained: bool) {
        let Some(id) = self.edit.as_ref().map(|e| e.0) else {
            return;
        };
        let r = self.card_rect(id).map_or(Rect::default(), |(_, r)| r);
        let t = self.theme;
        if let Some((_, field)) = self.edit.as_mut() {
            let focus = Event::Focus {
                node: NodeId(node_card(id)),
                gained,
                keyboard: false,
            };
            field.event(&mut Input::new(focus), edit_rect(r), &t);
        }
    }

    /// Close the title field; with `commit` its text becomes the title
    /// (an empty one keeps the old). True when the board changed.
    fn finish_edit(&mut self, commit: bool) -> bool {
        if self.edit.as_ref().is_some_and(|e| e.1.focused()) {
            // The node becomes a button again: composition ends.
            self.tell_field(false);
        }
        let Some((id, field)) = self.edit.take() else {
            return false;
        };
        let title = field.text().trim();
        let Some((c, i)) = self.board.find(id) else {
            return false;
        };
        if !commit || title.is_empty() || self.board.cols[c][i].title == title {
            return false;
        }
        self.board.cols[c][i].title = String::from(title);
        true
    }

    /// Move card `id` and let it glide from where it is drawn now.
    fn move_card(&mut self, id: u32, c: usize, index: usize, from: Option<(f32, f32)>) -> bool {
        let before = self.card_rect(id);
        if !self.board.move_to(id, c, index) {
            return false;
        }
        let shown = !self.layout().narrow || c == self.active;
        let from = from.or(before.map(|(_, r)| (r.x, r.y)));
        self.settle = match from {
            Some(from) if shown => Some(Settle {
                id,
                from,
                ms: 0.0,
                started: false,
            }),
            _ => None,
        };
        true
    }

    /// An arrow key on the focused card: one step within its column or to
    /// the column beside it. A narrow window follows the card.
    fn step_card(&mut self, id: u32, code: i32) -> bool {
        let Some((c, i)) = self.board.find(id) else {
            return false;
        };
        let (to, index) = match code {
            key::UP if i > 0 => (c, i - 1),
            key::DOWN if i + 1 < self.board.cols[c].len() => (c, i + 1),
            key::LEFT if c > 0 => (c - 1, i.min(self.board.cols[c - 1].len())),
            key::RIGHT if c + 1 < COLS => (c + 1, i.min(self.board.cols[c + 1].len())),
            _ => return false,
        };
        let narrow = self.layout().narrow;
        if narrow && to != c {
            self.active = to;
            self.settle = None;
            self.board.move_to(id, to, index);
        } else if !self.move_card(id, to, index, None) {
            return false;
        }
        self.reveal(id);
        true
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

    /// Let go of a dragged card where the layout says it lands.
    fn drop_card(&mut self) -> bool {
        let Press::Drag { id, .. } = self.press else {
            self.press = Press::Idle;
            return false;
        };
        let l = self.layout();
        self.press = Press::Idle;
        match (l.drop, l.float) {
            (Some((c, index)), Some(fl)) => self.move_card(id, c, index, Some((fl.x, fl.y))),
            _ => false,
        }
    }

    /// Pointer and keys on the cards. True when the board changed.
    fn card_event(&mut self, ui: &mut Input<'_>, l: &Layout) -> bool {
        let Some(event) = ui.unhandled() else {
            return false;
        };
        match event {
            Event::PointerDown { x, y, button, .. } if button == button::PRIMARY => {
                self.pointer = (x, y);
                let Some((c, i)) = l.card_at(x, y) else {
                    self.last_press = None;
                    return false;
                };
                let id = self.board.cols[c][i].id;
                let r = l.lanes[c].cards[i];
                ui.take();
                if self.edit.as_ref().is_some_and(|e| e.0 == id) {
                    return false;
                }
                ui.set_redraw();
                if del_rect(r).contains(x, y) {
                    return self.delete_card(id);
                }
                let now = now_ms();
                let double = self
                    .last_press
                    .is_some_and(|(last, at)| last == id && now - at <= DOUBLE_MS);
                if double {
                    self.last_press = None;
                    self.start_edit(id);
                } else {
                    self.last_press = Some((id, now));
                    self.press = Press::Down {
                        id,
                        at: (x, y),
                        grab: (x - r.x, y - r.y),
                        size: (r.w, r.h),
                    };
                }
                false
            }
            Event::PointerMove { x, y, buttons, .. } => {
                self.pointer = (x, y);
                let held = buttons & button::mask(button::PRIMARY) != 0;
                match self.press {
                    Press::Idle => false,
                    // Released outside the window: the card lands here.
                    _ if !held => {
                        ui.set_redraw();
                        self.drop_card()
                    }
                    Press::Down { id, at, grab, size } => {
                        if (x - at.0).abs() + (y - at.1).abs() > DRAG_START {
                            self.press = Press::Drag { id, grab, size };
                            self.last_press = None;
                            self.settle = None;
                            ui.take();
                            ui.set_redraw();
                        }
                        false
                    }
                    Press::Drag { .. } => {
                        ui.take();
                        ui.set_redraw();
                        false
                    }
                }
            }
            Event::PointerUp { x, y, button, .. } if button == button::PRIMARY => {
                self.pointer = (x, y);
                if self.press == Press::Idle {
                    return false;
                }
                ui.take();
                ui.set_redraw();
                self.drop_card()
            }
            Event::KeyDown { code, .. } => {
                if code == key::ESCAPE && matches!(self.press, Press::Drag { .. }) {
                    self.press = Press::Idle;
                    ui.take();
                    ui.set_redraw();
                    return false;
                }
                let Some((id, del)) = self.focus.and_then(card_of) else {
                    return false;
                };
                if self.edit.is_some() || self.board.find(id).is_none() {
                    return false;
                }
                let changed = match code {
                    key::ENTER | key::SPACE if del => self.delete_card(id),
                    key::DELETE => self.delete_card(id),
                    key::ENTER | F2 if !del => {
                        self.start_edit(id);
                        false
                    }
                    key::UP | key::DOWN | key::LEFT | key::RIGHT if !del => {
                        // The arrows are the card's even at an edge.
                        ui.take();
                        return self.step_card(id, code);
                    }
                    _ => return false,
                };
                ui.take();
                ui.set_redraw();
                changed
            }
            _ => false,
        }
    }

    fn status(&self) -> String {
        let n = self.board.count();
        let mut s = format!("{n} kort");
        if n >= MAX_CARDS {
            s.push_str(" · tavlan är full");
        }
        if !self.stored {
            s.push_str(" · sparas inte");
        }
        s
    }

    fn ring(&self, f: &mut Frame, r: Rect, radius: f32) -> Result<(), VyError> {
        f.rect(
            r.inset(-ui::RING),
            radius + ui::RING,
            self.theme.palette.ring,
        )
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

    /// A card's face at `r`: border, surface, title and delete button.
    fn draw_face(&self, f: &mut Frame, r: Rect, card: &Card) -> Result<(), VyError> {
        f.rect(r, 8.0, CARD_EDGE)?;
        f.rect(r.inset(1.0), 7.0, CARD)?;
        let text_w = (r.w - 2.0 * CARD_PAD - DEL).max(24.0);
        f.text(
            r.x + CARD_PAD,
            r.y + CARD_PAD,
            text_w,
            self.s.card,
            &card.title,
        )?;
        Self::centred(f, del_rect(r), self.s.glyph, "×")
    }

    /// The cards of column `c` inside its scroll viewport, with their nodes.
    fn draw_cards(&mut self, f: &mut Frame, l: &Layout, c: usize) -> Result<(), VyError> {
        let lane = &l.lanes[c];
        let t = self.theme;
        if let Some(slot) = lane.slot {
            f.rect(slot, 8.0, SLOT)?;
            let line = snap(2.0, self.window.scale);
            f.rect(
                Rect::new(slot.x + 8.0, slot.y - line / 2.0 - 3.0, slot.w - 16.0, line),
                line / 2.0,
                t.palette.accent,
            )?;
        }
        for i in 0..self.board.cols[c].len() {
            let id = self.board.cols[c][i].id;
            let r = lane.cards[i];
            let dragged = matches!(self.press, Press::Drag { id: d, .. } if d == id);
            let settling = self.settle.is_some_and(|s| s.id == id);
            if self.edit.as_ref().is_some_and(|e| e.0 == id) {
                f.rect(r, 8.0, CARD_EDGE)?;
                f.rect(r.inset(1.0), 7.0, CARD)?;
                if let Some((_, field)) = self.edit.as_mut() {
                    field.draw(f, edit_rect(r), &t, "Kortets titel")?;
                }
                continue;
            }
            let card = &self.board.cols[c][i];
            let (focus_card, focus_del) = match self.focus.and_then(card_of) {
                Some((k, del)) if k == id => (!del, del),
                _ => (false, false),
            };
            if !dragged && !settling {
                if focus_card {
                    self.ring(f, r, 8.0)?;
                }
                self.draw_face(f, r, card)?;
                if focus_del {
                    self.ring(f, del_rect(r), 6.0)?;
                    f.rect(del_rect(r), 6.0, CARD)?;
                    Self::centred(f, del_rect(r), self.s.glyph, "×")?;
                }
            }
            // A dragged card's node is where the card floats.
            let (node, text) = match l.float {
                Some(fl) if dragged => (fl, format!("{} (flyttas)", card.title)),
                _ => (visible_part(r, lane.view), card.title.clone()),
            };
            f.sem
                .node_with_id(node_card(id), Role::Button, node, &text)?;
            let del = if dragged {
                Rect::new(node.x, node.y, 0.0, 0.0)
            } else {
                visible_part(del_rect(r), lane.view)
            };
            let label = format!("Ta bort: {}", card.title);
            f.sem
                .node_with_id(node_del(id), Role::Button, del, &label)?;
        }
        Ok(())
    }

    fn draw_lane(&mut self, f: &mut Frame, l: &Layout, c: usize) -> Result<(), VyError> {
        let lane = &l.lanes[c];
        let t = self.theme;
        f.rect(lane.panel, 12.0, LANE)?;
        let role = if l.narrow {
            Role::TabPanel
        } else {
            Role::Region
        };
        f.group(role, lane.panel, NAMES[c], |f| {
            let dot = Rect::new(lane.head.x, lane.head.y + 5.0, 10.0, 10.0);
            f.rect(dot, 5.0, DOTS[c])?;
            let name_w = f
                .text(dot.right() + 8.0, lane.head.y, 0.0, self.s.lane, NAMES[c])?
                .w;
            f.sem.node(Role::Heading, lane.head, NAMES[c])?;
            let count = format!("{} kort", self.board.cols[c].len());
            let cx = dot.right() + 8.0 + name_w + 10.0;
            let cw = f.text(cx, lane.head.y + 2.0, 0.0, self.s.count, &count)?.w;
            f.sem.node(
                Role::Text,
                Rect::new(cx, lane.head.y + 2.0, cw, 16.0),
                &count,
            )?;

            let mut scroll = self.scroll[c];
            let drawn = scroll.draw(f, lane.view, &t, lane.content_h, |f| {
                self.draw_cards(f, l, c)
            });
            self.scroll[c] = scroll;
            drawn?;

            self.new[c].draw(f, lane.field, &t, "Nytt kort")?;
            let id = ID_ADD - c as i32;
            if self.focus == Some(id) {
                self.ring(f, lane.add, ui::RADIUS)?;
            }
            f.rect(lane.add, ui::RADIUS, t.palette.accent)?;
            Self::centred(f, lane.add, self.s.plus, "+")?;
            let label = format!("Lägg till kort i {}", NAMES[c]);
            f.sem.node_with_id(id, Role::Button, lane.add, &label)?;
            Ok(())
        })
    }

    fn draw_tabs(&self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let over = l.float.and(l.drop).map(|d| d.0);
        for (c, name) in NAMES.iter().enumerate() {
            let r = l.tabs[c];
            let on = c == self.active;
            let id = ID_TAB - c as i32;
            if self.focus == Some(id) {
                self.ring(f, r, 8.0)?;
            }
            let fill = match over {
                Some(o) if o == c && !on => SLOT,
                _ if on => CARD,
                _ => LANE,
            };
            f.rect(r, 8.0, fill)?;
            let n = self.board.cols[c].len();
            let label = format!("{name} {n}");
            let style = if on { self.s.tab_on } else { self.s.tab };
            Self::centred(f, r, style, &label)?;
            if on {
                let bar = Rect::new(r.x + 12.0, r.bottom() - 4.0, (r.w - 24.0).max(0.0), 3.0);
                f.rect(bar, 1.5, self.theme.palette.accent)?;
            }
            let chosen = if on { self.theme.words.selected } else { "" };
            let text = format!("{name}, {n} kort{chosen}");
            f.sem.node_with_id(id, Role::Tab, r, &text)?;
        }
        Ok(())
    }

    /// The card that is dragged or settling, above everything else.
    fn draw_floating(&self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let (id, at) = match (self.press, l.float, self.settle) {
            (Press::Drag { id, .. }, Some(fl), _) => (id, fl),
            (_, _, Some(s)) => {
                let Some((c, i)) = self.board.find(s.id) else {
                    return Ok(());
                };
                let to = l.lanes[c].cards[i];
                let k = ease(s.ms / SETTLE_MS);
                let x = s.from.0 + (to.x - s.from.0) * k;
                let y = s.from.1 + (to.y - s.from.1) * k;
                (s.id, Rect::new(x, y, to.w, to.h))
            }
            _ => return Ok(()),
        };
        let Some((c, i)) = self.board.find(id) else {
            return Ok(());
        };
        f.rect(at.inset(-1.0).offset(0.0, 4.0), 10.0, SHADOW)?;
        if self.focus == Some(node_card(id)) {
            self.ring(f, at, 8.0)?;
        }
        self.draw_face(f, at, &self.board.cols[c][i])
    }

    fn draw_page(&mut self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        f.text(l.title.x, l.title.y, 0.0, self.s.title, "Tavlan")?;
        f.sem.node(Role::Heading, l.title, "Tavlan")?;
        let status = self.status();
        let size = text_measure(self.s.status, 0.0, &status)?;
        let x = (l.right - size.w).max(l.title.right());
        f.text(x, 21.0, 0.0, self.s.status, &status)?;
        f.sem
            .node(Role::Text, Rect::new(x, 21.0, size.w, size.h), &status)?;
        if l.narrow {
            self.draw_tabs(f, l)?;
        }
        for c in 0..COLS {
            if l.lanes[c].visible {
                self.draw_lane(f, l, c)?;
            }
        }
        Ok(())
    }
}

impl App for Page {
    fn init(window: Window, _abi_minor: i32) -> Result<Self, VyError> {
        let p = Palette::LIGHT;
        let theme = Theme::custom(p, Font::Sans, 14.0)?;
        // The stored board, a sample on a first visit, and "sparas inte"
        // when the host refuses storage.
        let mut buf = vec![0u8; 2 + COLS + MAX_CARDS * (MAX_TITLE + 1)];
        let (board, stored) = match state_get(KEY, &mut buf) {
            Ok(n) => (
                buf.get(..n)
                    .and_then(Board::decode)
                    .unwrap_or_else(Board::sample),
                true,
            ),
            Err(VyError::Resource) => (Board::sample(), true),
            Err(e) => {
                let _ = log(Level::Info, &format!("kanban: no storage ({e})"));
                (Board::sample(), false)
            }
        };
        let field = |c: i32| TextBox::new(ID_NEW - c).with_max_len(MAX_TITLE);
        Ok(Self {
            window,
            theme,
            s: Styles {
                title: text_style(Font::Sans, 22.0, 700, p.ink)?,
                status: text_style(Font::Sans, 13.0, 400, p.muted)?,
                lane: text_style(Font::Sans, 14.0, 700, p.ink)?,
                count: text_style(Font::Sans, 12.0, 400, p.muted)?,
                card: text_style(Font::Sans, 14.0, 400, p.ink)?,
                glyph: text_style(Font::Sans, 17.0, 400, p.muted)?,
                plus: text_style(Font::Sans, 20.0, 700, p.on_accent)?,
                tab: text_style(Font::Sans, 14.0, 600, p.muted)?,
                tab_on: text_style(Font::Sans, 14.0, 600, p.accent)?,
            },
            board,
            stored,
            active: 0,
            scroll: [ScrollView::new(); COLS],
            new: [field(0), field(1), field(2)],
            edit: None,
            focus: None,
            press: Press::Idle,
            pointer: (0.0, 0.0),
            settle: None,
            last_press: None,
        })
    }

    fn frame(&mut self, f: &mut Frame, dt_ms: i32) -> Result<(), VyError> {
        if let Some(s) = self.settle.as_mut() {
            if s.started {
                s.ms += dt_ms.clamp(0, 50) as f32;
            }
            s.started = true;
            if s.ms >= SETTLE_MS {
                self.settle = None;
            } else {
                request_frame();
            }
        }
        let l = self.layout();
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        f.rect(window, 0.0, PAGE)?;
        f.group(Role::Main, window, "", |f| self.draw_page(f, &l))?;
        self.draw_floating(f, &l)
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        let t = self.theme;
        let mut ui = Input::new(event);
        let mut changed = false;
        let is_focus = matches!(event, Event::Focus { .. });
        if let Event::Focus { node, gained, .. } = event {
            if gained {
                self.focus = Some(node.0);
            } else if self.focus == Some(node.0) {
                self.focus = None;
            }
            ui.set_redraw();
        }

        // The title field first: it lies over its card.
        if let Some(id) = self.edit.as_ref().map(|e| e.0) {
            let r = self.card_rect(id).map_or(Rect::default(), |(_, r)| r);
            let escape = matches!(
                ui.unhandled(),
                Some(Event::KeyDown {
                    code: key::ESCAPE,
                    ..
                })
            );
            let lost = matches!(event, Event::Focus { node, gained: false, .. } if node.0 == node_card(id));
            let mut done = None;
            if escape {
                ui.take();
                done = Some(false);
            } else if let Some((_, field)) = self.edit.as_mut() {
                if field.event(&mut ui, edit_rect(r), &t).activated() || lost {
                    done = Some(true);
                }
            }
            if let Some(commit) = done {
                changed |= self.finish_edit(commit);
                ui.set_redraw();
            }
        }

        let l = self.layout();
        for c in 0..COLS {
            if l.narrow && self.pressed(&ui, ID_TAB - c as i32, l.tabs[c]) {
                ui.take();
                ui.set_redraw();
                self.active = c;
            }
            let lane = &l.lanes[c];
            if !lane.visible && !is_focus {
                continue;
            }
            let enter = self.new[c].event(&mut ui, lane.field, &t).activated();
            if enter || self.pressed(&ui, ID_ADD - c as i32, lane.add) {
                ui.take();
                ui.set_redraw();
                changed |= self.add_card(c);
            }
        }
        changed |= self.card_event(&mut ui, &l);
        for c in 0..COLS {
            let lane = &l.lanes[c];
            if lane.visible || is_focus {
                self.scroll[c].event(&mut ui, lane.view, &t, lane.content_h);
            }
        }
        // The keyboard moved focus to a card: bring it into view.
        if let Event::Focus {
            node,
            gained: true,
            keyboard: true,
        } = event
        {
            if let Some((id, _)) = card_of(node.0) {
                self.reveal(id);
            }
        }
        if changed {
            self.save();
            self.clamp_scrolls();
        }
        ui.redraw() || changed
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
        self.clamp_scrolls();
    }
}

vy_sdk::export!(Page);

#[cfg(test)]
mod tests;
