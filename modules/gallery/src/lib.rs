//! # gallery
//!
//! Vy reference module 4: a grid of eight PNG resources with captions, each
//! thumbnail fitted with one of the three `dl_image` modes, and a lightbox
//! that opens the clicked picture large.
//!
//! What it exercises in the ABI: package resources by index (`res/*.png`
//! are resource ids 1..=8, SPEC-4.5.1) decoded lazily by the host at the
//! first `dl_image` (SPEC-6.5.1); `dl_image` with `fit` 0 cover, 1 contain
//! and 2 stretch (SPEC-5.2.3), rotating through the grid so all three are
//! on screen at once, and contain again in the lightbox; `image` nodes with
//! alt text next to `text` captions (SPEC-10.1.2); a column count derived
//! from the window so `vy_resize` reflows the grid; `SCROLL`, `KEY_DOWN`
//! and `POINTER_DOWN` hit-testing against cell rectangles; a `dialog` node
//! with a `button` of fixed id while the lightbox is open. No capabilities.
#![no_std]

extern crate alloc;

use alloc::format;
use vy_sdk::event::key;
use vy_sdk::prelude::*;

const PAPER: Color = Color::rgb(0xF3, 0xF4, 0xF6);
const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const INK: Color = Color::rgb(0x1F, 0x23, 0x28);
const MUTED: Color = Color::rgb(0x64, 0x6B, 0x74);
const LINE: Color = Color::rgb(0xE3, 0xE5, 0xE8);
const THUMB_BG: Color = Color::rgb(0x2A, 0x2E, 0x35);
const BACKDROP: Color = Color::rgba(0x0B, 0x0D, 0x10, 0xE6);
const ON_DARK: Color = Color::rgb(0xF3, 0xF4, 0xF6);
const ON_DARK_MUTED: Color = Color::rgb(0x9C, 0xA3, 0xAF);
const BUTTON_BG: Color = Color::rgba(0xFF, 0xFF, 0xFF, 0x26);
const BAR: Color = Color::rgba(0x1F, 0x23, 0x28, 0x50);

const HEADER_H: f32 = 64.0;
const MARGIN: f32 = 24.0;
const GAP: f32 = 16.0;
/// Narrowest a cell gets before the grid drops a column.
const MIN_CELL: f32 = 220.0;
const CAPTION_H: f32 = 46.0;
const LINE_PX: f32 = 48.0;
const BUTTON_W: f32 = 88.0;
const BUTTON_H: f32 = 32.0;
/// Fixed semantic id of the lightbox's close button (SPEC-10.3.1).
const ID_CLOSE: i32 = 900;

/// One picture of the package: resource id is index + 1.
struct Picture {
    name: &'static str,
    alt: &'static str,
    w: u32,
    h: u32,
}

const PICTURES: [Picture; 8] = [
    Picture {
        name: "Ringar",
        alt: "Koncentriska ringar i mörkblått, vitt och ljusblått",
        w: 320,
        h: 200,
    },
    Picture {
        name: "Rutor",
        alt: "Schackrutor i gult och mörkgrått med enstaka ljusa rutor",
        w: 240,
        h: 240,
    },
    Picture {
        name: "Vågor",
        alt: "Vågräta vågband i tre turkosa nyanser",
        w: 200,
        h: 300,
    },
    Picture {
        name: "Diagonal",
        alt: "Diagonala band i rost och orange med tunna ljusa linjer",
        w: 320,
        h: 180,
    },
    Picture {
        name: "Prickar",
        alt: "Rosa prickar i tre storlekar på ljus botten",
        w: 256,
        h: 256,
    },
    Picture {
        name: "Spiral",
        alt: "En spiral i tre gröna nyanser",
        w: 300,
        h: 200,
    },
    Picture {
        name: "Rutnät",
        alt: "Ett tunt rutnät med en diagonal av grå rutor",
        w: 180,
        h: 320,
    },
    Picture {
        name: "Solnedgång",
        alt: "Stiliserad solnedgång: tre band himmel, en sol och mörkt hav",
        w: 320,
        h: 200,
    },
];

impl Picture {
    fn resource(i: usize) -> ResourceId {
        ResourceId(i as u16 + 1)
    }

    /// The fit mode of thumbnail `i`: the three modes in turn (SPEC-5.2.3).
    fn fit(i: usize) -> Fit {
        match i % 3 {
            0 => Fit::Cover,
            1 => Fit::Contain,
            _ => Fit::Stretch,
        }
    }

    fn fit_name(i: usize) -> &'static str {
        match Self::fit(i) {
            Fit::Cover => "cover",
            Fit::Contain => "contain",
            Fit::Stretch => "stretch",
        }
    }
}

/// The grid geometry for a window width.
#[derive(Clone, Copy)]
struct Grid {
    cols: usize,
    cell_w: f32,
    thumb_h: f32,
}

impl Grid {
    fn for_width(width: f32) -> Self {
        let avail = (width - 2.0 * MARGIN).max(0.0);
        let cols = (((avail + GAP) / (MIN_CELL + GAP)) as usize).max(1);
        let cell_w = (avail - (cols as f32 - 1.0) * GAP) / cols as f32;
        Self {
            cols,
            cell_w,
            thumb_h: cell_w * 2.0 / 3.0,
        }
    }

    fn cell_h(&self) -> f32 {
        self.thumb_h + CAPTION_H
    }

    fn rows(&self) -> usize {
        PICTURES.len().div_ceil(self.cols)
    }

    /// Height of everything below the header.
    fn content_h(&self) -> f32 {
        MARGIN + self.rows() as f32 * (self.cell_h() + GAP) - GAP + MARGIN
    }

    /// Cell `i` with the grid scrolled by `scroll`.
    fn cell(&self, i: usize, scroll: f32) -> Rect {
        let col = (i % self.cols) as f32;
        let row = (i / self.cols) as f32;
        Rect::new(
            MARGIN + col * (self.cell_w + GAP),
            HEADER_H + MARGIN + row * (self.cell_h() + GAP) - scroll,
            self.cell_w,
            self.cell_h(),
        )
    }
}

struct GalleryPage {
    window: Window,
    header: StyleId,
    sub: StyleId,
    caption: StyleId,
    detail: StyleId,
    light_title: StyleId,
    light_sub: StyleId,
    button: StyleId,
    scroll: f32,
    /// The picture open in the lightbox.
    open: Option<usize>,
}

impl GalleryPage {
    fn grid(&self) -> Grid {
        Grid::for_width(self.window.width)
    }

    fn max_scroll(&self) -> f32 {
        (self.grid().content_h() - (self.window.height - HEADER_H)).max(0.0)
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

    fn cell_at(&self, x: f32, y: f32) -> Option<usize> {
        if y < HEADER_H {
            return None;
        }
        let grid = self.grid();
        (0..PICTURES.len()).find(|&i| grid.cell(i, self.scroll).contains(x, y))
    }

    fn set_open(&mut self, open: Option<usize>) -> bool {
        let changed = open != self.open;
        self.open = open;
        changed
    }

    fn step(&mut self, delta: isize) -> bool {
        match self.open {
            Some(i) => {
                let n = PICTURES.len() as isize;
                let next = (i as isize + delta).rem_euclid(n) as usize;
                self.set_open(Some(next))
            }
            None => false,
        }
    }

    /// Where the lightbox's picture, caption and close button sit.
    fn lightbox_layout(&self) -> (Rect, Rect, Rect) {
        let w = self.window.width;
        let h = self.window.height;
        let picture = Rect::new(
            MARGIN,
            MARGIN + BUTTON_H + 12.0,
            (w - 2.0 * MARGIN).max(0.0),
            (h - 2.0 * MARGIN - BUTTON_H - 12.0 - 56.0).max(0.0),
        );
        let caption = Rect::new(MARGIN, picture.bottom() + 12.0, picture.w, 44.0);
        let button = Rect::new(w - MARGIN - BUTTON_W, MARGIN, BUTTON_W, BUTTON_H);
        (picture, caption, button)
    }

    fn draw_cell(&self, f: &mut Frame, i: usize) -> Result<(), VyError> {
        let grid = self.grid();
        let cell = grid.cell(i, self.scroll);
        if cell.bottom() <= HEADER_H || cell.y >= self.window.height {
            return Ok(());
        }
        let p = &PICTURES[i];
        let thumb = Rect::new(cell.x, cell.y, cell.w, grid.thumb_h);
        f.rect(cell, 8.0, WHITE)?;
        // Dark ground so contain's letterbox and cover's crop both read.
        f.clipped(thumb, 0.0, |f| {
            f.rect(thumb, 0.0, THUMB_BG)?;
            f.image(thumb, Picture::resource(i), Picture::fit(i))
        })?;
        f.sem.image(thumb, p.alt)?;
        let cy = thumb.bottom() + 8.0;
        f.text(cell.x + 12.0, cy, cell.w - 24.0, self.caption, p.name)?;
        let detail = format!("{} · {} × {} px", Picture::fit_name(i), p.w, p.h);
        f.text(
            cell.x + 12.0,
            cy + 20.0,
            cell.w - 24.0,
            self.detail,
            &detail,
        )?;
        let caption = format!("{}, {}", p.name, detail);
        f.sem.text(
            Rect::new(cell.x + 12.0, cy, cell.w - 24.0, CAPTION_H - 8.0),
            &caption,
        )?;
        Ok(())
    }

    fn draw_lightbox(&self, f: &mut Frame, i: usize) -> Result<(), VyError> {
        let p = &PICTURES[i];
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        let (picture, caption, button) = self.lightbox_layout();
        f.rect(window, 0.0, BACKDROP)?;
        f.group(Role::Dialog, window, p.name, |f| {
            f.image(picture, Picture::resource(i), Fit::Contain)?;
            f.sem.image(picture, p.alt)?;
            let title = format!("{} av {}: {}", i + 1, PICTURES.len(), p.name);
            f.text(caption.x, caption.y, caption.w, self.light_title, &title)?;
            let sub = format!(
                "{} · {} × {} px · contain · pil vänster/höger bläddrar, Escape stänger",
                p.alt, p.w, p.h
            );
            f.text(caption.x, caption.y + 24.0, caption.w, self.light_sub, &sub)?;
            f.sem.text(caption, &format!("{title}. {sub}"))?;
            f.rect(button, 16.0, BUTTON_BG)?;
            let tw = text_measure(self.button, 0.0, "Stäng")?.w;
            f.text(
                button.x + (button.w - tw) / 2.0,
                button.y + (BUTTON_H - 17.5) / 2.0,
                0.0,
                self.button,
                "Stäng",
            )?;
            f.sem
                .node_with_id(ID_CLOSE, Role::Button, button, "Stäng")?;
            Ok(())
        })
    }
}

const TITLE: &str = "Galleri";

impl App for GalleryPage {
    fn init(window: Window, _abi_minor: i32) -> Self {
        let style =
            |font, size, weight, color| text_style(font, size, weight, color).unwrap_or(StyleId(0));
        let _ = log(Level::Debug, "gallery: init");
        Self {
            window,
            header: style(Font::Sans, 22.0, 700, INK),
            sub: style(Font::Sans, 13.0, 400, MUTED),
            caption: style(Font::Sans, 14.0, 600, INK),
            detail: style(Font::Mono, 12.0, 400, MUTED),
            light_title: style(Font::Sans, 16.0, 600, ON_DARK),
            light_sub: style(Font::Sans, 13.0, 400, ON_DARK_MUTED),
            button: style(Font::Sans, 14.0, 600, ON_DARK),
            scroll: 0.0,
            open: None,
        }
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        let below = Rect::new(
            0.0,
            HEADER_H,
            self.window.width,
            self.window.height - HEADER_H,
        );
        f.rect(window, 0.0, PAPER)?;
        f.group(Role::Main, window, "", |f| {
            let header = Rect::new(0.0, 0.0, self.window.width, HEADER_H);
            f.rect(header, 0.0, WHITE)?;
            f.rect(
                Rect::new(0.0, HEADER_H - 1.0, self.window.width, 1.0),
                0.0,
                LINE,
            )?;
            let h = f.text(MARGIN, 18.0, 200.0, self.header, TITLE)?.h;
            f.sem.heading(Rect::new(MARGIN, 18.0, 200.0, h), TITLE)?;
            let sub = format!(
                "{} bilder · cover, contain och stretch i tur och ordning · klicka för att öppna",
                PICTURES.len()
            );
            let sw = text_measure(self.sub, 0.0, &sub)?.w;
            let sx = (self.window.width - MARGIN - sw).max(MARGIN + 110.0);
            f.clipped(Rect::new(sx, 0.0, self.window.width - sx, HEADER_H), 0.0, |f| {
                f.text(sx, 25.0, 0.0, self.sub, &sub)
            })?;
            f.sem.text(Rect::new(sx, 25.0, sw, 16.25), &sub)?;

            f.clipped(below, 0.0, |f| {
                f.group(Role::Region, below, "Bilder", |f| {
                    for i in 0..PICTURES.len() {
                        self.draw_cell(f, i)?;
                    }
                    Ok(())
                })
            })?;

            if self.max_scroll() > 0.0 {
                let track = below.h - 16.0;
                let thumb_h = (track * below.h / self.grid().content_h()).max(24.0);
                let thumb_y = below.y + 8.0 + (track - thumb_h) * (self.scroll / self.max_scroll());
                f.rect(
                    Rect::new(self.window.width - 10.0, thumb_y, 4.0, thumb_h),
                    2.0,
                    BAR,
                )?;
            }
            if let Some(i) = self.open {
                self.draw_lightbox(f, i)?;
            }
            Ok(())
        })
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        match event {
            Event::Scroll { dy, lines, .. } if self.open.is_none() => {
                let px = if lines { dy * LINE_PX } else { dy };
                self.scroll_by(px)
            }
            Event::KeyDown { code, .. } if self.open.is_some() => match code {
                key::ESCAPE | key::ENTER => self.set_open(None),
                key::RIGHT | key::DOWN | key::SPACE => self.step(1),
                key::LEFT | key::UP => self.step(-1),
                _ => false,
            },
            Event::KeyDown { code, .. } => {
                let page = (self.window.height - HEADER_H - LINE_PX).max(LINE_PX);
                match code {
                    key::DOWN => self.scroll_by(LINE_PX),
                    key::UP => self.scroll_by(-LINE_PX),
                    key::PAGE_DOWN | key::SPACE => self.scroll_by(page),
                    key::PAGE_UP => self.scroll_by(-page),
                    key::HOME => self.scroll_to(0.0),
                    key::END => self.scroll_to(f32::MAX),
                    _ => false,
                }
            }
            Event::PointerDown { .. } if self.open.is_some() => self.set_open(None),
            Event::PointerDown { x, y, .. } => match self.cell_at(x, y) {
                Some(i) => self.set_open(Some(i)),
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

vy_sdk::export!(GalleryPage);

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use vy_sdk::mock;
    use vy_sdk::mock::Op;

    fn page(width: f32, height: f32) -> GalleryPage {
        mock::reset();
        GalleryPage::init(
            Window {
                width,
                height,
                scale: 1.0,
            },
            0,
        )
    }

    #[test]
    fn grid_reflows_with_the_width() {
        assert_eq!(Grid::for_width(800.0).cols, 3);
        assert_eq!(Grid::for_width(1024.0).cols, 4);
        assert_eq!(Grid::for_width(400.0).cols, 1);
        assert_eq!(Grid::for_width(0.0).cols, 1);
        let g = Grid::for_width(800.0);
        assert_eq!(g.cell_w, 240.0);
        assert_eq!(g.cell(4, 0.0).x, MARGIN + 240.0 + GAP);
    }

    #[test]
    fn every_fit_mode_is_on_screen() {
        let mut page = page(800.0, 600.0);
        let mut f = Frame::begin();
        page.frame(&mut f, 16).unwrap();
        f.end().unwrap();
        mock::with(|h| {
            let fits: alloc::vec::Vec<i32> = h
                .frame_ops()
                .iter()
                .filter_map(|op| match op {
                    Op::Image { fit, .. } => Some(*fit),
                    _ => None,
                })
                .collect();
            assert_eq!(fits.len(), PICTURES.len());
            assert_eq!(&fits[..3], &[0, 1, 2]);
            let images = h
                .sem
                .iter()
                .filter(|n| n.role == Role::Image as i32)
                .count();
            assert_eq!(images, PICTURES.len(), "one image node per thumbnail");
            assert!(h.refusals.is_empty(), "{:?}", h.refusals);
        });
    }

    #[test]
    fn click_opens_lightbox_with_contain() {
        let mut page = page(800.0, 600.0);
        let cell = page.grid().cell(1, 0.0);
        assert!(page.event(Event::PointerDown {
            x: cell.x + 10.0,
            y: cell.y + 10.0,
            button: 1,
            modifiers: Modifiers(0),
        }));
        assert_eq!(page.open, Some(1));
        let mut f = Frame::begin();
        page.frame(&mut f, 16).unwrap();
        f.end().unwrap();
        mock::with(|h| {
            let last = h
                .frame_ops()
                .iter()
                .rev()
                .find_map(|op| match op {
                    Op::Image {
                        resource_id, fit, ..
                    } => Some((*resource_id, *fit)),
                    _ => None,
                })
                .unwrap();
            assert_eq!(last, (2, Fit::Contain as i32));
            assert!(h.sem.iter().any(|n| n.role == Role::Dialog as i32));
        });
        assert!(page.event(Event::KeyDown {
            code: key::RIGHT,
            modifiers: Modifiers(0),
            repeat: false,
        }));
        assert_eq!(page.open, Some(2));
        assert!(page.event(Event::KeyDown {
            code: key::ESCAPE,
            modifiers: Modifiers(0),
            repeat: false,
        }));
        assert_eq!(page.open, None);
    }
}
