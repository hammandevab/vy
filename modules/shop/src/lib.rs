//! # shop
//!
//! Vy reference module 4 (`shop`): a product grid with images, a link on
//! every card, an add-to-cart button per product and a cart with a total.
//!
//! What the page exercises in the ABI:
//! - `dl_image` with package resources (SPEC-5.2.3, SPEC-6.5.1): resource
//!   ids 1..=8 are the eight PNGs the manifest lists (SPEC-4.5.1), drawn
//!   with `cover` into cards whose size follows the window, clipped to the
//!   card's rounded corners (`dl_clip_push` with a radius).
//! - `link_register` (SPEC-5.2.10) on every card, re-registered each frame
//!   under a stable id because the cards move when the grid scrolls; the
//!   host, not the page, opens `vy://shop.example/produkt/<id>`.
//! - `navigate` (SPEC-5.2.10) from the checkout button; a headless host
//!   answers `E_DENIED` and the page carries on.
//! - A grid that re-flows on `vy_resize` (SPEC-5.1.4): the cart is a side
//!   panel on wide windows and a bar under the header on narrow ones.
//! - `POINTER_DOWN` hit-testing against the laid-out buttons, `SCROLL` in a
//!   clipped grid (SPEC-6.1.4), and a semantic tree with `list`, `listitem`,
//!   `image`, `link`, `button` and `region` nodes (SPEC-10.1.1). No
//!   capabilities.
#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use vy_sdk::event::key;
use vy_sdk::prelude::*;
use vy_sdk::{link_register, navigate};

const BG: Color = Color::rgb(0xF3, 0xF1, 0xEC);
const PANEL: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const INK: Color = Color::rgb(0x22, 0x21, 0x1F);
const MUTED: Color = Color::rgb(0x6E, 0x6B, 0x66);
const RULE: Color = Color::rgb(0xDD, 0xD9, 0xD0);
const ACCENT: Color = Color::rgb(0x1F, 0x6F, 0x5A);
const ACCENT_DIM: Color = Color::rgb(0xB9, 0xCF, 0xC8);
const BADGE: Color = Color::rgb(0xC9, 0x4A, 0x2B);
const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const BAR: Color = Color::rgba(0x22, 0x21, 0x1F, 0x50);

const TITLE: &str = "Vy-butiken";
const SUBTITLE: &str = "Åtta saker för skrivbordet · fri frakt över 500 kr";
const ADD: &str = "Lägg i varukorg";
const CART: &str = "Varukorg";
const EMPTY: &str = "Varukorgen är tom.";
const CHECKOUT: &str = "Till kassan";
const CLEAR: &str = "Töm varukorgen";
const CHECKOUT_URL: &str = "vy://shop.example/kassa";

const HEADER_H: f32 = 64.0;
const MARGIN: f32 = 20.0;
const GAP: f32 = 16.0;
const PAD: f32 = 12.0;
/// Windows at least this wide get the cart as a side panel.
const WIDE_MIN: f32 = 960.0;
const CART_W: f32 = 300.0;
const CART_BAR_H: f32 = 48.0;
const MIN_CARD_W: f32 = 200.0;
const MAX_IMAGE_H: f32 = 220.0;
const BUTTON_H: f32 = 36.0;
const NAME_H: f32 = 20.0;
const PRICE_H: f32 = 18.0;
const ROW_H: f32 = 24.0;
const LINE_PX: f32 = 24.0;
/// Orders above this ship for free.
const FREE_SHIPPING: u32 = 500;
const SHIPPING: u32 = 49;

struct Product {
    name: &'static str,
    slug: &'static str,
    price: u32,
    alt: &'static str,
}

const PRODUCTS: [Product; 8] = [
    Product {
        name: "Mugg Lund",
        slug: "mugg-lund",
        price: 149,
        alt: "Brun stengodsmugg med ljust band",
    },
    Product {
        name: "Anteckningsbok A5",
        slug: "anteckningsbok-a5",
        price: 89,
        alt: "Mörkblå anteckningsbok med linjerade sidor",
    },
    Product {
        name: "Bordslampa Ask",
        slug: "bordslampa-ask",
        price: 499,
        alt: "Bordslampa med konformad skärm i ockra",
    },
    Product {
        name: "Krukväxt Fikus",
        slug: "krukvaxt-fikus",
        price: 129,
        alt: "Grön krukväxt i mörkgrön kruka",
    },
    Product {
        name: "Hörlurar Ton",
        slug: "horlurar-ton",
        price: 899,
        alt: "Lila hörlurar med bygel",
    },
    Product {
        name: "Ryggsäck Fjäll",
        slug: "ryggsack-fjall",
        price: 1295,
        alt: "Tegelröd ryggsäck med framficka",
    },
    Product {
        name: "Termos 0,5 l",
        slug: "termos-05",
        price: 249,
        alt: "Petrolblå termos med ljust lock",
    },
    Product {
        name: "Ullpläd Rand",
        slug: "ullplad-rand",
        price: 399,
        alt: "Vinröd randig ullpläd",
    },
];

/// `1 295 kr`: thousands separated by a space.
fn kr(amount: u32) -> String {
    let mut digits = format!("{amount}");
    let mut i = digits.len();
    while i > 3 {
        i -= 3;
        digits.insert(i, ' ');
    }
    digits.push_str(" kr");
    digits
}

/// Rectangles of one window size; the same arithmetic the frame draws with.
struct Layout {
    /// The scrolling grid area (clip rectangle).
    grid: Rect,
    cols: usize,
    card_w: f32,
    card_h: f32,
    image_h: f32,
    /// Side panel on wide windows.
    panel: Option<Rect>,
    /// Summary bar under the header on narrow windows.
    bar: Option<Rect>,
    /// Height of all cards including top and bottom margin.
    content_h: f32,
}

struct ShopPage {
    window: Window,
    title: StyleId,
    subtitle: StyleId,
    name: StyleId,
    price: StyleId,
    button: StyleId,
    body: StyleId,
    muted: StyleId,
    heading: StyleId,
    total: StyleId,
    badge: StyleId,
    /// Quantity per product.
    cart: [u32; PRODUCTS.len()],
    scroll: f32,
}

impl ShopPage {
    fn layout(&self) -> Layout {
        let w = self.window.width;
        let h = self.window.height;
        let wide = w >= WIDE_MIN;
        let bar_h = if wide { 0.0 } else { CART_BAR_H };
        let grid_w = if wide {
            w - 2.0 * MARGIN - CART_W - MARGIN
        } else {
            w - 2.0 * MARGIN
        }
        .max(MIN_CARD_W / 2.0);
        let grid = Rect::new(
            MARGIN,
            HEADER_H + bar_h,
            grid_w,
            (h - HEADER_H - bar_h).max(0.0),
        );
        let cols = (((grid_w + GAP) / (MIN_CARD_W + GAP)) as usize).max(1);
        let card_w = (grid_w - (cols as f32 - 1.0) * GAP) / cols as f32;
        let image_h = (card_w * 0.72).min(MAX_IMAGE_H);
        let card_h = image_h + PAD + NAME_H + 4.0 + PRICE_H + PAD + BUTTON_H + PAD;
        let rows = PRODUCTS.len().div_ceil(cols) as f32;
        let content_h = MARGIN + rows * (card_h + GAP) - GAP + MARGIN;
        Layout {
            grid,
            cols,
            card_w,
            card_h,
            image_h,
            panel: wide.then(|| {
                Rect::new(
                    w - MARGIN - CART_W,
                    HEADER_H + MARGIN,
                    CART_W,
                    (h - HEADER_H - 2.0 * MARGIN).max(0.0),
                )
            }),
            bar: (!wide).then(|| Rect::new(0.0, HEADER_H, w, CART_BAR_H)),
            content_h,
        }
    }

    fn card(&self, l: &Layout, i: usize) -> Rect {
        let col = (i % l.cols) as f32;
        let row = (i / l.cols) as f32;
        Rect::new(
            l.grid.x + col * (l.card_w + GAP),
            l.grid.y + MARGIN + row * (l.card_h + GAP) - self.scroll,
            l.card_w,
            l.card_h,
        )
    }

    fn add_button(card: Rect) -> Rect {
        Rect::new(
            card.x + PAD,
            card.bottom() - PAD - BUTTON_H,
            card.w - 2.0 * PAD,
            BUTTON_H,
        )
    }

    fn count(&self) -> u32 {
        self.cart.iter().sum()
    }

    fn total(&self) -> u32 {
        self.cart
            .iter()
            .zip(PRODUCTS.iter())
            .map(|(q, p)| q * p.price)
            .sum()
    }

    /// `1 vara`, `3 varor`.
    fn count_label(&self) -> String {
        match self.count() {
            1 => String::from("1 vara"),
            n => format!("{n} varor"),
        }
    }

    fn shipping(&self) -> u32 {
        if self.total() >= FREE_SHIPPING {
            0
        } else {
            SHIPPING
        }
    }

    /// Where the checkout and clear buttons sit in the panel or the bar.
    fn checkout_button(&self, l: &Layout) -> Rect {
        if let Some(p) = l.panel {
            let lines = self.cart.iter().filter(|q| **q > 0).count() as f32;
            let y = p.y + PAD + 24.0 + 12.0 + lines.max(1.0) * ROW_H + 12.0 + 3.0 * ROW_H + 12.0;
            Rect::new(p.x + PAD, y, p.w - 2.0 * PAD, BUTTON_H + 4.0)
        } else {
            let b = l.bar.unwrap_or_default();
            Rect::new(
                b.right() - MARGIN - 120.0,
                b.y + 7.0,
                120.0,
                CART_BAR_H - 14.0,
            )
        }
    }

    fn clear_button(&self, l: &Layout) -> Rect {
        let c = self.checkout_button(l);
        if l.panel.is_some() {
            Rect::new(c.x, c.bottom() + 8.0, c.w, 28.0)
        } else {
            Rect::new(c.x - 80.0, c.y, 72.0, c.h)
        }
    }

    fn max_scroll(&self, l: &Layout) -> f32 {
        (l.content_h - l.grid.h).max(0.0)
    }

    fn scroll_by(&mut self, dy: f32) -> bool {
        let l = self.layout();
        let next = (self.scroll + dy).clamp(0.0, self.max_scroll(&l));
        let changed = next != self.scroll;
        self.scroll = next;
        changed
    }

    fn checkout(&mut self) -> bool {
        if self.count() == 0 {
            return false;
        }
        match navigate(CHECKOUT_URL, false) {
            Ok(()) => {
                let _ = log(Level::Info, "shop: navigating to checkout");
            }
            Err(e) => {
                let _ = log(Level::Warn, &format!("shop: navigate refused: {e}"));
            }
        }
        false
    }

    fn draw_card(&self, f: &mut Frame, l: &Layout, i: usize) -> Result<(), VyError> {
        let p = &PRODUCTS[i];
        let card = self.card(l, i);
        f.rect(card, 8.0, RULE)?;
        f.rect(card.inset(1.0), 7.0, PANEL)?;
        let image = Rect::new(card.x + 1.0, card.y + 1.0, card.w - 2.0, l.image_h);
        // The picture follows the card's rounded corners (SPEC-5.2.3).
        f.clipped(card.inset(1.0), 7.0, |f| {
            f.image(image, ResourceId(i as u16 + 1), Fit::Cover)
        })?;
        let x = card.x + PAD;
        let w = card.w - 2.0 * PAD;
        let name_y = image.bottom() + PAD;
        let price_y = name_y + NAME_H + 4.0;
        let name = Rect::new(x, name_y, w, NAME_H);
        let price = Rect::new(x, price_y, w, PRICE_H);
        let button = Self::add_button(card);
        let url = format!("vy://shop.example/produkt/{}", p.slug);
        link_register(i as i32 + 1, card, &url)?;
        f.group(Role::ListItem, card, p.name, |f| {
            f.sem.image(image, p.alt)?;
            f.clipped(name, 0.0, |f| {
                f.text(x, name_y, 0.0, self.name, p.name).map(|_| ())
            })?;
            f.sem.link(name, p.name)?;
            let price_text = kr(p.price);
            f.text(x, price_y, w, self.price, &price_text)?;
            f.sem.text(price, &price_text)?;
            f.rect(button, 6.0, ACCENT)?;
            let tw = text_measure(self.button, 0.0, ADD)?.w;
            f.text(
                button.x + (button.w - tw) / 2.0,
                button.y + (BUTTON_H - 18.0) / 2.0,
                0.0,
                self.button,
                ADD,
            )?;
            f.sem
                .button(button, &format!("Lägg {} i varukorgen", p.name))?;
            let qty = self.cart[i];
            if qty > 0 {
                let label = format!("{qty} i korgen");
                let tw = text_measure(self.badge, 0.0, &label)?.w;
                let badge = Rect::new(
                    card.right() - 10.0 - tw - 16.0,
                    card.y + 10.0,
                    tw + 16.0,
                    22.0,
                );
                f.rect(badge, 11.0, BADGE)?;
                f.text(badge.x + 8.0, badge.y + 3.0, 0.0, self.badge, &label)?;
                f.sem.text(badge, &label)?;
            }
            Ok(())
        })
    }

    fn draw_panel(&self, f: &mut Frame, l: &Layout, p: Rect) -> Result<(), VyError> {
        f.rect(p, 10.0, PANEL)?;
        f.rect(Rect::new(p.x, p.y, p.w, 4.0), 2.0, ACCENT)?;
        f.group(Role::Region, p, CART, |f| {
            let x = p.x + PAD;
            let w = p.w - 2.0 * PAD;
            let mut y = p.y + PAD;
            let heading = if self.count() == 0 {
                String::from(CART)
            } else {
                format!("{CART} ({})", self.count_label())
            };
            let h = f.text(x, y, w, self.heading, &heading)?.h;
            f.sem.heading(Rect::new(x, y, w, h), &heading)?;
            y += 24.0 + 12.0;
            if self.count() == 0 {
                let h = f.text(x, y, w, self.muted, EMPTY)?.h;
                f.sem.text(Rect::new(x, y, w, h), EMPTY)?;
                y += ROW_H;
            } else {
                f.group(Role::List, Rect::new(x, y, w, 0.0), "Rader", |f| {
                    for (i, qty) in self.cart.iter().enumerate() {
                        if *qty == 0 {
                            continue;
                        }
                        let line = format!("{qty} × {}", PRODUCTS[i].name);
                        let sum = kr(qty * PRODUCTS[i].price);
                        let sw = text_measure(self.body, 0.0, &sum)?.w;
                        f.clipped(Rect::new(x, y, w - sw - 8.0, ROW_H), 0.0, |f| {
                            f.text(x, y, 0.0, self.body, &line).map(|_| ())
                        })?;
                        f.text(x + w - sw, y, 0.0, self.body, &sum)?;
                        f.sem.node(
                            Role::ListItem,
                            Rect::new(x, y, w, ROW_H),
                            &format!("{line}, {sum}"),
                        )?;
                        y += ROW_H;
                    }
                    Ok(())
                })?;
            }
            y += 12.0;
            f.rect(Rect::new(x, y - 6.0, w, 1.0), 0.0, RULE)?;
            let shipping = if self.count() == 0 {
                String::from("–")
            } else if self.shipping() == 0 {
                String::from("fri")
            } else {
                kr(self.shipping())
            };
            let rows = [
                ("Varor", kr(self.total()), self.body),
                ("Frakt", shipping, self.body),
                ("Att betala", kr(self.total() + self.shipping()), self.total),
            ];
            for (label, value, style) in rows {
                f.text(x, y, 0.0, style, label)?;
                let vw = text_measure(style, 0.0, &value)?.w;
                f.text(x + w - vw, y, 0.0, style, &value)?;
                f.sem
                    .text(Rect::new(x, y, w, ROW_H), &format!("{label}: {value}"))?;
                y += ROW_H;
            }
            let b = self.checkout_button(l);
            let enabled = self.count() > 0;
            f.rect(b, 6.0, if enabled { ACCENT } else { ACCENT_DIM })?;
            let tw = text_measure(self.button, 0.0, CHECKOUT)?.w;
            f.text(
                b.x + (b.w - tw) / 2.0,
                b.y + (b.h - 18.0) / 2.0,
                0.0,
                self.button,
                CHECKOUT,
            )?;
            f.sem.button(b, CHECKOUT)?;
            if enabled {
                let c = self.clear_button(l);
                let tw = text_measure(self.muted, 0.0, CLEAR)?.w;
                f.text(c.x + (c.w - tw) / 2.0, c.y + 5.0, 0.0, self.muted, CLEAR)?;
                f.sem.button(c, CLEAR)?;
            }
            Ok(())
        })
    }

    fn draw_bar(&self, f: &mut Frame, l: &Layout, b: Rect) -> Result<(), VyError> {
        f.rect(b, 0.0, PANEL)?;
        f.rect(Rect::new(b.x, b.bottom() - 1.0, b.w, 1.0), 0.0, RULE)?;
        f.group(Role::Region, b, CART, |f| {
            let summary = if self.count() == 0 {
                String::from(EMPTY)
            } else {
                format!(
                    "{} · {}",
                    self.count_label(),
                    kr(self.total() + self.shipping())
                )
            };
            let c = self.checkout_button(l);
            let text_w = c.x - MARGIN - 8.0 - if self.count() > 0 { 80.0 } else { 0.0 };
            f.clipped(Rect::new(MARGIN, b.y, text_w.max(0.0), b.h), 0.0, |f| {
                f.text(MARGIN, b.y + 15.0, 0.0, self.body, &summary)
                    .map(|_| ())
            })?;
            f.sem
                .text(Rect::new(MARGIN, b.y, text_w.max(0.0), b.h), &summary)?;
            let enabled = self.count() > 0;
            f.rect(c, 6.0, if enabled { ACCENT } else { ACCENT_DIM })?;
            let tw = text_measure(self.button, 0.0, CHECKOUT)?.w;
            f.text(
                c.x + (c.w - tw) / 2.0,
                c.y + (c.h - 18.0) / 2.0,
                0.0,
                self.button,
                CHECKOUT,
            )?;
            f.sem.button(c, CHECKOUT)?;
            if enabled {
                let k = self.clear_button(l);
                f.rect(k, 6.0, RULE)?;
                let tw = text_measure(self.body, 0.0, "Töm")?.w;
                f.text(
                    k.x + (k.w - tw) / 2.0,
                    k.y + (k.h - 19.0) / 2.0,
                    0.0,
                    self.body,
                    "Töm",
                )?;
                f.sem.button(k, CLEAR)?;
            }
            Ok(())
        })
    }
}

impl App for ShopPage {
    fn init(window: Window, _abi_minor: i32) -> Self {
        let style =
            |font, size, weight, color| text_style(font, size, weight, color).unwrap_or(StyleId(0));
        let _ = log(Level::Debug, "shop: init");
        Self {
            window,
            title: style(Font::Sans, 22.0, 700, INK),
            subtitle: style(Font::Sans, 13.0, 400, MUTED),
            name: style(Font::Sans, 15.0, 600, INK),
            price: style(Font::Sans, 14.0, 400, MUTED),
            button: style(Font::Sans, 14.0, 600, WHITE),
            body: style(Font::Sans, 15.0, 400, INK),
            muted: style(Font::Sans, 14.0, 400, MUTED),
            heading: style(Font::Sans, 18.0, 700, INK),
            total: style(Font::Sans, 15.0, 700, INK),
            badge: style(Font::Sans, 12.0, 600, WHITE),
            cart: [0; PRODUCTS.len()],
            scroll: 0.0,
        }
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let l = self.layout();
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        f.rect(window, 0.0, BG)?;
        f.group(Role::Main, window, "", |f| {
            // Header.
            let th = f
                .text(
                    MARGIN,
                    12.0,
                    self.window.width - 2.0 * MARGIN,
                    self.title,
                    TITLE,
                )?
                .h;
            f.sem.heading(
                Rect::new(MARGIN, 12.0, self.window.width - 2.0 * MARGIN, th),
                TITLE,
            )?;
            let sh = f.text(MARGIN, 12.0 + th, 0.0, self.subtitle, SUBTITLE)?.h;
            f.sem.text(
                Rect::new(MARGIN, 12.0 + th, self.window.width - 2.0 * MARGIN, sh),
                SUBTITLE,
            )?;

            // Grid, clipped and scrolled.
            f.clipped(l.grid, 0.0, |f| {
                f.group(Role::List, l.grid, "Produkter", |f| {
                    for i in 0..PRODUCTS.len() {
                        let card = self.card(&l, i);
                        if card.bottom() > l.grid.y && card.y < l.grid.bottom() {
                            self.draw_card(f, &l, i)?;
                        }
                    }
                    Ok(())
                })?;
                let max = self.max_scroll(&l);
                if max > 0.0 {
                    let track = l.grid.h - 16.0;
                    let thumb_h = (track * l.grid.h / l.content_h).max(24.0);
                    let thumb_y = l.grid.y + 8.0 + (track - thumb_h) * (self.scroll / max);
                    f.rect(
                        Rect::new(l.grid.right() - 6.0, thumb_y, 4.0, thumb_h),
                        2.0,
                        BAR,
                    )?;
                }
                Ok(())
            })?;

            if let Some(p) = l.panel {
                self.draw_panel(f, &l, p)?;
            }
            if let Some(b) = l.bar {
                self.draw_bar(f, &l, b)?;
            }
            Ok(())
        })
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        match event {
            Event::PointerDown { x, y, .. } => {
                let l = self.layout();
                if l.grid.contains(x, y) {
                    for i in 0..PRODUCTS.len() {
                        if Self::add_button(self.card(&l, i)).contains(x, y) {
                            self.cart[i] += 1;
                            return true;
                        }
                    }
                    return false;
                }
                if self.checkout_button(&l).contains(x, y) {
                    return self.checkout();
                }
                if self.count() > 0 && self.clear_button(&l).contains(x, y) {
                    self.cart = [0; PRODUCTS.len()];
                    return true;
                }
                false
            }
            Event::Scroll { dy, lines, .. } => {
                let px = if lines { dy * LINE_PX } else { dy };
                self.scroll_by(px)
            }
            Event::KeyDown { code, .. } => match code {
                key::DOWN => self.scroll_by(48.0),
                key::UP => self.scroll_by(-48.0),
                key::PAGE_DOWN => self.scroll_by(self.layout().grid.h),
                key::PAGE_UP => self.scroll_by(-self.layout().grid.h),
                key::HOME => self.scroll_by(-self.scroll),
                key::END => self.scroll_by(f32::MAX),
                _ => false,
            },
            _ => false,
        }
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
        // Keep the offset valid for the new grid height and column count.
        self.scroll_by(0.0);
    }
}

vy_sdk::export!(ShopPage);
