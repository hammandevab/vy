//! # chat
//!
//! Vy reference module 3 (`chat`): a message thread with bubbles on both
//! sides, timestamps from the host clock, a composer with IME, and a send
//! path over the network that degrades to a visible offline state.
//!
//! What the page exercises in the ABI:
//! - `now_ms` (SPEC-5.2.11) stamps every message; the page never has a clock
//!   of its own, so the same events give the same stamps (SPEC-2.1.5).
//! - `ime_begin`/`ime_end` (SPEC-5.2.6) follow focus on the composer, which
//!   also re-issues `ime_begin` after `vy_resize` (SPEC-5.1.4) because the
//!   field moved; committed text arrives as `TEXT_INPUT` through `vy_alloc`
//!   (SPEC-5.1.6).
//! - Enter appends the message locally and calls `fetch_begin` (SPEC-5.2.7)
//!   against the one origin the manifest lists (SPEC-5.2.8). A refusal
//!   (`E_DENIED` without `net` or in a headless host, `E_ORIGIN` for a
//!   foreign origin; SPEC-5.3.1) marks the message "ej skickat" and shows an
//!   offline banner instead of failing. `FETCH_DONE` (SPEC-A.2.1) closes the
//!   request: a 2xx reads the answer with `fetch_read` and marks the message
//!   delivered, anything else marks it failed; `KIND_CAPABILITY` revoking
//!   `net` switches the page offline at once.
//! - `SCROLL` moves a clipped list (SPEC-6.1.4); `KIND_FOCUS` on the composer
//!   is honoured (SPEC-10.3.1). Requires capabilities `ime` (SPEC-10.4.2)
//!   and `net`.
#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use vy_sdk::api::Request;
use vy_sdk::event::key;
use vy_sdk::prelude::*;
use vy_sdk::{fetch_begin, fetch_read, ime_begin, ime_end, now_ms, text_caret, Method};

const BG: Color = Color::rgb(0xEE, 0xF0, 0xF4);
const PANEL: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const INK: Color = Color::rgb(0x1C, 0x1E, 0x26);
const MUTED: Color = Color::rgb(0x6B, 0x70, 0x7C);
const RULE: Color = Color::rgb(0xD8, 0xDB, 0xE2);
const ACCENT: Color = Color::rgb(0x2F, 0x5B, 0xD6);
const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const BUBBLE_THEIRS: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const BUBBLE_FAILED: Color = Color::rgb(0x8A, 0x8F, 0x9A);
const AMBER_BG: Color = Color::rgb(0xFE, 0xF3, 0xC7);
const AMBER_INK: Color = Color::rgb(0x92, 0x40, 0x0E);
const BAR: Color = Color::rgba(0x1C, 0x1E, 0x26, 0x50);

/// The one origin the manifest lists under `net` (SPEC-5.2.8).
const API_URL: &str = "https://chat.example/api/messages";

const TITLE: &str = "Teamkanalen";
const MEMBERS: &str = "Maja, Olle och du";
const PLACEHOLDER: &str = "Skriv ett meddelande";
const SEND: &str = "Skicka";
const BANNER: &str =
    "Ingen anslutning: meddelandet sparas lokalt och skickas igen när nätet är tillbaka.";

const HEADER_H: f32 = 56.0;
const BANNER_H: f32 = 32.0;
const COMPOSER_H: f32 = 64.0;
const MARGIN: f32 = 16.0;
const BUBBLE_PAD_X: f32 = 14.0;
const BUBBLE_PAD_Y: f32 = 9.0;
const BUBBLE_RADIUS: f32 = 14.0;
/// Height of a sender name line (12 px × 1.25).
const NAME_H: f32 = 15.0;
/// Height of a timestamp line (11 px × 1.25) plus its gap to the bubble.
const STAMP_H: f32 = 18.0;
const MSG_GAP: f32 = 12.0;
const FIELD_H: f32 = 40.0;
const FIELD_PAD: f32 = 14.0;
const SEND_W: f32 = 84.0;
/// Longest message the composer accepts, in bytes.
const MAX_INPUT: usize = 500;
/// Pixels per line for `SCROLL` events that count lines.
const LINE_PX: f32 = 24.0;
/// Wall clock the page pretends `now_ms = 0` corresponds to: 09:12.
const BASE_MINUTES: i64 = 9 * 60 + 12;

/// Fixed semantic ids of the interactive nodes (SPEC-10.3.1).
const ID_COMPOSER: i32 = 1000;
const ID_SEND: i32 = 1001;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sender {
    Me,
    Maja,
    Olle,
}

impl Sender {
    fn name(self) -> &'static str {
        match self {
            Sender::Me => "Du",
            Sender::Maja => "Maja",
            Sender::Olle => "Olle",
        }
    }

    fn is_me(self) -> bool {
        self == Sender::Me
    }
}

/// Where a message of mine is on its way to the server.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Delivery {
    /// Came from someone else; nothing to deliver.
    Received,
    /// `fetch_begin` accepted it; waiting for `FETCH_DONE`.
    Sending(Request),
    /// The server answered 2xx.
    Delivered,
    /// Refused by the host or the server; kept locally.
    Failed,
}

impl Delivery {
    fn label(self) -> &'static str {
        match self {
            Delivery::Received => "",
            Delivery::Sending(_) => "skickas",
            Delivery::Delivered => "levererat",
            Delivery::Failed => "ej skickat",
        }
    }
}

struct Message {
    from: Sender,
    text: String,
    at_ms: i64,
    delivery: Delivery,
    /// Height of the wrapped text at the current bubble width.
    text_h: f32,
    /// Width of the widest line at the current bubble width.
    text_w: f32,
}

impl Message {
    /// Outer height: name line for others, bubble, stamp line.
    fn height(&self) -> f32 {
        let name = if self.from.is_me() { 0.0 } else { NAME_H };
        name + self.text_h + 2.0 * BUBBLE_PAD_Y + STAMP_H
    }
}

/// `HH:MM` of a host clock reading (SPEC-5.2.11).
fn stamp(at_ms: i64) -> String {
    let minutes = (BASE_MINUTES + at_ms.div_euclid(60_000)).rem_euclid(24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// `text` as a JSON string literal.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

struct ChatPage {
    window: Window,
    title: StyleId,
    subtitle: StyleId,
    name: StyleId,
    mine: StyleId,
    theirs: StyleId,
    stamp: StyleId,
    field: StyleId,
    placeholder: StyleId,
    button: StyleId,
    banner: StyleId,
    messages: Vec<Message>,
    input: String,
    focused: bool,
    /// Scroll offset of the thread in logical pixels, 0 at the top.
    scroll: f32,
    /// True while the view follows the newest message.
    stick: bool,
    content_h: f32,
    /// Why the last send was refused, while the page is offline.
    offline: Option<VyError>,
    /// The request in flight and the index of its message.
    pending: Option<(Request, usize)>,
}

impl ChatPage {
    fn banner_h(&self) -> f32 {
        if self.offline.is_some() {
            BANNER_H
        } else {
            0.0
        }
    }

    fn header(&self) -> Rect {
        Rect::new(0.0, 0.0, self.window.width, HEADER_H)
    }

    fn banner(&self) -> Rect {
        Rect::new(0.0, HEADER_H, self.window.width, self.banner_h())
    }

    fn thread(&self) -> Rect {
        let top = HEADER_H + self.banner_h();
        let h = (self.window.height - top - COMPOSER_H).max(0.0);
        Rect::new(0.0, top, self.window.width, h)
    }

    fn composer(&self) -> Rect {
        Rect::new(
            0.0,
            self.window.height - COMPOSER_H,
            self.window.width,
            COMPOSER_H,
        )
    }

    fn field_rect(&self) -> Rect {
        let c = self.composer();
        let w = (c.w - 2.0 * MARGIN - SEND_W - 10.0).max(40.0);
        Rect::new(MARGIN, c.y + (COMPOSER_H - FIELD_H) / 2.0, w, FIELD_H)
    }

    fn send_rect(&self) -> Rect {
        let f = self.field_rect();
        Rect::new(f.right() + 10.0, f.y, SEND_W, FIELD_H)
    }

    /// Widest a bubble's text gets.
    fn text_max_w(&self) -> f32 {
        ((self.window.width - 2.0 * MARGIN) * 0.72 - 2.0 * BUBBLE_PAD_X).clamp(60.0, 420.0)
    }

    fn style_for(&self, from: Sender) -> StyleId {
        if from.is_me() {
            self.mine
        } else {
            self.theirs
        }
    }

    /// Measure every bubble at the current width (SPEC-6.4.3).
    fn relayout(&mut self) {
        let max_w = self.text_max_w();
        let mut h = MARGIN;
        for i in 0..self.messages.len() {
            let style = self.style_for(self.messages[i].from);
            let m = &mut self.messages[i];
            let size = text_measure(style, max_w, &m.text).unwrap_or(Size { w: max_w, h: 0.0 });
            m.text_h = size.h;
            m.text_w = size.w.min(max_w);
            h += m.height() + MSG_GAP;
        }
        self.content_h = h - MSG_GAP + MARGIN;
        let target = if self.stick { f32::MAX } else { self.scroll };
        self.scroll_to(target);
    }

    fn max_scroll(&self) -> f32 {
        (self.content_h - self.thread().h).max(0.0)
    }

    fn scroll_to(&mut self, y: f32) -> bool {
        let clamped = y.clamp(0.0, self.max_scroll());
        let changed = clamped != self.scroll;
        self.scroll = clamped;
        self.stick = clamped >= self.max_scroll();
        changed
    }

    fn scroll_by(&mut self, dy: f32) -> bool {
        self.scroll_to(self.scroll + dy)
    }

    fn set_focus(&mut self, focused: bool) -> bool {
        if focused == self.focused {
            return false;
        }
        self.focused = focused;
        if focused {
            let _ = ime_begin(self.field_rect());
        } else {
            let _ = ime_end();
        }
        true
    }

    /// Append the composer's text to the thread and hand it to the network.
    fn send(&mut self) -> bool {
        let text = self.input.trim();
        if text.is_empty() {
            return false;
        }
        let text = String::from(text);
        self.input.clear();
        let body = format!("{{\"text\":{}}}", json_string(&text));
        let delivery = match fetch_begin(Method::Post, API_URL, body.as_bytes()) {
            Ok(req) => Delivery::Sending(req),
            Err(e) => {
                let _ = log(Level::Warn, &format!("chat: fetch_begin refused: {e}"));
                self.offline = Some(e);
                Delivery::Failed
            }
        };
        self.messages.push(Message {
            from: Sender::Me,
            text,
            at_ms: now_ms(),
            delivery,
            text_h: 0.0,
            text_w: 0.0,
        });
        if let Delivery::Sending(req) = delivery {
            self.pending = Some((req, self.messages.len() - 1));
        }
        self.stick = true;
        self.relayout();
        true
    }

    /// `FETCH_DONE` for the request in flight.
    fn finish(&mut self, request: i32, status: i32, bytes: i32) -> bool {
        let Some((req, idx)) = self.pending else {
            return false;
        };
        if req.0 != request {
            return false;
        }
        self.pending = None;
        if (200..300).contains(&status) {
            let mut buf = [0u8; 256];
            let read = fetch_read(req, &mut buf).unwrap_or(0);
            let _ = log(
                Level::Info,
                &format!("chat: delivered, read {read} of {bytes} bytes"),
            );
            self.messages[idx].delivery = Delivery::Delivered;
            self.offline = None;
        } else {
            let _ = log(Level::Warn, &format!("chat: send answered {status}"));
            self.messages[idx].delivery = Delivery::Failed;
            self.offline = Some(VyError::from_code(status).unwrap_or(VyError::Net));
        }
        self.relayout();
        true
    }

    fn draw_message(&self, f: &mut Frame, m: &Message, y: f32) -> Result<(), VyError> {
        let w = self.window.width;
        let bubble_w = m.text_w + 2.0 * BUBBLE_PAD_X;
        let bubble_h = m.text_h + 2.0 * BUBBLE_PAD_Y;
        let mut top = y;
        let bubble = if m.from.is_me() {
            Rect::new(w - MARGIN - bubble_w, top, bubble_w, bubble_h)
        } else {
            f.text(MARGIN, top, 0.0, self.name, m.from.name())?;
            top += NAME_H;
            Rect::new(MARGIN, top, bubble_w, bubble_h)
        };
        let fill = match (m.from.is_me(), m.delivery) {
            (false, _) => BUBBLE_THEIRS,
            (true, Delivery::Failed) => BUBBLE_FAILED,
            (true, _) => ACCENT,
        };
        f.rect(bubble, BUBBLE_RADIUS, fill)?;
        f.text(
            bubble.x + BUBBLE_PAD_X,
            bubble.y + BUBBLE_PAD_Y,
            self.text_max_w(),
            self.style_for(m.from),
            &m.text,
        )?;
        let mut line = stamp(m.at_ms);
        let label = m.delivery.label();
        if !label.is_empty() {
            line.push_str(" · ");
            line.push_str(label);
        }
        let sy = bubble.bottom() + 3.0;
        if m.from.is_me() {
            let sw = text_measure(self.stamp, 0.0, &line)?.w;
            f.text(w - MARGIN - sw, sy, 0.0, self.stamp, &line)?;
        } else {
            f.text(MARGIN, sy, 0.0, self.stamp, &line)?;
        }
        let item = Rect::new(MARGIN, y, w - 2.0 * MARGIN, m.height());
        let spoken = format!("{} {}: {}", m.from.name(), line, m.text);
        f.sem.node(Role::ListItem, item, &spoken)?;
        Ok(())
    }

    fn draw_composer(&self, f: &mut Frame) -> Result<(), VyError> {
        let c = self.composer();
        f.rect(c, 0.0, PANEL)?;
        f.rect(Rect::new(0.0, c.y, c.w, 1.0), 0.0, RULE)?;
        let r = self.field_rect();
        let border = if self.focused { ACCENT } else { RULE };
        let line = if self.focused { 2.0 } else { 1.0 };
        f.rect(r, FIELD_H / 2.0, border)?;
        f.rect(r.inset(line), FIELD_H / 2.0 - line, PANEL)?;
        let tx = r.x + FIELD_PAD;
        let ty = r.y + (FIELD_H - 19.0) / 2.0;
        if self.input.is_empty() {
            f.text(tx, ty, 0.0, self.placeholder, PLACEHOLDER)?;
        } else {
            f.clipped(r.inset(line), 0.0, |f| {
                f.text(tx, ty, 0.0, self.field, &self.input).map(|_| ())
            })?;
        }
        if self.focused {
            let (cx, _) = text_caret(self.field, &self.input, self.input.len())?;
            f.rect(Rect::new(tx + cx, ty + 1.0, 1.0, 17.0), 0.0, INK)?;
        }
        f.sem
            .node_with_id(ID_COMPOSER, Role::Textbox, r, &self.input)?;
        let b = self.send_rect();
        f.rect(b, FIELD_H / 2.0, ACCENT)?;
        let tw = text_measure(self.button, 0.0, SEND)?.w;
        f.text(
            b.x + (b.w - tw) / 2.0,
            b.y + (FIELD_H - 19.0) / 2.0,
            0.0,
            self.button,
            SEND,
        )?;
        f.sem.node_with_id(ID_SEND, Role::Button, b, SEND)?;
        Ok(())
    }
}

impl App for ChatPage {
    fn init(window: Window, _abi_minor: i32) -> Self {
        let style =
            |font, size, weight, color| text_style(font, size, weight, color).unwrap_or(StyleId(0));
        let _ = log(Level::Debug, "chat: init");
        let now = now_ms();
        let seed = |from: Sender, text: &str, minutes_ago: i64| Message {
            from,
            text: String::from(text),
            at_ms: now - minutes_ago * 60_000,
            delivery: if from.is_me() {
                Delivery::Delivered
            } else {
                Delivery::Received
            },
            text_h: 0.0,
            text_w: 0.0,
        };
        let mut page = Self {
            window,
            title: style(Font::Sans, 18.0, 700, INK),
            subtitle: style(Font::Sans, 13.0, 400, MUTED),
            name: style(Font::Sans, 12.0, 600, MUTED),
            mine: style(Font::Sans, 15.0, 400, WHITE),
            theirs: style(Font::Sans, 15.0, 400, INK),
            stamp: style(Font::Sans, 11.0, 400, MUTED),
            field: style(Font::Sans, 15.0, 400, INK),
            placeholder: style(Font::Sans, 15.0, 400, MUTED),
            button: style(Font::Sans, 15.0, 600, WHITE),
            banner: style(Font::Sans, 13.0, 400, AMBER_INK),
            messages: alloc::vec![
                seed(Sender::Maja, "Hej! Har ni hunnit läsa utkastet till specen?", 26),
                seed(
                    Sender::Me,
                    "Ja, läste det i morse. Avsnitt 5 ser bra ut, bara bilaga A som behöver fler tal.",
                    24,
                ),
                seed(
                    Sender::Maja,
                    "Toppen. Jag undrar över bränslebudgeten: räcker 8 ms på referensmaskinen?",
                    23,
                ),
                seed(
                    Sender::Olle,
                    "Vi mätte i går. 2 000 opkoder rasteriseras på under 2 ms, så det finns marginal.",
                    21,
                ),
                seed(Sender::Me, "Bra, då kör vi vidare med det.", 20),
                seed(Sender::Maja, "Kan du skicka guldbilderna när de är klara?", 3),
            ],
            input: String::new(),
            focused: false,
            scroll: 0.0,
            stick: true,
            content_h: 0.0,
            offline: None,
            pending: None,
        };
        page.relayout();
        page.set_focus(true);
        page
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        f.rect(window, 0.0, BG)?;
        f.group(Role::Main, window, "", |f| {
            // Header.
            let h = self.header();
            f.rect(h, 0.0, PANEL)?;
            f.rect(Rect::new(0.0, h.bottom() - 1.0, h.w, 1.0), 0.0, RULE)?;
            let th = f.text(MARGIN, 10.0, h.w - 2.0 * MARGIN, self.title, TITLE)?.h;
            f.sem
                .heading(Rect::new(MARGIN, 10.0, h.w - 2.0 * MARGIN, th), TITLE)?;
            let status = if self.offline.is_some() {
                format!("{MEMBERS} · offline")
            } else {
                format!("{MEMBERS} · ansluten")
            };
            let sy = 10.0 + th;
            let sh = f.text(MARGIN, sy, h.w - 2.0 * MARGIN, self.subtitle, &status)?.h;
            f.sem
                .text(Rect::new(MARGIN, sy, h.w - 2.0 * MARGIN, sh), &status)?;

            // Offline banner (SPEC-5.3.1: a refused import is not a failure).
            if let Some(e) = self.offline {
                let b = self.banner();
                f.rect(b, 0.0, AMBER_BG)?;
                f.rect(Rect::new(b.x, b.y, 4.0, b.h), 0.0, AMBER_INK)?;
                let text = format!("{BANNER} ({})", e.name());
                f.clipped(b, 0.0, |f| {
                    f.text(MARGIN, b.y + 8.0, 0.0, self.banner, &text)
                        .map(|_| ())
                })?;
                f.sem.text(b, &text)?;
            }

            // Thread.
            let t = self.thread();
            f.clipped(t, 0.0, |f| {
                f.group(Role::List, t, "Meddelanden", |f| {
                    let mut y = t.y + MARGIN - self.scroll;
                    for m in &self.messages {
                        let h = m.height();
                        if y + h > t.y && y < t.bottom() {
                            self.draw_message(f, m, y)?;
                        }
                        y += h + MSG_GAP;
                    }
                    Ok(())
                })?;
                if self.max_scroll() > 0.0 {
                    let track = t.h - 16.0;
                    let thumb_h = (track * t.h / self.content_h).max(24.0);
                    let thumb_y = t.y + 8.0 + (track - thumb_h) * (self.scroll / self.max_scroll());
                    f.rect(Rect::new(t.right() - 10.0, thumb_y, 4.0, thumb_h), 2.0, BAR)?;
                }
                Ok(())
            })?;

            self.draw_composer(f)
        })
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        match event {
            Event::PointerDown { x, y, .. } => {
                if self.field_rect().contains(x, y) {
                    self.set_focus(true)
                } else if self.send_rect().contains(x, y) {
                    self.send()
                } else {
                    self.set_focus(false)
                }
            }
            Event::TextInput { text } => {
                if self.focused && self.input.len() + text.len() <= MAX_INPUT {
                    self.input.push_str(text);
                    true
                } else {
                    false
                }
            }
            Event::KeyDown { code, .. } => match code {
                key::ENTER => self.send(),
                key::ESCAPE => self.set_focus(false),
                key::BACKSPACE if self.focused => self.input.pop().is_some(),
                key::PAGE_DOWN => self.scroll_by(self.thread().h),
                key::PAGE_UP => self.scroll_by(-self.thread().h),
                _ => false,
            },
            Event::Scroll { dy, lines, .. } => {
                let px = if lines { dy * LINE_PX } else { dy };
                self.scroll_by(px)
            }
            Event::Focus { node, gained, .. } => {
                if node.0 == ID_COMPOSER {
                    self.set_focus(gained)
                } else if gained {
                    self.set_focus(false)
                } else {
                    false
                }
            }
            Event::FetchDone {
                request,
                status,
                bytes,
            } => self.finish(request, status, bytes),
            Event::Capability { id, granted } => {
                if id != vy_abi::cap::NET {
                    return false;
                }
                if granted {
                    self.offline = None;
                } else {
                    self.offline = Some(VyError::Denied);
                }
                self.relayout();
                true
            }
            _ => false,
        }
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
        self.relayout();
        if self.focused {
            // The composer moved; tell the host where composition happens now.
            let _ = ime_begin(self.field_rect());
        }
    }
}

vy_sdk::export!(ChatPage);
