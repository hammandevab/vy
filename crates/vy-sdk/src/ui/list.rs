//! A virtualised list with one selected row.

use super::scroll::ScrollView;
use super::{clip_str, ring, Input, Response, Theme, PAD, RADIUS};
use crate::event::{button, key, Event};
use crate::frame::Frame;
use crate::sem::Role;
use crate::{math, Rect, VyError};
use alloc::string::String;
use core::ops::Range;
use vy_abi::limits;

/// The rows of a [`List`]: how many there are, each one's label, and (if
/// the default single line of text is not enough) how a row is drawn.
///
/// `[&str]`, `[&str; N]` and `[String]` are rows already: pass
/// `&items[..]` or `&ITEMS`.
pub trait Rows {
    /// Number of rows.
    fn len(&self) -> usize;

    /// True without rows.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Write row `i`'s label – the text of its `listitem` node – into
    /// `out`, which is empty; the list reuses one buffer for every row.
    fn label(&self, i: usize, out: &mut String);

    /// Draw row `i` in `r` (clipped to the list; the list has drawn the
    /// selection behind it). The default draws `label` on one line.
    fn draw_row(
        &self,
        f: &mut Frame,
        theme: &Theme,
        i: usize,
        r: Rect,
        label: &str,
        selected: bool,
    ) -> Result<(), VyError> {
        let _ = (i, selected);
        f.text(
            r.x + PAD,
            r.y + (r.h - theme.line_h) / 2.0,
            0.0,
            theme.text,
            label,
        )?;
        Ok(())
    }
}

impl Rows for [&str] {
    fn len(&self) -> usize {
        <[&str]>::len(self)
    }
    fn label(&self, i: usize, out: &mut String) {
        out.push_str(self[i]);
    }
}

impl<const N: usize> Rows for [&str; N] {
    fn len(&self) -> usize {
        N
    }
    fn label(&self, i: usize, out: &mut String) {
        out.push_str(self[i]);
    }
}

impl Rows for [String] {
    fn len(&self) -> usize {
        <[String]>::len(self)
    }
    fn label(&self, i: usize, out: &mut String) {
        out.push_str(&self[i]);
    }
}

/// A list of equal rows with at most one selected, scrolled with the wheel,
/// the keys and a scroll bar. Only the rows in view are drawn, but every
/// row is registered as a `listitem` node under a `list` node
/// (SPEC-10.1.1: the whole tree, off-screen nodes too), so a frame costs one
/// `sem_node` per row; page through data rather than listing tens of
/// thousands of rows.
///
/// - A primary press on a row selects it (`Changed`), on the selected row
///   activates it (`Activated`).
/// - `list` and `listitem` are not focusable (SPEC-10.3.2), so Tab never
///   reaches the list: it takes keys while *active*, after a press inside
///   it that gave no node the host's focus, or after
///   [`List::set_active`]. Active, it has a ring; Up/Down move the
///   selection, Page Up/Down by a page, Home/End to the ends (`Changed`,
///   scrolled into view), Enter activates the selected row.
/// - The selected row's node text ends in [`super::Words::selected`].
/// - Drawn with a 1 px border like a text field; rows are clipped inside it.
#[derive(Clone, Debug, PartialEq)]
pub struct List {
    row_h: f32,
    selected: Option<usize>,
    scroll: ScrollView,
    label: String,
}

impl List {
    /// An empty selection over rows `row_h` high (40 fits one line of the
    /// default theme with room around it).
    pub fn new(row_h: f32) -> Self {
        Self {
            row_h: row_h.max(1.0),
            selected: None,
            scroll: ScrollView::new(),
            label: String::new(),
        }
    }

    /// Height of a row.
    pub fn row_h(&self) -> f32 {
        self.row_h
    }

    /// The selected row.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// True while the list takes keys.
    pub fn active(&self) -> bool {
        self.scroll.active()
    }

    /// Make the list take keys (a page whose list is its main control) or stop.
    pub fn set_active(&mut self, active: bool) {
        self.scroll.set_active(active);
    }

    /// The list's scroll position.
    pub fn offset(&self) -> f32 {
        self.scroll.offset()
    }

    fn content_h(&self, len: usize) -> f32 {
        len as f32 * self.row_h
    }

    /// Row `i` in window coordinates for the list in `r`, as scrolled now.
    pub fn row_rect(&self, r: Rect, i: usize) -> Rect {
        Rect::new(
            r.x,
            r.y + i as f32 * self.row_h - self.scroll.offset(),
            r.w,
            self.row_h,
        )
    }

    /// The row under `(x, y)`, if any.
    pub fn row_at(&self, r: Rect, len: usize, x: f32, y: f32) -> Option<usize> {
        if !r.contains(x, y) {
            return None;
        }
        let i = ((y - r.y + self.scroll.offset()) / self.row_h) as usize;
        (i < len).then_some(i)
    }

    /// The rows at least partly in view.
    pub fn visible(&self, r: Rect, len: usize) -> Range<usize> {
        let first = math::floor(self.scroll.offset() / self.row_h) as usize;
        let last = math::ceil((self.scroll.offset() + r.h) / self.row_h) as usize;
        first.min(len)..last.min(len)
    }

    /// Select row `i` (clamped to the rows; `None` clears) and scroll it
    /// into view; true when the selection changed.
    pub fn select(&mut self, i: Option<usize>, r: Rect, len: usize) -> bool {
        let i = i.filter(|_| len > 0).map(|i| i.min(len - 1));
        let changed = i != self.selected;
        self.selected = i;
        if let Some(i) = i {
            let row = self.row_rect(r, i);
            self.scroll.reveal(row, r, self.content_h(len));
        }
        changed
    }

    fn page_rows(&self, r: Rect) -> usize {
        ((r.h / self.row_h) as usize).saturating_sub(1).max(1)
    }

    /// Handle an event for the list in `r` over `rows`.
    pub fn event<R: Rows + ?Sized>(
        &mut self,
        ui: &mut Input<'_>,
        r: Rect,
        theme: &Theme,
        rows: &R,
    ) -> Response {
        let len = rows.len();
        let content_h = self.content_h(len);
        let response = match ui.event() {
            Event::PointerDown { x, y, button, .. }
                if !ui.taken()
                    && button == button::PRIMARY
                    && r.contains(x, y)
                    && !ScrollView::on_bar(r, content_h, x, y) =>
            {
                ui.take();
                let was = self.scroll.active();
                self.scroll.activity.press(true);
                match self.row_at(r, len, x, y) {
                    Some(i) if self.selected == Some(i) => Response::Activated,
                    Some(i) => {
                        self.select(Some(i), r, len);
                        Response::Changed
                    }
                    None if was != self.scroll.active() => Response::Redraw,
                    None => Response::Nothing,
                }
            }
            Event::KeyDown { code, .. } if !ui.taken() && self.scroll.active() => {
                let last = len.saturating_sub(1);
                let current = self.selected;
                let target = match (code, current) {
                    (key::ENTER, Some(_)) => {
                        ui.take();
                        return ui.respond(Response::Activated);
                    }
                    (key::DOWN, None) | (key::HOME, _) => 0,
                    (key::UP, None) => 0,
                    (key::END, _) => last,
                    (key::DOWN, Some(i)) => (i + 1).min(last),
                    (key::UP, Some(i)) => i.saturating_sub(1),
                    (key::PAGE_DOWN, i) => (i.unwrap_or(0) + self.page_rows(r)).min(last),
                    (key::PAGE_UP, i) => i.unwrap_or(0).saturating_sub(self.page_rows(r)),
                    _ => return Response::Nothing,
                };
                ui.take();
                if len > 0 && self.select(Some(target), r, len) {
                    Response::Changed
                } else {
                    Response::Nothing
                }
            }
            // Focus (activity), the wheel, the scroll bar, hover: the area's.
            _ => return self.scroll.event(ui, r, theme, content_h),
        };
        ui.respond(response)
    }

    /// Draw the rows in view in `r` and register the `list` node with a
    /// `listitem` for every row.
    pub fn draw<R: Rows + ?Sized>(
        &mut self,
        f: &mut Frame,
        r: Rect,
        theme: &Theme,
        rows: &R,
    ) -> Result<(), VyError> {
        let p = &theme.palette;
        let len = rows.len();
        let content_h = self.content_h(len);
        self.scroll.clamp(r, content_h);
        if self.scroll.active() {
            ring(f, r, RADIUS, theme)?;
        }
        f.rect(r, RADIUS, p.border)?;
        let inner = r.inset(1.0);
        f.rect(inner, RADIUS - 1.0, p.surface)?;
        let visible = self.visible(r, len);
        let mut label = core::mem::take(&mut self.label);
        let this = &*self;
        let drawn = f.group(Role::List, r, "", |f| {
            f.clip_push(inner, RADIUS - 1.0)?;
            let rows_drawn = (|| {
                for i in 0..len {
                    label.clear();
                    rows.label(i, &mut label);
                    let row = this.row_rect(r, i);
                    let selected = this.selected == Some(i);
                    if visible.contains(&i) {
                        if selected {
                            f.rect(row, 0.0, p.selection)?;
                        }
                        f.rect(
                            Rect::new(row.x, row.bottom() - 1.0, row.w, 1.0),
                            0.0,
                            p.rule,
                        )?;
                        rows.draw_row(f, theme, i, row, &label, selected)?;
                    }
                    if selected {
                        label.push_str(theme.words.selected);
                    }
                    let text = clip_str(&label, limits::MAX_SEM_TEXT_BYTES as usize);
                    f.sem.node(Role::ListItem, row, text)?;
                }
                Ok(())
            })();
            f.clip_pop()?;
            rows_drawn
        });
        self.label = label;
        drawn?;
        self.scroll.draw_bar(f, r, theme, content_h)
    }
}
