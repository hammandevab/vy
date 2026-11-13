//! Vy reference module 2 (`form`): two text fields with focus and IME, a
//! checkbox and a submit button.
//!
//! Focus is the host's (SPEC-10.3.1): Tab, Shift+Tab and pointer presses
//! arrive as `KIND_FOCUS` on the fixed node ids below, the page keeps no
//! focus of its own (a failed submit shows the error and leaves focus where
//! the host has it), and every control draws its focus mark itself
//! (SPEC-10.3.3). The text fields and the button are `vy_sdk::ui`
//! components: a focused `ui::TextBox` opens composition with `ime_begin`
//! over its rectangle (SPEC-5.2.6) and closes it with `ime_end` when focus
//! leaves, and takes committed text as `TEXT_INPUT` (SPEC-5.1.6). The
//! checkbox is drawn by the page, as before the components existed: the
//! component's ring would change this page's pixels. Interactive nodes
//! keep fixed semantic ids so focus events can name them. Requires
//! capability `ime` (SPEC-10.4.2).
#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use vy_sdk::event::key;
use vy_sdk::prelude::*;
use vy_sdk::ui::{Button, Input, Label, Palette, TextBox, Theme};

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
const BOX: f32 = 20.0;
const BUTTON_H: f32 = 44.0;
const GAP: f32 = 8.0;
const SECTION: f32 = 20.0;
/// Longest value a field accepts, in bytes.
const MAX_VALUE: usize = 200;

/// Fixed semantic ids of the interactive nodes (SPEC-10.3.1); a page's own
/// ids are negative (SPEC-5.2.13).
const ID_NAME: i32 = -10;
const ID_EMAIL: i32 = -11;
const ID_NEWS: i32 = -12;
const ID_SUBMIT: i32 = -13;

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
    /// The fields' and the button's look: this page's colours, the
    /// placeholder in the border colour and a translucent accent ring.
    theme: Theme,
    title: StyleId,
    help: StyleId,
    label: StyleId,
    error: StyleId,
    ok: StyleId,
    name: TextBox,
    email: TextBox,
    send: Button,
    news: bool,
    news_focused: bool,
    submitted: bool,
    error_text: Option<&'static str>,
}

impl FormPage {
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
            self.name.text().trim(),
            self.email.text().trim()
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

    /// Validate and submit. Focus stays where the host has it (SPEC-10.3.1):
    /// the error text says which field needs attention.
    fn submit(&mut self) -> bool {
        if self.name.text().trim().is_empty() {
            self.error_text = Some("Fyll i ditt namn.");
            self.submitted = false;
        } else if !self.email.text().contains('@') {
            self.error_text = Some("E-postadressen behöver ett @.");
            self.submitted = false;
        } else {
            self.error_text = None;
            self.submitted = true;
        }
        true
    }

    /// A label above the field, then the field itself (`ui::TextBox`),
    /// which registers the label as its name in the tree.
    fn draw_field(&mut self, f: &mut Frame, r: Rect, email: bool) -> Result<(), VyError> {
        let (label, placeholder) = if email {
            ("E-post", "namn@exempel.se")
        } else {
            ("Namn", "Förnamn Efternamn")
        };
        let above = Rect::new(r.x, r.y - 20.0, r.w, 20.0);
        f.text(above.x, above.y, above.w, self.label, label)?;
        let theme = self.theme;
        let field = if email {
            &mut self.email
        } else {
            &mut self.name
        };
        field.draw(f, r, &theme, Label::shown(above, label), placeholder)
    }
}

const TITLE: &str = "Anmälan";
const HELP: &str = "Fyll i namn och e-post. Tab flyttar fokus, Enter skickar. Textfälten öppnar inmatning via värden (ime).";
const NEWS: &str = "Jag vill ha nyhetsbrevet";
const SUBMIT: &str = "Skicka";

impl App for FormPage {
    fn init(window: Window, _abi_minor: i32) -> Result<Self, VyError> {
        let style = |font, size, weight, color| text_style(font, size, weight, color);
        let _ = log(Level::Debug, "form: init");
        let palette = Palette {
            surface: CARD,
            ink: INK,
            muted: MUTED,
            placeholder: BORDER,
            border: BORDER,
            accent: ACCENT,
            on_accent: WHITE,
            ring: ACCENT.with_alpha(0x50),
            ..Palette::LIGHT
        };
        Ok(Self {
            window,
            theme: Theme::custom(palette, Font::Sans, 16.0)?,
            title: style(Font::Sans, 26.0, 700, INK)?,
            help: style(Font::Sans, 14.0, 400, MUTED)?,
            label: style(Font::Sans, 13.0, 600, MUTED)?,
            error: style(Font::Sans, 14.0, 400, ERROR)?,
            ok: style(Font::Sans, 15.0, 400, OK_INK)?,
            name: TextBox::new(ID_NAME).with_max_len(MAX_VALUE),
            email: TextBox::new(ID_EMAIL).with_max_len(MAX_VALUE),
            send: Button::new(ID_SUBMIT),
            news: false,
            news_focused: false,
            submitted: false,
            error_text: None,
        })
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
            let h = f.text(x, y, w, self.title, TITLE)?.h;
            f.sem.node(Role::Heading, Rect::new(x, y, w, h), TITLE)?;
            y += h + GAP;
            let h = f.text(x, y, w, self.help, HELP)?.h;
            f.sem.node(Role::Text, Rect::new(x, y, w, h), HELP)?;

            self.draw_field(f, l.name, false)?;
            self.draw_field(f, l.email, true)?;

            // Checkbox: box, tick as a stroked path, label.
            let b = Rect::new(l.news.x, l.news.y, BOX, BOX);
            f.rect(b, 4.0, if self.news_focused { ACCENT } else { BORDER })?;
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
                self.theme.text,
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

            // Button (`ui::Button`: ring, fill, centred label, node).
            let theme = self.theme;
            self.send.draw(f, l.submit, &theme, SUBMIT)?;

            // Outcome.
            let my = l.submit.bottom() + 16.0;
            if let Some(e) = self.error_text {
                let h = f.text(x, my, w, self.error, e)?.h;
                f.sem.node(Role::Text, Rect::new(x, my, w, h), e)?;
            } else if self.submitted {
                let msg = self.message();
                let h = self.message_h(w - 24.0);
                let panel = Rect::new(x, my, w, h + 28.0);
                f.rect(panel, 6.0, OK_BG)?;
                f.text(x + 12.0, my + 14.0, w - 24.0, self.ok, &msg)?;
                f.sem.node(
                    Role::Text,
                    Rect::new(x + 12.0, my + 14.0, w - 24.0, h),
                    &msg,
                )?;
            }
            Ok(())
        })
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        let l = self.layout();
        let theme = self.theme;
        let mut ui = Input::new(event);
        // The host has already moved focus to the control under the
        // pointer (SPEC-10.3.1); the components take what is theirs.
        let name = self.name.event(&mut ui, l.name, &theme);
        let email = self.email.event(&mut ui, l.email, &theme);
        let send = self.send.event(&mut ui, l.submit, &theme);
        if name.activated() || email.activated() || send.activated() {
            self.submit();
        }
        // The checkbox, and Enter anywhere else, are the page's.
        let own = match ui.unhandled() {
            Some(Event::PointerDown { x, y, .. }) if l.news.contains(x, y) => {
                self.news = !self.news;
                true
            }
            Some(Event::KeyDown { code, .. }) => match code {
                key::ENTER => self.submit(),
                key::SPACE if self.news_focused => {
                    self.news = !self.news;
                    true
                }
                _ => false,
            },
            Some(Event::Focus { node, gained, .. }) if node.0 == ID_NEWS => {
                self.news_focused = gained;
                true
            }
            _ => false,
        };
        ui.redraw() || own
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
    }
}

vy_sdk::export!(FormPage);
