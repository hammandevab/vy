//! # dashboard
//!
//! Vy reference module 6: four stat tiles, a two-series line chart and a
//! bar chart drawn from a fixed data table, with a legend and a hover
//! tooltip.
//!
//! What it exercises in the ABI: `dl_path` stroked as polylines for the
//! lines (SPEC-5.2.3, SPEC-C.1.2) and `dl_rect` for bars, gridlines and
//! tiles, in paint order (SPEC-6.1.3); `POINTER_MOVE` hit-tested against
//! the bar and month slots so a tooltip follows the pointer (SPEC-A.2.1)
//! and only a change in what is hovered asks for a frame (SPEC-5.1.3);
//! `text_measure` for right-aligned axis ticks and a tooltip sized to its
//! text (SPEC-5.2.5); a layout that reflows from one row of tiles and two
//! chart cards to two by two and stacked when the window is narrow
//! (SPEC-5.1.4); the charts' data exposed as `table`/`row`/`cell` nodes
//! next to an `image` node per chart, so a reader without pixels gets the
//! numbers (SPEC-10.1.2). Colours are the validated two-slot categorical
//! palette, text in ink tokens. No capabilities.
#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use vy_sdk::prelude::*;

const SURFACE: Color = Color::rgb(0xFC, 0xFC, 0xFB);
const CARD: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const CARD_BORDER: Color = Color::rgb(0xE6, 0xE4, 0xDF);
const INK: Color = Color::rgb(0x0B, 0x0B, 0x0B);
const INK_2: Color = Color::rgb(0x52, 0x51, 0x4E);
const INK_3: Color = Color::rgb(0x8A, 0x87, 0x81);
const GRID: Color = Color::rgb(0xEC, 0xEA, 0xE5);
const AXIS: Color = Color::rgb(0xD6, 0xD3, 0xCC);
/// Categorical slots 1 and 2 (validated: CVD dE 24.7, normal 33.6).
const SERIES_1: Color = Color::rgb(0x2A, 0x78, 0xD6);
const SERIES_2: Color = Color::rgb(0xEB, 0x68, 0x34);
const SERIES_1_HOVER: Color = Color::rgb(0x1D, 0x5A, 0xA8);
const GOOD: Color = Color::rgb(0x0C, 0xA3, 0x0C);
const CRITICAL: Color = Color::rgb(0xD0, 0x3B, 0x3B);
const TOOLTIP_BG: Color = Color::rgba(0x0B, 0x0B, 0x0B, 0xE6);
const TOOLTIP_INK: Color = Color::rgb(0xFC, 0xFC, 0xFB);

const MARGIN: f32 = 24.0;
const GAP: f32 = 16.0;
const TITLE_H: f32 = 56.0;
const TILE_H: f32 = 104.0;
const CARD_PAD: f32 = 16.0;
/// Below this width tiles go two by two and the charts stack.
const WIDE: f32 = 1000.0;
const STACKED_CHART_H: f32 = 320.0;
const AXIS_W: f32 = 44.0;
const AXIS_H: f32 = 24.0;
const LEGEND_H: f32 = 28.0;
/// Room right of the line plot for the direct labels on the last points.
const LINE_LABEL_W: f32 = 52.0;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "Maj", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dec",
];
/// Visits per month.
const VISITS: [u32; 12] = [
    12_400, 13_100, 15_800, 14_900, 17_600, 19_200, 16_300, 18_800, 22_100, 24_500, 23_800, 27_200,
];
/// Returning visits per month: same unit, so one axis.
const RETURNING: [u32; 12] = [
    4_100, 4_600, 5_900, 5_700, 7_000, 7_900, 6_800, 8_100, 9_800, 11_200, 11_000, 13_100,
];
const LINE_MAX: f32 = 30_000.0;
const LINE_TICK: f32 = 10_000.0;
const DAYS: [&str; 7] = ["Mån", "Tis", "Ons", "Tor", "Fre", "Lör", "Sön"];
/// Pages published per weekday.
const PUBLISHED: [u32; 7] = [38, 44, 52, 47, 41, 18, 12];
const BAR_MAX: f32 = 60.0;
const BAR_TICK: f32 = 20.0;

struct Tile {
    label: &'static str,
    value: &'static str,
    delta: &'static str,
    up: bool,
}

const TILES: [Tile; 4] = [
    Tile {
        label: "Besök i år",
        value: "225 700",
        delta: "12,4 % mot förra året",
        up: true,
    },
    Tile {
        label: "Återkommande",
        value: "95 200",
        delta: "8,1 % mot förra året",
        up: true,
    },
    Tile {
        label: "Publicerade sidor per vecka",
        value: "252",
        delta: "3,2 % mot förra året",
        up: false,
    },
    Tile {
        label: "Genomsnittlig lästid",
        value: "3 min 42 s",
        delta: "0,6 % mot förra året",
        up: true,
    },
];

/// What the pointer is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hover {
    None,
    Month(usize),
    Day(usize),
}

/// Rectangles of the cards, from the window size.
struct Layout {
    tiles: [Rect; 4],
    line: Rect,
    bar: Rect,
}

/// `27,2 k`: thousands with one decimal and a Swedish decimal comma.
fn compact(v: u32) -> String {
    let tenths = (v + 50) / 100;
    format!("{},{} k", tenths / 10, tenths % 10)
}

/// `12 400`: digits grouped by three.
fn grouped(v: u32) -> String {
    let digits = format!("{v}");
    let len = digits.len();
    let mut s = String::with_capacity(len + len / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            s.push(' ');
        }
        s.push(c);
    }
    s
}

struct Dashboard {
    window: Window,
    title: StyleId,
    subtitle: StyleId,
    tile_label: StyleId,
    tile_value: StyleId,
    delta_up: StyleId,
    delta_down: StyleId,
    card_title: StyleId,
    tick: StyleId,
    legend: StyleId,
    tooltip_title: StyleId,
    tooltip_body: StyleId,
    hover: Hover,
}

impl Dashboard {
    fn layout(&self) -> Layout {
        let w = (self.window.width - 2.0 * MARGIN).max(0.0);
        let top = MARGIN + TITLE_H;
        let wide = self.window.width >= WIDE;
        let (tile_cols, tile_rows) = if wide { (4.0, 1.0) } else { (2.0, 2.0) };
        let tile_w = (w - (tile_cols - 1.0) * GAP) / tile_cols;
        let mut tiles = [Rect::default(); 4];
        for (i, t) in tiles.iter_mut().enumerate() {
            let col = (i as f32) % tile_cols;
            let row = ((i as f32) / tile_cols) as i32 as f32;
            *t = Rect::new(
                MARGIN + col * (tile_w + GAP),
                top + row * (TILE_H + GAP),
                tile_w,
                TILE_H,
            );
        }
        let charts_top = top + tile_rows * (TILE_H + GAP);
        let (line, bar) = if wide {
            let h = (self.window.height - charts_top - MARGIN).max(160.0);
            let line_w = (w - GAP) * 0.6;
            (
                Rect::new(MARGIN, charts_top, line_w, h),
                Rect::new(MARGIN + line_w + GAP, charts_top, w - GAP - line_w, h),
            )
        } else {
            (
                Rect::new(MARGIN, charts_top, w, STACKED_CHART_H),
                Rect::new(
                    MARGIN,
                    charts_top + STACKED_CHART_H + GAP,
                    w,
                    STACKED_CHART_H,
                ),
            )
        };
        Layout { tiles, line, bar }
    }

    /// The plot area of a chart card: below the title and legend, right of
    /// the y-axis ticks, above the x-axis labels, `right` short of the
    /// card's right padding (room for the lines' end labels).
    fn plot(card: Rect, right: f32) -> Rect {
        let top = card.y + CARD_PAD + LEGEND_H + 8.0;
        Rect::new(
            card.x + CARD_PAD + AXIS_W,
            top,
            (card.w - 2.0 * CARD_PAD - AXIS_W - right).max(0.0),
            (card.bottom() - CARD_PAD - AXIS_H - top).max(0.0),
        )
    }

    fn line_plot(card: Rect) -> Rect {
        Self::plot(card, LINE_LABEL_W)
    }

    fn bar_plot(card: Rect) -> Rect {
        Self::plot(card, 8.0)
    }

    fn month_x(plot: Rect, i: usize) -> f32 {
        plot.x + plot.w * i as f32 / (MONTHS.len() as f32 - 1.0)
    }

    fn bar_rect(plot: Rect, i: usize) -> Rect {
        let slot = plot.w / DAYS.len() as f32;
        let w = slot * 0.6;
        let h = plot.h * PUBLISHED[i] as f32 / BAR_MAX;
        Rect::new(
            plot.x + slot * i as f32 + (slot - w) / 2.0,
            plot.bottom() - h,
            w,
            h,
        )
    }

    fn hover_at(&self, x: f32, y: f32) -> Hover {
        let l = self.layout();
        let line = Self::line_plot(l.line);
        if line.contains(x, y) {
            let slot = line.w / (MONTHS.len() as f32 - 1.0);
            let i = ((x - line.x) / slot + 0.5) as usize;
            return Hover::Month(i.min(MONTHS.len() - 1));
        }
        let bar = Self::bar_plot(l.bar);
        if bar.contains(x, y) {
            let slot = bar.w / DAYS.len() as f32;
            let i = ((x - bar.x) / slot) as usize;
            return Hover::Day(i.min(DAYS.len() - 1));
        }
        Hover::None
    }

    fn card(&self, f: &mut Frame, r: Rect) -> Result<(), VyError> {
        f.rect(r, 10.0, CARD_BORDER)?;
        f.rect(r.inset(1.0), 9.0, CARD)
    }

    fn draw_tile(&self, f: &mut Frame, r: Rect, t: &Tile) -> Result<(), VyError> {
        self.card(f, r)?;
        let x = r.x + CARD_PAD;
        let w = r.w - 2.0 * CARD_PAD;
        f.text(x, r.y + 14.0, w, self.tile_label, t.label)?;
        f.text(x, r.y + 36.0, w, self.tile_value, t.value)?;
        // Status colour never alone: the arrow and the sign carry it too.
        let (style, glyph) = if t.up {
            (self.delta_up, "▲ +")
        } else {
            (self.delta_down, "▼ −")
        };
        let delta = format!("{glyph}{}", t.delta);
        f.text(x, r.y + 76.0, w, style, &delta)?;
        let name = format!("{}: {}, {}", t.label, t.value, delta);
        f.sem.text(r, &name)?;
        Ok(())
    }

    /// Gridlines, y ticks and the baseline of a plot with `max` and `tick`.
    fn draw_axes(&self, f: &mut Frame, plot: Rect, max: f32, tick: f32) -> Result<(), VyError> {
        let mut v = 0.0;
        while v <= max + 0.5 {
            let y = plot.bottom() - plot.h * v / max;
            let color = if v == 0.0 { AXIS } else { GRID };
            f.rect(Rect::new(plot.x, y - 0.5, plot.w, 1.0), 0.0, color)?;
            let label = if v > 0.0 && max >= 10_000.0 {
                format!("{} k", (v / 1000.0) as u32)
            } else {
                format!("{}", v as u32)
            };
            let tw = text_measure(self.tick, 0.0, &label)?.w;
            f.text(plot.x - 8.0 - tw, y - 8.0, 0.0, self.tick, &label)?;
            v += tick;
        }
        Ok(())
    }

    fn legend_item(
        &self,
        f: &mut Frame,
        x: f32,
        y: f32,
        color: Color,
        name: &str,
    ) -> Result<f32, VyError> {
        f.rect(Rect::new(x, y + 4.0, 10.0, 10.0), 2.0, color)?;
        f.text(x + 16.0, y, 0.0, self.legend, name)?;
        Ok(text_measure(self.legend, 0.0, name)?.w + 16.0 + 20.0)
    }

    fn tooltip(
        &self,
        f: &mut Frame,
        anchor_x: f32,
        anchor_y: f32,
        card: Rect,
        title: &str,
        lines: &[String],
    ) -> Result<(), VyError> {
        let mut w = text_measure(self.tooltip_title, 0.0, title)?.w;
        for l in lines {
            w = w.max(text_measure(self.tooltip_body, 0.0, l)?.w);
        }
        let w = w + 24.0;
        let h = 12.0 + 18.0 + 17.0 * lines.len() as f32 + 10.0;
        // Above the anchor, flipped below when there is no room, kept
        // inside the card horizontally.
        let x =
            (anchor_x - w / 2.0).clamp(card.x + 4.0, (card.right() - 4.0 - w).max(card.x + 4.0));
        let y = if anchor_y - 10.0 - h >= card.y {
            anchor_y - 10.0 - h
        } else {
            anchor_y + 14.0
        };
        let r = Rect::new(x, y, w, h);
        f.rect(r, 6.0, TOOLTIP_BG)?;
        f.text(x + 12.0, y + 10.0, 0.0, self.tooltip_title, title)?;
        let mut ly = y + 30.0;
        let mut name = String::from(title);
        for l in lines {
            f.text(x + 12.0, ly, 0.0, self.tooltip_body, l)?;
            ly += 17.0;
            name.push_str(": ");
            name.push_str(l);
        }
        f.sem.text(r, &name)?;
        Ok(())
    }

    fn draw_line_chart(&self, f: &mut Frame, card: Rect) -> Result<(), VyError> {
        self.card(f, card)?;
        let plot = Self::line_plot(card);
        let x = card.x + CARD_PAD;
        let y = card.y + CARD_PAD;
        f.text(x, y, card.w * 0.5, self.card_title, "Besök per månad")?;
        let mut lx = card.right() - CARD_PAD - 230.0;
        lx += self.legend_item(f, lx, y + 2.0, SERIES_1, "Besök")?;
        self.legend_item(f, lx, y + 2.0, SERIES_2, "Återkommande")?;
        self.draw_axes(f, plot, LINE_MAX, LINE_TICK)?;
        for (i, m) in MONTHS.iter().enumerate() {
            let tw = text_measure(self.tick, 0.0, m)?.w;
            f.text(
                Self::month_x(plot, i) - tw / 2.0,
                plot.bottom() + 6.0,
                0.0,
                self.tick,
                m,
            )?;
        }
        let hovered = match self.hover {
            Hover::Month(i) => Some(i),
            _ => None,
        };
        if let Some(i) = hovered {
            let cx = Self::month_x(plot, i);
            f.rect(Rect::new(cx - 0.5, plot.y, 1.0, plot.h), 0.0, AXIS)?;
        }
        for (series, color) in [(&VISITS, SERIES_1), (&RETURNING, SERIES_2)] {
            let mut path = Path::new();
            for (i, v) in series.iter().enumerate() {
                let px = Self::month_x(plot, i);
                let py = plot.bottom() - plot.h * (*v as f32) / LINE_MAX;
                if i == 0 {
                    path.move_to(px, py);
                } else {
                    path.line_to(px, py);
                }
            }
            f.path(&path, color, 2.0)?;
            // Direct label on the last point only.
            let last = series[series.len() - 1];
            let py = plot.bottom() - plot.h * (last as f32) / LINE_MAX;
            f.rect(
                Rect::new(plot.right() - 4.0, py - 4.0, 8.0, 8.0),
                4.0,
                color,
            )?;
            f.text(plot.right() + 8.0, py - 8.0, 0.0, self.tick, &compact(last))?;
            if let Some(i) = hovered {
                let hx = Self::month_x(plot, i);
                let hy = plot.bottom() - plot.h * (series[i] as f32) / LINE_MAX;
                f.rect(Rect::new(hx - 5.0, hy - 5.0, 10.0, 10.0), 5.0, CARD)?;
                f.rect(Rect::new(hx - 4.0, hy - 4.0, 8.0, 8.0), 4.0, color)?;
            }
        }
        f.sem.image(
            plot,
            "Linjediagram: besök och återkommande besök per månad, januari till december, 0 till 30 000",
        )?;
        f.group(Role::Table, plot, "Besök per månad", |f| {
            let slot = plot.w / (MONTHS.len() as f32 - 1.0);
            for (i, m) in MONTHS.iter().enumerate() {
                let r = Rect::new(Self::month_x(plot, i) - slot / 2.0, plot.y, slot, plot.h);
                f.group(Role::Row, r, m, |f| {
                    f.sem.node(Role::Cell, r, &grouped(VISITS[i]))?;
                    f.sem.node(Role::Cell, r, &grouped(RETURNING[i]))?;
                    Ok(())
                })?;
            }
            Ok(())
        })?;
        if let Some(i) = hovered {
            let cx = Self::month_x(plot, i);
            let cy = plot.bottom() - plot.h * (VISITS[i] as f32) / LINE_MAX;
            let lines = [
                format!("Besök {}", grouped(VISITS[i])),
                format!("Återkommande {}", grouped(RETURNING[i])),
            ];
            self.tooltip(f, cx, cy, card, MONTHS[i], &lines)?;
        }
        Ok(())
    }

    fn draw_bar_chart(&self, f: &mut Frame, card: Rect) -> Result<(), VyError> {
        self.card(f, card)?;
        let plot = Self::bar_plot(card);
        let x = card.x + CARD_PAD;
        let y = card.y + CARD_PAD;
        f.text(
            x,
            y,
            card.w - 2.0 * CARD_PAD,
            self.card_title,
            "Publicerade sidor per veckodag",
        )?;
        self.draw_axes(f, plot, BAR_MAX, BAR_TICK)?;
        let hovered = match self.hover {
            Hover::Day(i) => Some(i),
            _ => None,
        };
        for (i, d) in DAYS.iter().enumerate() {
            let r = Self::bar_rect(plot, i);
            let color = if hovered == Some(i) {
                SERIES_1_HOVER
            } else {
                SERIES_1
            };
            // Rounded top, square bottom on the baseline.
            f.rect(r, 4.0, color)?;
            f.rect(Rect::new(r.x, r.bottom() - 4.0, r.w, 4.0), 0.0, color)?;
            let tw = text_measure(self.tick, 0.0, d)?.w;
            f.text(
                r.x + (r.w - tw) / 2.0,
                plot.bottom() + 6.0,
                0.0,
                self.tick,
                d,
            )?;
        }
        f.sem.image(
            plot,
            "Stapeldiagram: publicerade sidor per veckodag, måndag till söndag, 0 till 60",
        )?;
        f.group(Role::Table, plot, "Publicerade sidor per veckodag", |f| {
            for (i, d) in DAYS.iter().enumerate() {
                let r = Self::bar_rect(plot, i);
                f.group(Role::Row, r, d, |f| {
                    f.sem.node(Role::Cell, r, &format!("{}", PUBLISHED[i]))?;
                    Ok(())
                })?;
            }
            Ok(())
        })?;
        if let Some(i) = hovered {
            let r = Self::bar_rect(plot, i);
            let lines = [format!("{} sidor", PUBLISHED[i])];
            self.tooltip(f, r.x + r.w / 2.0, r.y, card, DAYS[i], &lines)?;
        }
        Ok(())
    }
}

const TITLE: &str = "Översikt";
const SUBTITLE: &str =
    "Januari till december 2026 · fast datatabell · för pekaren över en stapel eller en månad";

impl App for Dashboard {
    fn init(window: Window, _abi_minor: i32) -> Self {
        let style =
            |font, size, weight, color| text_style(font, size, weight, color).unwrap_or(StyleId(0));
        let _ = log(Level::Debug, "dashboard: init");
        Self {
            window,
            title: style(Font::Sans, 24.0, 700, INK),
            subtitle: style(Font::Sans, 13.0, 400, INK_2),
            tile_label: style(Font::Sans, 13.0, 400, INK_2),
            tile_value: style(Font::Sans, 28.0, 700, INK),
            delta_up: style(Font::Sans, 13.0, 600, GOOD),
            delta_down: style(Font::Sans, 13.0, 600, CRITICAL),
            card_title: style(Font::Sans, 15.0, 600, INK),
            tick: style(Font::Sans, 12.0, 400, INK_3),
            legend: style(Font::Sans, 13.0, 400, INK_2),
            tooltip_title: style(Font::Sans, 13.0, 600, TOOLTIP_INK),
            tooltip_body: style(Font::Sans, 13.0, 400, TOOLTIP_INK),
            hover: Hover::None,
        }
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        let l = self.layout();
        f.rect(window, 0.0, SURFACE)?;
        f.group(Role::Main, window, "", |f| {
            let w = self.window.width - 2.0 * MARGIN;
            let h = f.text(MARGIN, MARGIN, w, self.title, TITLE)?;
            f.sem.heading(Rect::new(MARGIN, MARGIN, w, h), TITLE)?;
            let sh = f.text(MARGIN, MARGIN + 32.0, w, self.subtitle, SUBTITLE)?;
            f.sem
                .text(Rect::new(MARGIN, MARGIN + 32.0, w, sh), SUBTITLE)?;
            f.group(
                Role::Region,
                Rect::new(MARGIN, l.tiles[0].y, w, l.tiles[3].bottom() - l.tiles[0].y),
                "Nyckeltal",
                |f| {
                    for (r, t) in l.tiles.iter().zip(TILES.iter()) {
                        self.draw_tile(f, *r, t)?;
                    }
                    Ok(())
                },
            )?;
            f.group(Role::Region, l.line, "Besök per månad", |f| {
                self.draw_line_chart(f, l.line)
            })?;
            f.group(Role::Region, l.bar, "Publicerade sidor per veckodag", |f| {
                self.draw_bar_chart(f, l.bar)
            })?;
            Ok(())
        })
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        match event {
            Event::PointerMove { x, y, .. } => {
                let hover = self.hover_at(x, y);
                let changed = hover != self.hover;
                self.hover = hover;
                changed
            }
            _ => false,
        }
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
    }
}

vy_sdk::export!(Dashboard);

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    extern crate std;

    use super::*;
    use vy_sdk::mock;
    use vy_sdk::mock::Op;

    fn page(width: f32, height: f32) -> Dashboard {
        mock::reset();
        Dashboard::init(
            Window {
                width,
                height,
                scale: 1.0,
            },
            0,
        )
    }

    #[test]
    fn layout_reflows_below_1000_px() {
        let wide = page(1280.0, 800.0).layout();
        assert_eq!(wide.tiles[3].y, wide.tiles[0].y);
        assert_eq!(wide.line.y, wide.bar.y);
        let narrow = page(800.0, 1000.0).layout();
        assert_eq!(narrow.tiles[2].y, narrow.tiles[0].y + TILE_H + GAP);
        assert_eq!(narrow.bar.y, narrow.line.bottom() + GAP);
        assert!(narrow.bar.bottom() + MARGIN <= 1000.0);
    }

    #[test]
    fn hover_over_a_bar_shows_a_tooltip_once() {
        let mut page = page(1280.0, 800.0);
        let bar = Dashboard::bar_rect(Dashboard::bar_plot(page.layout().bar), 2);
        let mv = |page: &mut Dashboard, x: f32, y: f32| {
            page.event(Event::PointerMove {
                x,
                y,
                buttons: 0,
                modifiers: Modifiers(0),
            })
        };
        assert!(mv(&mut page, bar.x + bar.w / 2.0, bar.y + 10.0));
        assert_eq!(page.hover, Hover::Day(2));
        assert!(
            !mv(&mut page, bar.x + 1.0, bar.y + 20.0),
            "same bar: no redraw"
        );
        let mut f = Frame::begin();
        page.frame(&mut f, 16).unwrap();
        f.end().unwrap();
        mock::with(|h| {
            assert!(h.sem.iter().any(|n| n.text == "Ons: 52 sidor"));
            assert!(h.refusals.is_empty(), "{:?}", h.refusals);
            let paths = h
                .frame_ops()
                .iter()
                .filter(|op| matches!(op, Op::Path { .. }))
                .count();
            assert_eq!(paths, 2, "one polyline per series");
            let tables = h
                .sem
                .iter()
                .filter(|n| n.role == Role::Table as i32)
                .count();
            let cells = h.sem.iter().filter(|n| n.role == Role::Cell as i32).count();
            assert_eq!((tables, cells), (2, 24 + 7));
        });
        assert!(mv(&mut page, 1.0, 1.0));
        assert_eq!(page.hover, Hover::None);
    }

    #[test]
    fn number_formats() {
        assert_eq!(compact(27_200), "27,2 k");
        assert_eq!(compact(13_050), "13,1 k");
        assert_eq!(grouped(225_700), "225 700");
        assert_eq!(grouped(252), "252");
    }

    #[test]
    fn print_hover_coordinates() {
        let page = page(1280.0, 800.0);
        let bar = Dashboard::bar_rect(Dashboard::bar_plot(page.layout().bar), 2);
        let plot = Dashboard::line_plot(page.layout().line);
        std::println!(
            "bar Ons centre ({}, {}); month Sep x {} plot y {}..{}",
            bar.x + bar.w / 2.0,
            bar.y + bar.h / 2.0,
            Dashboard::month_x(plot, 8),
            plot.y,
            plot.bottom()
        );
    }
}
