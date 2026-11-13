//! A text field: one line or several, with caret, selection, IME and the
//! clipboard.

use super::{clip_str, focus_of, Input, Response, Theme, CONTROL_H, PAD, RADIUS};
use crate::api::{clipboard_write, ime_begin, ime_end, text_caret, text_measure};
use crate::event::{button, key, Event, Modifiers};
use crate::frame::Frame;
use crate::sem::{NodeId, Role};
use crate::{math, Rect, StyleId, VyError};
use alloc::string::String;
use alloc::vec::Vec;
use core::marker::PhantomData;
use core::ops::Range;
use vy_abi::limits;

/// Longest text a one-line field takes by default, in bytes.
pub const MAX_LINE_BYTES: usize = 1024;
/// Longest text a multi-line field takes by default, in bytes.
pub const MAX_MULTILINE_BYTES: usize = 4096;
/// Width of the block that shows a selected line break.
const NEWLINE_W: f32 = 6.0;

/// Visual lines of a multi-line field, wrapped at the field's width and
/// kept until the text, the width or the style changes. A paragraph that
/// did not change (found at its old index or one away, as after typing,
/// Enter or joining two paragraphs) is not wrapped again, so typing in one
/// paragraph costs one `text_measure` per word of that paragraph.
#[derive(Clone, Debug, Default)]
struct Wrap {
    key: Option<(u64, u32, i32)>,
    /// `[start, end)` byte ranges into the text, without line breaks.
    lines: Vec<(usize, usize)>,
    /// Each paragraph's (hash, length) and lines relative to its start,
    /// in text order.
    paras: Vec<Para>,
    paras_key: Option<(u32, i32)>,
}

/// One paragraph's wrapped lines, keyed by its hash and length.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Para {
    key: (u64, usize),
    lines: Vec<(usize, usize)>,
}

/// A field's state and all of its logic, whatever its line mode: the one
/// copy of the editing code a page carries, however many fields and
/// whichever modes it has. The mode comes in as an argument from
/// [`TextBox`], whose type knows it, so a page with one-line fields only
/// still carries no wrapping code: every call says `false` and the
/// compiler drops the other half.
#[derive(Clone, Debug)]
struct Field {
    id: i32,
    text: String,
    caret: usize,
    anchor: usize,
    compose: String,
    compose_caret: usize,
    max_len: usize,
    focused: bool,
    dragging: bool,
    hover: bool,
    reveal: bool,
    scroll_x: f32,
    scroll_y: f32,
    preferred_x: Option<f32>,
    ime: Option<Rect>,
    last_copy: Option<Result<(), VyError>>,
    revision: u64,
    wrap: Wrap,
}

/// A text field with a `textbox` node of a fixed id whose text is the
/// value (at most 4 KB of it, SPEC-9.3.2).
///
/// - **Typing** arrives as `TEXT_INPUT` (SPEC-5.1.6) and replaces the
///   selection; Ctrl+V comes the same way, the host delivers the clipboard
///   as text (SPEC-8.2.4). `IME_COMPOSE` shows the composition underlined
///   at the caret until the text is committed. The field opens composition
///   with `ime_begin` over its rectangle when it gains the host's focus and
///   closes it with `ime_end` when it loses it (SPEC-5.2.6), so the page's
///   manifest must request `ime` (SPEC-10.4.2).
/// - **Keys** while focused: arrows (Ctrl/Cmd: by word), Home/End (Ctrl:
///   whole text), Up/Down between lines, Backspace/Delete (Ctrl: a word),
///   Shift with any movement extends the selection, Ctrl+A selects all,
///   Enter activates a one-line field and breaks the line in a multi-line
///   one. Escape is left to the page.
/// - **Clipboard.** Ctrl+C writes the selection with `clipboard_write`
///   while the page handles that `KEY_DOWN`, which SPEC-8.2.4 allows
///   without capability `clipboard`; Ctrl+X does the same and removes the
///   selection only when the write succeeded. [`TextBox::last_copy`] says
///   how it went.
/// - **Pointer.** A primary press puts the caret under the pointer
///   (Shift extends), dragging selects; the wheel scrolls a multi-line
///   field under the pointer.
/// - **Look.** A border that becomes 2 px of the accent colour while
///   focused (the field's focus mark, SPEC-10.3.3), the placeholder while
///   empty, the selection behind the text while focused. A one-line field
///   scrolls sideways to keep the caret in view; a multi-line one wraps at
///   word boundaries and scrolls down.
///
/// The line mode is part of the type: `TextBox` (one line, the default) and
/// `TextBox<Multiline>`, so a page with one-line fields only carries no
/// wrapping code.
#[derive(Clone, Debug)]
pub struct TextBox<L: Lines = OneLine> {
    field: Field,
    lines: PhantomData<L>,
}

/// The line mode of a [`TextBox`]: [`OneLine`] or [`Multiline`].
pub trait Lines: Copy + core::fmt::Debug + 'static {
    /// True when the field wraps and takes line breaks.
    const MULTI: bool;
    /// Most bytes a field of this mode takes unless `with_max_len` says
    /// otherwise.
    const MAX_BYTES: usize;
}

/// A one-line field: Enter activates it, a line break typed or pasted
/// becomes a space, and the text scrolls sideways.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OneLine {}

/// A multi-line field: Enter breaks the line, the text wraps at word
/// boundaries and scrolls down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Multiline {}

impl Lines for OneLine {
    const MULTI: bool = false;
    const MAX_BYTES: usize = MAX_LINE_BYTES;
}

impl Lines for Multiline {
    const MULTI: bool = true;
    const MAX_BYTES: usize = MAX_MULTILINE_BYTES;
}

impl TextBox<OneLine> {
    /// An empty one-line field with node id `id` (negative, SPEC-5.2.13).
    pub fn new(id: i32) -> Self {
        Self::empty(id)
    }
}

impl TextBox<Multiline> {
    /// An empty multi-line field with node id `id`; its type is
    /// `TextBox<Multiline>`.
    pub fn multiline(id: i32) -> Self {
        Self::empty(id)
    }
}

impl<L: Lines> TextBox<L> {
    fn empty(id: i32) -> Self {
        Self {
            field: Field::empty(id, L::MAX_BYTES),
            lines: PhantomData,
        }
    }

    /// The same field holding `text`, caret at its end.
    pub fn with_text(mut self, text: &str) -> Self {
        self.set_text(text);
        self
    }

    /// The same field taking at most `max_len` bytes; longer input is cut
    /// at a character boundary.
    pub fn with_max_len(mut self, max_len: usize) -> Self {
        self.field.max_len = max_len;
        let t = String::from(self.field.text.as_str());
        self.set_text(&t);
        self
    }

    /// The node id `Event::Focus` names.
    pub fn id(&self) -> NodeId {
        NodeId(self.field.id)
    }

    /// The value.
    pub fn text(&self) -> &str {
        &self.field.text
    }

    /// True for a multi-line field.
    pub fn is_multiline(&self) -> bool {
        L::MULTI
    }

    /// Most bytes the field takes.
    pub fn max_len(&self) -> usize {
        self.field.max_len
    }

    /// Replace the value (cleaned and cut like typed text), caret at its
    /// end, no selection.
    pub fn set_text(&mut self, text: &str) {
        self.field.set_text(text, L::MULTI);
    }

    /// True while the field has the host's focus.
    pub fn focused(&self) -> bool {
        self.field.focused
    }

    /// The caret as a byte offset into the value.
    pub fn caret(&self) -> usize {
        self.field.caret
    }

    /// The selected bytes; empty when nothing is selected.
    pub fn selection(&self) -> Range<usize> {
        self.field.selection()
    }

    /// The selected text.
    pub fn selected_text(&self) -> &str {
        &self.field.text[self.field.selection()]
    }

    /// Select `range` (clamped to character boundaries); the caret goes to
    /// its end.
    pub fn select(&mut self, range: Range<usize>) {
        self.field.select(range);
    }

    /// Select the whole value, as Ctrl+A does.
    pub fn select_all(&mut self) {
        self.field.select_all();
    }

    /// The outcome of the last Ctrl+C or Ctrl+X: `Ok` when the selection
    /// reached the clipboard, the host's refusal otherwise (a host without
    /// a clipboard refuses), `None` before the first.
    pub fn last_copy(&self) -> Option<Result<(), VyError>> {
        self.field.last_copy
    }

    /// Handle an event for the field in `r`: `Changed` when the value
    /// changed, `Activated` on Enter in a one-line field.
    pub fn event(&mut self, ui: &mut Input<'_>, r: Rect, theme: &Theme) -> Response {
        self.field.event(ui, r, theme, L::MULTI)
    }

    /// Draw the field in `r`, with `placeholder` while it is empty, and
    /// register its node.
    pub fn draw(
        &mut self,
        f: &mut Frame,
        r: Rect,
        theme: &Theme,
        placeholder: &str,
    ) -> Result<(), VyError> {
        self.field.draw(f, r, theme, placeholder, L::MULTI)
    }
}

impl Field {
    fn empty(id: i32, max_len: usize) -> Self {
        Self {
            id,
            text: String::new(),
            caret: 0,
            anchor: 0,
            compose: String::new(),
            compose_caret: 0,
            max_len,
            focused: false,
            dragging: false,
            hover: false,
            reveal: false,
            scroll_x: 0.0,
            scroll_y: 0.0,
            preferred_x: None,
            ime: None,
            last_copy: None,
            revision: 0,
            wrap: Wrap::default(),
        }
    }

    fn set_text(&mut self, text: &str, multi: bool) {
        self.text.clear();
        self.caret = 0;
        self.anchor = 0;
        self.compose.clear();
        self.insert(text, multi);
        self.edited();
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }

    fn select(&mut self, range: Range<usize>) {
        self.anchor = self.text.floor_char_boundary(range.start);
        self.caret = self.text.floor_char_boundary(range.end.max(range.start));
        self.reveal = true;
    }

    fn select_all(&mut self) {
        self.select(0..self.text.len());
    }

    // ---- editing --------------------------------------------------------

    fn edited(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.reveal = true;
        self.preferred_x = None;
    }

    /// Insert `s` at the caret, replacing the selection; `\r\n` and a lone
    /// `\r` are one line break, a line break becomes a space in a one-line
    /// field, other control characters are dropped, and what does not fit
    /// `max_len` is cut. False when nothing changed.
    fn insert(&mut self, s: &str, multi: bool) -> bool {
        let sel = self.selection();
        let room = self
            .max_len
            .saturating_sub(self.text.len() - (sel.end - sel.start));
        let mut clean = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            let c = match c {
                '\r' if chars.peek() == Some(&'\n') => continue,
                '\n' | '\r' if multi => '\n',
                '\n' | '\r' | '\t' => ' ',
                c if c.is_control() => continue,
                c => c,
            };
            if clean.len() + c.len_utf8() > room {
                break;
            }
            clean.push(c);
        }
        if clean.is_empty() {
            return false;
        }
        self.text.replace_range(sel.clone(), &clean);
        self.caret = sel.start + clean.len();
        self.anchor = self.caret;
        self.edited();
        true
    }

    fn delete(&mut self, range: Range<usize>) -> Response {
        if range.is_empty() {
            return Response::Nothing;
        }
        self.text.replace_range(range.clone(), "");
        self.caret = range.start;
        self.anchor = range.start;
        self.edited();
        Response::Changed
    }

    fn prev_char(&self, o: usize) -> usize {
        self.text[..o]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next_char(&self, o: usize) -> usize {
        self.text[o..]
            .chars()
            .next()
            .map_or(self.text.len(), |c| o + c.len_utf8())
    }

    /// Start of the word before `o`: back over spaces, then over the word.
    fn prev_word(&self, o: usize) -> usize {
        let before = &self.text[..o];
        let trimmed = before.trim_end_matches(char::is_whitespace);
        trimmed
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map_or(0, |(i, c)| i + c.len_utf8())
    }

    /// Start of the word after `o`: over the word, then over spaces.
    fn next_word(&self, o: usize) -> usize {
        let after = &self.text[o..];
        let word = after.find(char::is_whitespace).unwrap_or(after.len());
        let rest = &after[word..];
        o + word + (rest.len() - rest.trim_start_matches(char::is_whitespace).len())
    }

    fn move_to(&mut self, o: usize, extend: bool) -> Response {
        let before = (self.caret, self.anchor);
        self.caret = o;
        if !extend {
            self.anchor = o;
        }
        self.reveal = true;
        self.preferred_x = None;
        if (self.caret, self.anchor) == before {
            Response::Nothing
        } else {
            Response::Redraw
        }
    }

    fn copy(&mut self, cut: bool) -> Response {
        let sel = self.selection();
        if sel.is_empty() {
            return Response::Nothing;
        }
        let result = clipboard_write(&self.text[sel.clone()]);
        self.last_copy = Some(result);
        if cut && result.is_ok() {
            self.delete(sel)
        } else {
            Response::Redraw
        }
    }

    // ---- geometry -------------------------------------------------------

    fn pad_y(theme: &Theme) -> f32 {
        ((CONTROL_H - theme.line_h) / 2.0).max(4.0)
    }

    /// Where the first line's text starts.
    fn origin(r: Rect, theme: &Theme, multi: bool) -> (f32, f32) {
        let y = if multi {
            r.y + Self::pad_y(theme)
        } else {
            r.y + (r.h - theme.line_h) / 2.0
        };
        (r.x + PAD, y)
    }

    fn text_w(r: Rect) -> f32 {
        (r.w - 2.0 * PAD).max(1.0)
    }

    fn view_h(r: Rect, theme: &Theme) -> f32 {
        (r.h - 2.0 * Self::pad_y(theme)).max(theme.line_h)
    }

    fn content_h(&self, theme: &Theme) -> f32 {
        self.wrap.lines.len() as f32 * theme.line_h
    }

    fn layout(&mut self, theme: &Theme, width: f32, multi: bool) -> Result<(), VyError> {
        let key = (self.revision, width.to_bits(), theme.text.0);
        if self.wrap.key == Some(key) {
            return Ok(());
        }
        self.wrap.key = None;
        self.wrap.lines.clear();
        if !multi {
            self.wrap.lines.push((0, self.text.len()));
            self.wrap.key = Some(key);
            return Ok(());
        }
        if self.wrap.paras_key != Some((width.to_bits(), theme.text.0)) {
            self.wrap.paras.clear();
            self.wrap.paras_key = Some((width.to_bits(), theme.text.0));
        }
        let mut old = core::mem::take(&mut self.wrap.paras);
        let mut start = 0;
        loop {
            let end = self.text[start..]
                .find('\n')
                .map_or(self.text.len(), |i| start + i);
            let para = &self.text[start..end];
            let key = (fnv1a(para), para.len());
            let i = self.wrap.paras.len();
            let near = [i, i + 1, i.wrapping_sub(1)];
            let lines = match near
                .iter()
                .find(|&&j| old.get(j).is_some_and(|p| p.key == key))
            {
                Some(&j) => {
                    // Used once: a repeated paragraph further on takes its own.
                    old[j].key = (0, usize::MAX);
                    core::mem::take(&mut old[j].lines)
                }
                None => wrap_paragraph(theme.text, width, para)?,
            };
            self.wrap
                .lines
                .extend(lines.iter().map(|&(a, b)| (start + a, start + b)));
            self.wrap.paras.push(Para { key, lines });
            if end == self.text.len() {
                break;
            }
            start = end + 1;
        }
        self.wrap.key = Some(key);
        Ok(())
    }

    /// The visual line holding offset `o` (a soft break belongs to the
    /// line after it).
    fn line_of(&self, o: usize) -> usize {
        self.wrap.lines.iter().rposition(|l| l.0 <= o).unwrap_or(0)
    }

    fn line_str(&self, k: usize) -> (usize, &str) {
        let (s, e) = self.wrap.lines.get(k).copied().unwrap_or((0, 0));
        (s, &self.text[s..e])
    }

    /// The offset in visual line `k` nearest to `x` (relative to the
    /// line's start), found with `text_caret` by bisection.
    fn offset_in_line(&self, theme: &Theme, k: usize, x: f32) -> usize {
        let (s, line) = self.line_str(k);
        let mut bounds: Vec<usize> = line.char_indices().map(|(i, _)| i).collect();
        bounds.push(line.len());
        let cx = |i: usize| caret_x(theme.text, line, bounds[i]);
        let (mut lo, mut hi) = (0, bounds.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if cx(mid) < x {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let pick = if lo == 0 {
            0
        } else if lo < bounds.len() && cx(lo) - x < x - cx(lo - 1) {
            lo
        } else {
            lo - 1
        };
        let mut o = s + bounds[pick];
        // The end of a soft-wrapped line is the next line's start: keep the
        // caret on this line.
        let end = s + line.len();
        let soft = self.wrap.lines.get(k + 1).is_some_and(|n| n.0 == end);
        if o == end && soft && bounds.len() > 1 {
            o = s + bounds[bounds.len() - 2];
        }
        o
    }

    fn offset_at(&mut self, x: f32, y: f32, r: Rect, theme: &Theme, multi: bool) -> usize {
        if self.layout(theme, Self::text_w(r), multi).is_err() {
            return self.caret;
        }
        let (tx, ty) = Self::origin(r, theme, multi);
        let k = if multi {
            let row = math::floor((y - ty + self.scroll_y) / theme.line_h);
            (row.max(0.0) as usize).min(self.wrap.lines.len().saturating_sub(1))
        } else {
            0
        };
        let scroll_x = if multi { 0.0 } else { self.scroll_x };
        self.offset_in_line(theme, k, x - tx + scroll_x)
    }

    fn vertical(
        &mut self,
        down: bool,
        extend: bool,
        r: Rect,
        theme: &Theme,
        multi: bool,
    ) -> Response {
        if !multi || self.layout(theme, Self::text_w(r), multi).is_err() {
            return Response::Nothing;
        }
        let k = self.line_of(self.caret);
        let (s, line) = self.line_str(k);
        let x = self
            .preferred_x
            .unwrap_or_else(|| caret_x(theme.text, line, self.caret - s));
        let o = if !down && k == 0 {
            0
        } else if down && k + 1 >= self.wrap.lines.len() {
            self.text.len()
        } else {
            self.offset_in_line(theme, if down { k + 1 } else { k - 1 }, x)
        };
        let response = self.move_to(o, extend);
        self.preferred_x = Some(x);
        response
    }

    // ---- events ---------------------------------------------------------

    fn set_focused(&mut self, gained: bool, r: Rect) -> Response {
        self.focused = gained;
        if gained {
            self.ime = ime_begin(r).ok().map(|()| r);
        } else {
            self.dragging = false;
            self.compose.clear();
            // Always paired with the `ime_begin` of the focus: without
            // `ime` the host refuses both and logs it (SPEC-5.3.1).
            self.ime = None;
            let _ = ime_end();
        }
        Response::Redraw
    }

    fn key(
        &mut self,
        ui: &mut Input<'_>,
        code: i32,
        m: Modifiers,
        r: Rect,
        theme: &Theme,
        multi: bool,
    ) -> Response {
        let extend = m.shift();
        if m.shortcut() {
            let response = match code {
                key::A => {
                    self.select_all();
                    Response::Redraw
                }
                key::C | key::X => self.copy(code == key::X),
                key::LEFT => self.move_to(self.prev_word(self.caret), extend),
                key::RIGHT => self.move_to(self.next_word(self.caret), extend),
                key::HOME => self.move_to(0, extend),
                key::END => self.move_to(self.text.len(), extend),
                key::BACKSPACE if self.selection().is_empty() => {
                    self.delete(self.prev_word(self.caret)..self.caret)
                }
                key::DELETE if self.selection().is_empty() => {
                    self.delete(self.caret..self.next_word(self.caret))
                }
                key::BACKSPACE | key::DELETE => self.delete(self.selection()),
                // Ctrl+V never arrives from a window (the host turns it into
                // TEXT_INPUT); other shortcuts are the page's.
                _ => return Response::Nothing,
            };
            ui.take();
            return response;
        }
        if code == key::ESCAPE {
            return Response::Nothing;
        }
        ui.take();
        let sel = self.selection();
        match code {
            key::ENTER if !multi => Response::Activated,
            key::ENTER => {
                if self.insert("\n", multi) {
                    Response::Changed
                } else {
                    Response::Nothing
                }
            }
            key::LEFT if !extend && !sel.is_empty() => self.move_to(sel.start, false),
            key::RIGHT if !extend && !sel.is_empty() => self.move_to(sel.end, false),
            key::LEFT => self.move_to(self.prev_char(self.caret), extend),
            key::RIGHT => self.move_to(self.next_char(self.caret), extend),
            key::UP => self.vertical(false, extend, r, theme, multi),
            key::DOWN => self.vertical(true, extend, r, theme, multi),
            key::HOME | key::END => {
                let _ = self.layout(theme, Self::text_w(r), multi);
                let k = self.line_of(self.caret);
                let (s, line) = self.line_str(k);
                let o = if code == key::HOME {
                    s
                } else {
                    let end = s + line.len();
                    let soft = self.wrap.lines.get(k + 1).is_some_and(|n| n.0 == end);
                    if soft && end > s {
                        self.prev_char(end)
                    } else {
                        end
                    }
                };
                self.move_to(o, extend)
            }
            key::BACKSPACE if sel.is_empty() => self.delete(self.prev_char(self.caret)..self.caret),
            key::DELETE if sel.is_empty() => self.delete(self.caret..self.next_char(self.caret)),
            key::BACKSPACE | key::DELETE => self.delete(sel),
            // A printable key: its text comes as TEXT_INPUT.
            _ => Response::Nothing,
        }
    }

    fn event(&mut self, ui: &mut Input<'_>, r: Rect, theme: &Theme, multi: bool) -> Response {
        let event = ui.event();
        if let Some(gained) = focus_of(&event, self.id) {
            return ui.respond(self.set_focused(gained, r));
        }
        let Some(event) = ui.unhandled() else {
            return Response::Nothing;
        };
        let response = match event {
            Event::PointerDown {
                x,
                y,
                button,
                modifiers,
            } => {
                if button != button::PRIMARY || !r.contains(x, y) {
                    self.dragging = false;
                    return Response::Nothing;
                }
                ui.take();
                self.compose.clear();
                let o = self.offset_at(x, y, r, theme, multi);
                self.dragging = true;
                self.move_to(o, modifiers.shift());
                Response::Redraw
            }
            Event::PointerMove { x, y, buttons, .. } => {
                self.hover = r.contains(x, y);
                if !self.dragging {
                    return Response::Nothing;
                }
                if buttons & button::mask(button::PRIMARY) == 0 {
                    self.dragging = false;
                    return Response::Nothing;
                }
                ui.take();
                let o = self.offset_at(x, y, r, theme, multi);
                self.move_to(o, true)
            }
            Event::PointerUp { .. } if self.dragging => {
                self.dragging = false;
                ui.take();
                Response::Nothing
            }
            Event::PointerLeave => {
                self.hover = false;
                Response::Nothing
            }
            Event::Scroll { dy, lines, .. } if multi && self.hover => {
                if self.layout(theme, Self::text_w(r), multi).is_err() {
                    return Response::Nothing;
                }
                let px = if lines { dy * theme.line_h } else { dy };
                let max = (self.content_h(theme) - Self::view_h(r, theme)).max(0.0);
                let next = (self.scroll_y + px).clamp(0.0, max);
                if next == self.scroll_y {
                    return Response::Nothing;
                }
                ui.take();
                self.scroll_y = next;
                // The reader looks elsewhere: the next frame keeps this
                // scroll instead of bringing the caret back.
                self.reveal = false;
                Response::Redraw
            }
            Event::KeyDown {
                code, modifiers, ..
            } if self.focused => self.key(ui, code, modifiers, r, theme, multi),
            Event::TextInput { text } if self.focused => {
                ui.take();
                self.compose.clear();
                if self.insert(text, multi) {
                    Response::Changed
                } else {
                    Response::Nothing
                }
            }
            Event::ImeCompose { text, caret } if self.focused => {
                ui.take();
                self.compose.clear();
                self.compose.push_str(text);
                let caret = usize::try_from(caret).unwrap_or(0).min(text.len());
                self.compose_caret = text.floor_char_boundary(caret);
                self.reveal = true;
                Response::Redraw
            }
            _ => Response::Nothing,
        };
        ui.respond(response)
    }

    // ---- drawing --------------------------------------------------------

    /// Scroll so the caret is in view after an edit or a caret move, and
    /// keep the scroll inside the content after a resize.
    fn scroll_into_view(&mut self, r: Rect, theme: &Theme, multi: bool) {
        let k = self.line_of(self.caret);
        let (s, line) = self.line_str(k);
        if multi {
            let max = (self.content_h(theme) - Self::view_h(r, theme)).max(0.0);
            if self.reveal {
                let y = k as f32 * theme.line_h;
                if y < self.scroll_y {
                    self.scroll_y = y;
                } else if y + theme.line_h > self.scroll_y + Self::view_h(r, theme) {
                    self.scroll_y = y + theme.line_h - Self::view_h(r, theme);
                }
            }
            self.scroll_y = self.scroll_y.clamp(0.0, max);
        } else {
            let vw = Self::text_w(r) - 1.0;
            let cx = caret_x(theme.text, line, self.caret - s);
            let full = caret_x(theme.text, line, line.len());
            if self.reveal {
                if cx - self.scroll_x > vw {
                    self.scroll_x = cx - vw;
                } else if cx < self.scroll_x {
                    self.scroll_x = cx;
                }
            }
            self.scroll_x = self.scroll_x.clamp(0.0, (full - vw).max(0.0));
        }
        self.reveal = false;
    }

    fn draw(
        &mut self,
        f: &mut Frame,
        r: Rect,
        theme: &Theme,
        placeholder: &str,
        multi: bool,
    ) -> Result<(), VyError> {
        let p = &theme.palette;
        let line = if self.focused { 2.0 } else { 1.0 };
        f.rect(r, RADIUS, if self.focused { p.accent } else { p.border })?;
        let inner = r.inset(line);
        f.rect(inner, (RADIUS - line).max(0.0), p.surface)?;
        // The field moved (resize, scroll): composition follows it.
        if let Some(at) = self.ime {
            if at != r && ime_begin(r).is_ok() {
                self.ime = Some(r);
            }
        }
        self.layout(theme, Self::text_w(r), multi)?;
        self.scroll_into_view(r, theme, multi);
        let this = &*self;
        f.clipped(inner, 0.0, |f| this.paint(f, r, theme, placeholder, multi))?;
        let value = clip_str(&self.text, limits::MAX_SEM_TEXT_BYTES as usize);
        f.sem.node_with_id(self.id, Role::Textbox, r, value)?;
        Ok(())
    }

    fn paint(
        &self,
        f: &mut Frame,
        r: Rect,
        theme: &Theme,
        placeholder: &str,
        multi: bool,
    ) -> Result<(), VyError> {
        let p = &theme.palette;
        let lh = theme.line_h;
        let (tx, ty) = Self::origin(r, theme, multi);
        let caret_rect = |x: f32, y: f32| Rect::new(x, y + 1.0, 1.0, (lh - 2.0).max(1.0));
        if self.text.is_empty() && self.compose.is_empty() {
            if !placeholder.is_empty() {
                f.text(tx, ty, 0.0, theme.placeholder, placeholder)?;
            }
            if self.focused {
                f.rect(caret_rect(tx, ty), 0.0, p.ink)?;
            }
            return Ok(());
        }
        let x0 = if multi { tx } else { tx - self.scroll_x };
        let n = self.wrap.lines.len();
        let (first, last) = if multi {
            let first = math::floor(self.scroll_y / lh).max(0.0) as usize;
            let last = math::ceil((self.scroll_y + Self::view_h(r, theme)) / lh) as usize;
            (first.min(n), last.min(n))
        } else {
            (0, n)
        };
        let sel = if self.focused && self.compose.is_empty() {
            self.selection()
        } else {
            0..0
        };
        let caret_line = self.line_of(self.caret);
        for k in first..last {
            let (s, line) = self.line_str(k);
            let e = s + line.len();
            let y = ty + k as f32 * lh - self.scroll_y;
            if !self.compose.is_empty() && k == caret_line {
                // The composition, underlined, inside the caret's line.
                let a = self.caret - s;
                let mut shown = String::with_capacity(line.len() + self.compose.len());
                shown.push_str(&line[..a]);
                shown.push_str(&self.compose);
                shown.push_str(&line[a..]);
                f.text(x0, y, 0.0, theme.text, &shown)?;
                let xa = caret_x(theme.text, &shown, a);
                let xb = caret_x(theme.text, &shown, a + self.compose.len());
                f.rect(
                    Rect::new(x0 + xa, y + lh - 2.0, (xb - xa).max(1.0), 1.0),
                    0.0,
                    p.ink,
                )?;
                if self.focused {
                    let xc = caret_x(theme.text, &shown, a + self.compose_caret);
                    f.rect(caret_rect(x0 + xc, y), 0.0, p.ink)?;
                }
                continue;
            }
            let (lo, hi) = (sel.start.max(s), sel.end.min(e));
            if lo < hi || (sel.start <= e && sel.end > e) {
                let xa = caret_x(theme.text, line, lo.min(e) - s);
                let xb = caret_x(theme.text, line, hi.max(lo).min(e) - s);
                // A selected line break shows as a narrow block.
                let nl = if sel.end > e && self.text[e..].starts_with('\n') {
                    NEWLINE_W
                } else {
                    0.0
                };
                if xb - xa + nl > 0.0 {
                    f.rect(Rect::new(x0 + xa, y, xb - xa + nl, lh), 0.0, p.selection)?;
                }
            }
            if !line.is_empty() {
                f.text(x0, y, 0.0, theme.text, line)?;
            }
            if self.focused && k == caret_line {
                let xc = caret_x(theme.text, line, self.caret - s);
                f.rect(caret_rect(x0 + xc, y), 0.0, p.ink)?;
            }
        }
        if multi {
            let view = Self::view_h(r, theme);
            let content = self.content_h(theme);
            if content > view {
                let thumb = (view * view / content).max(16.0);
                let max = content - view;
                let ty = r.y + Self::pad_y(theme) + (view - thumb) * self.scroll_y / max;
                f.rect(Rect::new(r.right() - 7.0, ty, 3.0, thumb), 1.5, p.track)?;
            }
        }
        Ok(())
    }
}

/// x of the caret before byte `o` of `line`, unwrapped (SPEC-5.2.5).
fn caret_x(style: StyleId, line: &str, o: usize) -> f32 {
    text_caret(style, line, o).map_or(0.0, |(x, _)| x)
}

/// FNV-1a over the bytes: the key of a paragraph's wrapped lines.
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Lines of one paragraph (no `\n`) wrapped greedily at word boundaries to
/// `width`: spaces stay at the end of the line they follow, and a word
/// wider than the line is broken after the last character that fits.
fn wrap_paragraph(style: StyleId, width: f32, p: &str) -> Result<Vec<(usize, usize)>, VyError> {
    wrap_with(width, p, &|s| text_measure(style, 0.0, s).map(|z| z.w))
}

/// [`wrap_paragraph`] over any measure of a string's width. What is
/// measured is in the order of the paragraph's length, however long its
/// words are: each word once, each token with its spaces once, and for a
/// word broken over several lines a few prefixes of about a line each
/// (the host charges every measured byte, SPEC-5.3.3).
fn wrap_with(
    width: f32,
    p: &str,
    measure: &impl Fn(&str) -> Result<f32, VyError>,
) -> Result<Vec<(usize, usize)>, VyError> {
    let mut lines = Vec::new();
    if p.is_empty() || width <= 0.0 {
        lines.push((0, p.len()));
        return Ok(lines);
    }
    let (mut start, mut line_w, mut i) = (0usize, 0.0f32, 0usize);
    while i < p.len() {
        let word_end = p[i..].find(' ').map_or(p.len(), |n| i + n);
        let rest = &p[word_end..];
        let token_end = word_end + rest.len() - rest.trim_start_matches(' ').len();
        let mut word_w = if word_end > i {
            measure(&p[i..word_end])?
        } else {
            0.0
        };
        if i > start && line_w + word_w > width {
            lines.push((start, i));
            start = i;
            line_w = 0.0;
        }
        if i == start && word_w > width {
            // Wider than the line: a line of it at a time until the rest
            // fits. The first line's length is guessed from the word's
            // width, each later one from the line before it.
            let mut hint = ((word_end - i) as f32 * width / word_w) as usize;
            loop {
                let (k, w) = fit_prefix(p, i, word_end, width, hint, measure)?;
                if k == word_end && w <= width {
                    word_w = w;
                    break;
                }
                lines.push((i, k));
                hint = k - i;
                start = k;
                i = k;
                if k == word_end {
                    break;
                }
            }
            if i == word_end {
                // Not even its last character fitted: it has its own line.
                continue;
            }
        }
        line_w += if token_end == word_end {
            word_w
        } else {
            measure(&p[i..token_end])?
        };
        i = token_end;
    }
    lines.push((start, p.len()));
    Ok(lines)
}

/// The end of the longest prefix of `p[i..end]` no wider than `width`, at
/// least one character, with that prefix's width; `i < end`.
///
/// A prefix never gets narrower by growing, so there is one such end. It
/// is looked for around `hint` bytes, where the caller expects it: a step
/// of one character first, doubling while the answer lies further away,
/// then by bisection between the last prefix that fitted and the first
/// that did not. No prefix measured is more than about twice the answer,
/// so finding it costs in the order of the line, not of the word.
fn fit_prefix(
    p: &str,
    i: usize,
    end: usize,
    width: f32,
    hint: usize,
    measure: &impl Fn(&str) -> Result<f32, VyError>,
) -> Result<(usize, f32), VyError> {
    let first = p.ceil_char_boundary(i + 1);
    let mut at = p.floor_char_boundary(i + hint).clamp(first, end);
    let mut w = measure(&p[i..at])?;
    let mut step = 1;
    // `lo` fits, `hi` does not.
    let (mut lo, mut lo_w, mut hi);
    if w <= width {
        (lo, lo_w) = (at, w);
        loop {
            if lo == end {
                return Ok((lo, lo_w));
            }
            at = p.ceil_char_boundary((lo + step).min(end));
            w = measure(&p[i..at])?;
            if w > width {
                hi = at;
                break;
            }
            (lo, lo_w) = (at, w);
            step *= 2;
        }
    } else {
        hi = at;
        loop {
            if hi == first {
                return Ok((hi, w));
            }
            at = p.floor_char_boundary(hi.saturating_sub(step)).max(first);
            w = measure(&p[i..at])?;
            if w <= width {
                (lo, lo_w) = (at, w);
                break;
            }
            hi = at;
            step *= 2;
        }
    }
    loop {
        let mut mid = p.floor_char_boundary(lo + (hi - lo) / 2);
        if mid <= lo {
            mid = p.ceil_char_boundary(lo + 1);
        }
        if mid >= hi {
            return Ok((lo, lo_w));
        }
        w = measure(&p[i..mid])?;
        if w <= width {
            (lo, lo_w) = (mid, w);
        } else {
            hi = mid;
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "textbox_tests.rs"]
mod tests;
