//! Vy reference module 0 (`hello`).
//!
//! Draws a background, a card, a heading and a paragraph, and registers the
//! two text runs in the semantic tree. No capabilities are requested, so this
//! module runs in restricted mode without any difference.
//!
//! Built on `vy-sdk`: the six exports (SPEC-4.2.1) come from `export!`, the
//! imports (SPEC-4.2.2) from the SDK's bindings; no WASI, no threads
//! (SPEC-4.2.3).
#![no_std]

use vy_sdk::prelude::*;

const BG: Color = Color::rgb(0x1A, 0x1A, 0x2E);
const CARD: Color = Color::rgb(0xF5, 0xF5, 0xF0);
const INK: Color = Color::rgb(0x1A, 0x1A, 0x2E);
const MUTED: Color = Color::rgb(0x5F, 0x5E, 0x5A);

const TITLE: &str = "Hej från Vy";
const BODY: &str = "Den här sidan är en WebAssembly-modul. Den ritar sig själv via sex importer och äger inte en enda pixel.";

struct Hello {
    window: Window,
    title: StyleId,
    body: StyleId,
}

impl App for Hello {
    fn init(window: Window, _abi_minor: i32) -> Self {
        let title = text_style(Font::Sans, 28.0, 600, INK).unwrap_or(StyleId(0));
        let body = text_style(Font::Sans, 16.0, 400, MUTED).unwrap_or(StyleId(0));
        let _ = log(Level::Debug, "hello: init");
        let _ = vy_sdk::abi_version();
        Self {
            window,
            title,
            body,
        }
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let margin = 24.0;
        let pad = 20.0;
        let card_w = (self.window.width - 2.0 * margin).max(0.0);
        let text_w = card_w - 2.0 * pad;

        // Measure first, then emit background, card and text in paint order
        // (SPEC-6.1.3).
        let title_h = text_measure(self.title, text_w, TITLE)?.h;
        let body_h = text_measure(self.body, text_w, BODY)?.h;
        let title_y = margin + pad;
        let body_y = title_y + title_h + 12.0;
        let card_h = pad + title_h + 12.0 + body_h + pad;
        let card = Rect::new(margin, margin, card_w, card_h);

        f.rect(Rect::new(0.0, 0.0, self.window.width, self.window.height), 0.0, BG)?;
        f.rect(card, 12.0, CARD)?;
        f.group(Role::Main, card, "", |f| {
            let x = margin + pad;
            f.text(x, title_y, text_w, self.title, TITLE)?;
            f.sem.heading(Rect::new(x, title_y, text_w, title_h), TITLE)?;
            f.text(x, body_y, text_w, self.body, BODY)?;
            f.sem.text(Rect::new(x, body_y, text_w, body_h), BODY)?;
            Ok(())
        })
    }

    fn event(&mut self, _event: Event<'_>) -> bool {
        false
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
    }
}

vy_sdk::export!(Hello);
