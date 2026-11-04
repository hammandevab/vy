//! Vy reference module 2 (`form`): two text fields with focus and IME, a
//! checkbox and a submit button.
//!
//! Focus is moved by pointer and by Tab/Shift+Tab, or by the host through
//! `KIND_FOCUS` (SPEC-10.3.1); a focused text field opens composition with
//! `ime_begin` over its rectangle (SPEC-5.2.6) and closes it with `ime_end`
//! when focus leaves. Committed text arrives as `TEXT_INPUT` (SPEC-5.1.6).
//! Interactive nodes keep fixed semantic ids so focus events can name them.
//! Requires capability `ime` (SPEC-10.4.2).
#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use vy_sdk::event::key;
use vy_sdk::prelude::*;
use vy_sdk::{ime_begin, ime_end, text_caret};

const PAPER: Color = Color::rgb(0xF4, 0xF5, 0xF7);
const CARD: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const INK: Color = Color::rgb(0x1C, 0x1E, 0x26);
const MUTED: Color = Color::rgb(0x6B, 0x70, 0x7C);
const BORDER: Color = Color::rgb(0xC5, 0xC9, 0xD2);
const ACCENT: Color = Color::rgb(0x2F, 0x5B, 0xD6);
const ERROR: Color = Color::rgb(0xB4, 0x2B, 0x2B);
const OK_BG: Color = Color::rgb(0xE6, 0xF4, 0xEA);
const OK_INK: Color = Color::rgb(0x1E, 0x5E, 0x33);
const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);

const MAX_CARD: f32 = 440.0;
const MARGIN: f32 = 24.0;
const PAD: f32 = 24.0;
const FIELD_H: f32 = 40.0;
const FIELD_PAD: f32 = 12.0;
const BOX: f32 = 20.0;
const BUTTON_H: f32 = 44.0;
const GAP: f32 = 8.0;
const SECTION: f32 = 20.0;
/// Longest value a field accepts, in bytes.
const MAX_VALUE: usize = 200;

/// Fixed semantic ids of the interactive nodes (SPEC-10.3.1).
const ID_NAME: i32 = 10;
const ID_EMAIL: i32 = 11;
const ID_NEWS: i32 = 12;
const ID_SUBMIT: i32 = 13;

/// What can hold keyboard focus, in Tab order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Name,
    Email,
    News,
    Submit,
}

impl Focus {
    const ORDER: [Focus; 4] = [Focus::Name, Focus::Email, Focus::News, Focus::Submit];

    fn id(self) -> i32 {
        match self {
            Focus::Name => ID_NAME,
            Focus::Email => ID_EMAIL,
            Focus::News => ID_NEWS,
            Focus::Submit => ID_SUBMIT,
        }
    }

    fn from_id(id: i32) -> Option<Focus> {
        Self::ORDER.into_iter().find(|f| f.id() == id)
    }

    fn is_field(self) -> bool {
        matches!(self, Focus::Name | Focus::Email)
    }
}

/// Rectangles of everything that can be hit, computed from the window.
struct Layout {
    card: Rect,
    name: Rect,
    email: Rect,
    news: Rect,
    submit: Rect,
}

struct FormPage {
    window: Window,
    title: StyleId,
    help: StyleId,
    label: StyleId,
    field: StyleId,
    placeholder: StyleId,
    button: StyleId,
    error: StyleId,
    ok: StyleId,
    name: String,
    email: String,
    news: bool,
    focus: Option<Focus>,
    submitted: bool,
    error_text: Option<&'static str>,
}

impl FormPage {
    fn value_mut(&mut self, f: Focus) -> Option<&mut String> {
        match f {
            Focus::Name => Some(&mut self.name),
            Focus::Email => Some(&mut self.email),
            _ => None,
        }
    }

    /// Where everything sits; the same arithmetic the frame draws with.
    fn layout(&self) -> Layout {
        let w = (self.window.width - 2.0 * MARGIN).clamp(0.0, MAX_CARD);
        let x = ((self.window.width - w) / 2.0).max(MARGIN);
        let card_x = x;
        let inner_x = card_x + PAD;
        let inner_w = (w - 2.0 * PAD).max(0.0);
        // Title and help are text; their heights come from the host in the
        // frame, so hit rectangles are laid out below a fixed header height.
        let mut y = MARGIN + PAD + self.header_h();
        let name = Rect::new(inner_x, y + 20.0, inner_w, FIELD_H);
        y = name.bottom() + SECTION;
        let email = Rect::new(inner_x, y + 20.0, inner_w, FIELD_H);
        y = email.bottom() + SECTION;
        let news = Rect::new(inner_x, y, inner_w, BOX);
        y = news.bottom() + SECTION + 4.0;
        let submit = Rect::new(inner_x, y, inner_w, BUTTON_H);
        y = submit.bottom();
        // Room for the outcome below the button: measured, so a wrapped
        // message grows the panel and the card with it.
        let w_text = inner_w;
        let extra = if let Some(e) = self.error_text {
            16.0 + text_measure(self.error, w_text, e).map_or(18.0, |s| s.h)
        } else if self.submitted {
            16.0 + self.message_h(w_text - 24.0) + 28.0
        } else {
            0.0
        };
        let card_h = y + PAD - MARGIN + extra;
        Layout {
            card: Rect::new(card_x, MARGIN, w, card_h),
            name,
            email,
            news,
            submit,
        }
    }

    fn message(&self) -> String {
        format!(
            "Tack, {}! Vi hör av oss till {}.",
            self.name.trim(),
            self.email.trim()
        )
    }

    /// Height of the success message wrapped at `w`.
    fn message_h(&self, w: f32) -> f32 {
        text_measure(self.ok, w, &self.message()).map_or(19.0, |s| s.h)
    }

    /// Height of the title and help text block; measured so the fields
    /// follow the wrapped help text.
    fn header_h(&self) -> f32 {
        let w = (self.window.width - 2.0 * MARGIN).clamp(0.0, MAX_CARD) - 2.0 * PAD;
        let title = text_measure(self.title, w, TITLE).map_or(32.0, |s| s.h);
        let help = text_measure(self.help, w, HELP).map_or(36.0, |s| s.h);
        title + GAP + help + SECTION
    }

    fn set_focus(&mut self, next: Option<Focus>) -> bool {
        if next == self.focus {
            return false;
        }
        if self.focus.is_some_and(Focus::is_field) {
            ime_end();
        }
        self.focus = next;
        if let Some(f) = next.filter(|f| f.is_field()) {
            let l = self.layout();
            ime_begin(if f == Focus::Name { l.name } else { l.email });
        }
        true
    }

    fn move_focus(&mut self, backwards: bool) -> bool {
        let n = Focus::ORDER.len();
        let i = self
            .focus
            .and_then(|f| Focus::ORDER.iter().position(|o| *o == f));
        let next = match (i, backwards) {
            (None, false) => 0,
            (None, true) => n - 1,
            (Some(i), false) => (i + 1) % n,
            (Some(i), true) => (i + n - 1) % n,
        };
        self.set_focus(Some(Focus::ORDER[next]))
    }

    fn submit(&mut self) -> bool {
        if self.name.trim().is_empty() {
            self.error_text = Some("Fyll i ditt namn.");
            self.submitted = false;
            self.set_focus(Some(Focus::Name));
        } else if !self.email.contains('@') {
            self.error_text = Some("E-postadressen behöver ett @.");
            self.submitted = false;
            self.set_focus(Some(Focus::Email));
        } else {
            self.error_text = None;
            self.submitted = true;
            self.set_focus(None);
        }
        true
    }

    fn draw_field(&self, f: &mut Frame, r: Rect, focus: Focus) -> Result<(), VyError> {
        let (label, placeholder, value) = match focus {
            Focus::Name => ("Namn", "Förnamn Efternamn", self.name.as_str()),
            _ => ("E-post", "namn@exempel.se", self.email.as_str()),
        };
        f.text(r.x, r.y - 20.0, r.w, self.label, label)?;
        let focused = self.focus == Some(focus);
        let border = if focused { ACCENT } else { BORDER };
        let line = if focused { 2.0 } else { 1.0 };
        f.rect(r, 6.0, border)?;
        f.rect(r.inset(line), (6.0 - line).max(0.0), CARD)?;
        let tx = r.x + FIELD_PAD;
        let ty = r.y + (FIELD_H - 20.0) / 2.0;
        if value.is_empty() {
            f.text(tx, ty, 0.0, self.placeholder, placeholder)?;
        } else {
            f.clipped(r.inset(line), |f| {
                f.text(tx, ty, 0.0, self.field, value).map(|_| ())
            })?;
        }
        if focused {
            let (cx, _) = text_caret(self.field, value, value.len())?;
            f.rect(Rect::new(tx + cx, ty + 1.0, 1.0, 18.0), 0.0, INK)?;
        }
        f.sem.node_with_id(focus.id(), Role::Textbox, r, value)?;
        Ok(())
    }
}

const TITLE: &str = "Anmälan";
const HELP: &str = "Fyll i namn och e-post. Tab flyttar fokus, Enter skickar. Textfälten öppnar inmatning via värden (ime).";
const NEWS: &str = "Jag vill ha nyhetsbrevet";
const SUBMIT: &str = "Skicka";

impl App for FormPage {
    fn init(window: Window, _abi_minor: i32) -> Self {
        let style =
            |font, size, weight, color| text_style(font, size, weight, color).unwrap_or(StyleId(0));
        let _ = log(Level::Debug, "form: init");
        Self {
            window,
            title: style(Font::Sans, 26.0, 700, INK),
            help: style(Font::Sans, 14.0, 400, MUTED),
            label: style(Font::Sans, 13.0, 600, MUTED),
            field: style(Font::Sans, 16.0, 400, INK),
            placeholder: style(Font::Sans, 16.0, 400, BORDER),
            button: style(Font::Sans, 16.0, 600, WHITE),
            error: style(Font::Sans, 14.0, 400, ERROR),
            ok: style(Font::Sans, 15.0, 400, OK_INK),
            name: String::new(),
            email: String::new(),
            news: false,
            focus: None,
            submitted: false,
            error_text: None,
        }
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let l = self.layout();
        f.rect(
            Rect::new(0.0, 0.0, self.window.width, self.window.height),
            0.0,
            PAPER,
        )?;
        f.rect(l.card, 10.0, CARD)?;
        f.rect(Rect::new(l.card.x, l.card.y, l.card.w, 4.0), 2.0, ACCENT)?;
        f.group(Role::Main, l.card, "", |f| {
            let x = l.card.x + PAD;
            let w = l.card.w - 2.0 * PAD;
            let mut y = l.card.y + PAD;
            let h = f.text(x, y, w, self.title, TITLE)?;
            f.sem.heading(Rect::new(x, y, w, h), TITLE)?;
            y += h + GAP;
            let h = f.text(x, y, w, self.help, HELP)?;
            f.sem.text(Rect::new(x, y, w, h), HELP)?;

            self.draw_field(f, l.name, Focus::Name)?;
            self.draw_field(f, l.email, Focus::Email)?;

            // Checkbox: box, tick as a stroked path, label.
            let b = Rect::new(l.news.x, l.news.y, BOX, BOX);
            let focused = self.focus == Some(Focus::News);
            f.rect(b, 4.0, if focused { ACCENT } else { BORDER })?;
            f.rect(b.inset(1.0), 3.0, if self.news { ACCENT } else { CARD })?;
            if self.news {
                let mut tick = Path::new();
                tick.move_to(b.x + 5.0, b.y + 10.5)
                    .line_to(b.x + 8.5, b.y + 14.0)
                    .line_to(b.x + 15.0, b.y + 6.5);
                f.path(&tick, WHITE, 2.2)?;
            }
            f.text(
                b.right() + 10.0,
                b.y + 1.0,
                l.news.w - BOX - 10.0,
                self.field,
                NEWS,
            )?;
            f.sem.node_with_id(
                ID_NEWS,
                Role::Checkbox,
                l.news,
                if self.news {
                    "Jag vill ha nyhetsbrevet (markerad)"
                } else {
                    NEWS
                },
            )?;

            // Button.
            let focused = self.focus == Some(Focus::Submit);
            if focused {
                f.rect(l.submit.inset(-3.0), 9.0, ACCENT.with_alpha(0x50))?;
            }
            f.rect(l.submit, 6.0, ACCENT)?;
            let tw = text_measure(self.button, 0.0, SUBMIT)?.w;
            f.text(
                l.submit.x + (l.submit.w - tw) / 2.0,
                l.submit.y + (BUTTON_H - 20.0) / 2.0,
                0.0,
                self.button,
                SUBMIT,
            )?;
            f.sem
                .node_with_id(ID_SUBMIT, Role::Button, l.submit, SUBMIT)?;

            // Outcome.
            let my = l.submit.bottom() + 16.0;
            if let Some(e) = self.error_text {
                let h = f.text(x, my, w, self.error, e)?;
                f.sem.text(Rect::new(x, my, w, h), e)?;
            } else if self.submitted {
                let msg = self.message();
                let h = self.message_h(w - 24.0);
                let panel = Rect::new(x, my, w, h + 28.0);
                f.rect(panel, 6.0, OK_BG)?;
                f.text(x + 12.0, my + 14.0, w - 24.0, self.ok, &msg)?;
                f.sem
                    .text(Rect::new(x + 12.0, my + 14.0, w - 24.0, h), &msg)?;
            }
            Ok(())
        })
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        match event {
            Event::PointerDown { x, y, .. } => {
                let l = self.layout();
                if l.name.contains(x, y) {
                    self.set_focus(Some(Focus::Name));
                    true
                } else if l.email.contains(x, y) {
                    self.set_focus(Some(Focus::Email));
                    true
                } else if l.news.contains(x, y) {
                    self.news = !self.news;
                    self.set_focus(Some(Focus::News));
                    true
                } else if l.submit.contains(x, y) {
                    self.set_focus(Some(Focus::Submit));
                    self.submit()
                } else {
                    self.set_focus(None)
                }
            }
            Event::TextInput { text } => match self.focus.and_then(|f| self.value_mut(f)) {
                Some(v) if v.len() + text.len() <= MAX_VALUE => {
                    v.push_str(text);
                    true
                }
                _ => false,
            },
            Event::KeyDown {
                code, modifiers, ..
            } => match code {
                key::TAB => self.move_focus(modifiers.shift()),
                key::ESCAPE => self.set_focus(None),
                key::ENTER => self.submit(),
                key::BACKSPACE => match self.focus.and_then(|f| self.value_mut(f)) {
                    Some(v) => v.pop().is_some(),
                    None => false,
                },
                key::SPACE if self.focus == Some(Focus::News) => {
                    self.news = !self.news;
                    true
                }
                key::SPACE if self.focus == Some(Focus::Submit) => self.submit(),
                _ => false,
            },
            Event::Focus { node, gained, .. } => {
                if gained {
                    self.set_focus(Focus::from_id(node.0))
                } else if self.focus.map(Focus::id) == Some(node.0) {
                    self.set_focus(None)
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
    }
}

vy_sdk::export!(FormPage);
