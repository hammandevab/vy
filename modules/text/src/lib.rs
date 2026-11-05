//! Vy reference module 1 (`text`): a long article with headings, paragraphs
//! and inset quotations, scrolled with the wheel and the keyboard.
//!
//! Block heights are measured once per width (`vy_init`, `vy_resize`) and
//! cached, so a frame only shapes the blocks that are on screen; the page
//! is clipped to the window and registers exactly the visible blocks in the
//! semantic tree. No capabilities.
#![no_std]

mod article;

use article::{Block, ARTICLE, BLOCKS};
use vy_sdk::event::key;
use vy_sdk::prelude::*;

const PAPER: Color = Color::rgb(0xFB, 0xFA, 0xF6);
const INK: Color = Color::rgb(0x22, 0x22, 0x26);
const MUTED: Color = Color::rgb(0x5A, 0x5A, 0x60);
const RULE: Color = Color::rgb(0xC9, 0xC2, 0xB4);
const QUOTE_BG: Color = Color::rgb(0xF1, 0xEE, 0xE6);
const BAR: Color = Color::rgba(0x22, 0x22, 0x26, 0x60);

/// Widest the text column gets; narrower windows use the window minus margins.
const MAX_COLUMN: f32 = 680.0;
const MARGIN: f32 = 24.0;
const TOP: f32 = 40.0;
const BOTTOM: f32 = 80.0;
/// Pixels per line for `SCROLL` events that count lines.
const LINE_PX: f32 = 24.0;
const ARROW_PX: f32 = 48.0;
const QUOTE_PAD: f32 = 16.0;
const QUOTE_BAR: f32 = 4.0;

/// Vertical space above and below each kind of block.
fn spacing(block: &Block) -> (f32, f32) {
    match block {
        Block::Title(_) => (0.0, 20.0),
        Block::Heading(_) => (28.0, 10.0),
        Block::Para(_) => (0.0, 14.0),
        Block::Quote(_) => (6.0, 20.0),
    }
}

struct TextPage {
    window: Window,
    title: StyleId,
    heading: StyleId,
    body: StyleId,
    quote: StyleId,
    /// Scroll offset in logical pixels, 0 at the top.
    scroll: f32,
    /// Height of each block's text at the current column width.
    heights: [f32; BLOCKS],
    /// Height of the whole article including top and bottom space.
    content_h: f32,
}

impl TextPage {
    fn column(&self) -> Rect {
        let w = (self.window.width - 2.0 * MARGIN).clamp(0.0, MAX_COLUMN);
        let x = ((self.window.width - w) / 2.0).max(MARGIN);
        Rect::new(x, 0.0, w, self.window.height)
    }

    fn style_of(&self, block: &Block) -> StyleId {
        match block {
            Block::Title(_) => self.title,
            Block::Heading(_) => self.heading,
            Block::Para(_) => self.body,
            Block::Quote(_) => self.quote,
        }
    }

    /// Width the text of a block wraps at.
    fn text_width(&self, block: &Block) -> f32 {
        let w = self.column().w;
        match block {
            Block::Quote(_) => w - 2.0 * QUOTE_PAD - QUOTE_BAR,
            _ => w,
        }
    }

    /// Measure every block at the current width (SPEC-6.4.3) and the total height.
    fn relayout(&mut self) {
        let mut y = TOP;
        for (i, block) in ARTICLE.iter().enumerate() {
            let text = match block {
                Block::Title(t) | Block::Heading(t) | Block::Para(t) | Block::Quote(t) => *t,
            };
            let h = text_measure(self.style_of(block), self.text_width(block), text)
                .map(|s| s.h)
                .unwrap_or(0.0);
            self.heights[i] = h;
            let (above, below) = spacing(block);
            y += above + self.block_height(i, h) + below;
        }
        self.content_h = y + BOTTOM;
        self.scroll_to(self.scroll);
    }

    /// Outer height of block `i` given its text height.
    fn block_height(&self, i: usize, text_h: f32) -> f32 {
        match ARTICLE[i] {
            Block::Quote(_) => text_h + 2.0 * QUOTE_PAD,
            _ => text_h,
        }
    }

    fn max_scroll(&self) -> f32 {
        (self.content_h - self.window.height).max(0.0)
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
}

impl App for TextPage {
    fn init(window: Window, _abi_minor: i32) -> Self {
        let style =
            |font, size, weight, color| text_style(font, size, weight, color).unwrap_or(StyleId(0));
        // Only DejaVu Sans has a bold face in the host (SPEC-6.4.2); a bold
        // serif would be matched to it anyway, so ask for sans outright.
        let mut page = Self {
            window,
            title: style(Font::Sans, 34.0, 700, INK),
            heading: style(Font::Sans, 22.0, 700, INK),
            body: style(Font::Serif, 17.0, 400, INK),
            quote: style(Font::Serif, 17.0, 400, MUTED),
            scroll: 0.0,
            heights: [0.0; BLOCKS],
            content_h: 0.0,
        };
        page.relayout();
        let _ = log(Level::Debug, "text: init");
        page
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        let col = self.column();
        f.rect(window, 0.0, PAPER)?;
        f.clipped(window, 0.0, |f| {
            f.group(Role::Main, col, "", |f| {
                let mut y = TOP - self.scroll;
                for (i, block) in ARTICLE.iter().enumerate() {
                    let (above, below) = spacing(block);
                    let h = self.block_height(i, self.heights[i]);
                    y += above;
                    let visible = y + h > 0.0 && y < self.window.height;
                    if visible {
                        let style = self.style_of(block);
                        match block {
                            Block::Title(t) => {
                                f.text(col.x, y, col.w, style, t)?;
                                let r = Rect::new(col.x, y, col.w, h);
                                f.sem.heading(r, t)?;
                                // A rule under the title.
                                f.rect(Rect::new(col.x, y + h + 10.0, col.w, 1.0), 0.0, RULE)?;
                            }
                            Block::Heading(t) => {
                                f.text(col.x, y, col.w, style, t)?;
                                f.sem.heading(Rect::new(col.x, y, col.w, h), t)?;
                            }
                            Block::Para(t) => {
                                f.text(col.x, y, col.w, style, t)?;
                                f.sem.text(Rect::new(col.x, y, col.w, h), t)?;
                            }
                            Block::Quote(t) => {
                                let outer = Rect::new(col.x, y, col.w, h);
                                f.rect(outer, 4.0, QUOTE_BG)?;
                                f.rect(Rect::new(col.x, y, QUOTE_BAR, h), 0.0, RULE)?;
                                let tx = col.x + QUOTE_BAR + QUOTE_PAD;
                                let tw = self.text_width(block);
                                f.text(tx, y + QUOTE_PAD, tw, style, t)?;
                                f.sem
                                    .text(Rect::new(tx, y + QUOTE_PAD, tw, self.heights[i]), t)?;
                            }
                        }
                    }
                    y += h + below;
                }
                Ok(())
            })?;
            // Scroll indicator along the right edge.
            if self.max_scroll() > 0.0 {
                let track = self.window.height - 2.0 * 8.0;
                let thumb_h = (track * self.window.height / self.content_h).max(24.0);
                let thumb_y = 8.0 + (track - thumb_h) * (self.scroll / self.max_scroll());
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
                    key::DOWN => self.scroll_by(ARROW_PX),
                    key::UP => self.scroll_by(-ARROW_PX),
                    key::PAGE_DOWN | key::SPACE => self.scroll_by(page),
                    key::PAGE_UP => self.scroll_by(-page),
                    key::HOME => self.scroll_to(0.0),
                    key::END => self.scroll_to(f32::MAX),
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

vy_sdk::export!(TextPage);
