//! The frame SPEC-13.2.1 is measured on: 2 000 opcodes in the proportions
//! the reference pages draw, with the sizes they draw them at.
//!
//! The proportions are a tally of the display lists of the 48 headless
//! conformance cases (2 491 ops in all): 41 % text runs of about 30
//! bytes, 19 % opaque rectangles, 19 % opaque rounded rectangles, 6 %
//! plain clips and 2 % rounded clips with their pops, 2 % images, 2 %
//! translucent rounded rectangles, 0.4 % stroked paths. `vy-cli bench`
//! and `spec_13_2_1_raster_budget` rasterize the same bytes, so the
//! number in the notes and the number the test asserts are one number.
//!
//! Beside it, the pathological lists M3's bench measured ([`grid`]) and
//! the rounded cards of the vy-render tests ([`rounded_cards`]), which
//! `vy-cli bench` reports without a budget of their own.

use crate::image::{ImageCache, RgbaImage};
use crate::text::TextEngine;
use vy_abi::{font, weight};
use vy_dl::{DisplayList, Fit, PathCmd};

/// The frame: its display list and the image cache its `IMAGE` ops name.
pub struct Frame {
    /// Display list bytes, exactly 2 000 ops.
    pub list: Vec<u8>,
    /// Resource 1: a 64 × 48 picture, decoded.
    pub images: ImageCache,
}

/// Ops in the frame.
pub const OPS: usize = 2_000;

/// A 64 × 48 picture with a gradient and a diagonal, as a PNG.
fn picture() -> Vec<u8> {
    let (w, h) = (64u32, 48u32);
    let mut img = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let on_diagonal = (x as i32 - y as i32).abs() < 3;
            img.data[i..i + 4].copy_from_slice(&[
                (x * 4) as u8,
                (y * 5) as u8,
                if on_diagonal { 255 } else { 90 },
                255,
            ]);
        }
    }
    img.encode_png().expect("png")
}

/// Build the frame for a `width` × `height` logical window. The styles
/// are registered in `text`, so the frame must be rasterized with the
/// same engine.
pub fn frame_2000(text: &mut TextEngine, width: f32, height: f32) -> Frame {
    let body = text
        .style(font::SANS, 14.0, weight::NORMAL, 0x2221_1FFF)
        .expect("style");
    let bold = text
        .style(font::SANS, 16.0, weight::BOLD, 0x1A1A_2EFF)
        .expect("style");
    let mono = text
        .style(font::MONO, 12.0, weight::NORMAL, 0x3344_55FF)
        .expect("style");
    let styles = [body, body, bold, mono];
    const LINES: [&str; 4] = [
        "Vy ritar text i värden, aldrig i modulen",
        "Sidor är WebAssembly-moduler som ritar",
        "Lägesrapport för november: allt grönt",
        "0x2F5BD6FF  rect 12 x 7  clip 3  text 41",
    ];
    let mut dl = DisplayList::new();
    dl.begin().expect("begin");
    let mut n = 0usize;
    // The page background.
    dl.rect(0.0, 0.0, width, height, 0.0, 0xF5F5_F0FF)
        .expect("rect");
    n += 1;
    // A grid of 40 cards, 8 × 5, each clipped (every fourth rounded) and
    // holding a rounded background, an image in every other card, and
    // rows of text and boxes; positions cycle over the window.
    let cols = 8usize;
    let rows = 5usize;
    let cw = width / cols as f32;
    let ch = height / rows as f32;
    let card = |i: usize| -> (f32, f32) {
        (
            (i % cols) as f32 * cw + 6.0,
            ((i / cols) % rows) as f32 * ch + 6.0,
        )
    };
    let cards = cols * rows;
    let push = |dl: &mut DisplayList, n: &mut usize, i: usize| {
        let (x, y) = card(i);
        let radius = if i.is_multiple_of(4) { 8.0 } else { 0.0 };
        dl.clip_push(x, y, cw - 12.0, ch - 12.0, radius)
            .expect("clip");
        *n += 1;
    };
    // 40 cards: 10 rounded clips, 30 plain clips, 40 pops. With the clips
    // below: 120 plain and 40 rounded pushes, 160 pops.
    for i in 0..cards {
        push(&mut dl, &mut n, i);
        let (x, y) = card(i);
        // Card background: rounded, opaque.
        dl.rect(x, y, cw - 12.0, ch - 12.0, 6.0, 0xFFFF_FFFF)
            .expect("rect");
        n += 1;
        if i.is_multiple_of(2) {
            dl.image(x + 8.0, y + 8.0, 96.0, 72.0, 1, Fit::Cover)
                .expect("image");
            n += 1;
        }
        dl.clip_pop().expect("pop");
        n += 1;
    }
    // 90 more plain clips (list rows, cells), each with one pop.
    for i in 0..90usize {
        let (x, y) = card(i % cards);
        dl.clip_push(x, y + (i % 7) as f32 * 18.0, cw - 12.0, 18.0, 0.0)
            .expect("clip");
        dl.clip_pop().expect("pop");
        n += 2;
    }
    // 30 more rounded clips with pops.
    for i in 0..30usize {
        let (x, y) = card((i * 7) % cards);
        dl.clip_push(x + 4.0, y + 4.0, 60.0, 40.0, 5.0)
            .expect("clip");
        dl.clip_pop().expect("pop");
        n += 2;
    }
    // 20 more images (every other card had one above).
    for i in 0..20usize {
        let (x, y) = card((i * 2 + 1) % cards);
        dl.image(x + 8.0, y + 8.0, 96.0, 72.0, 1, Fit::Contain)
            .expect("image");
        n += 1;
    }
    // 399 opaque rectangles: rows, separators, badges.
    for i in 0..399usize {
        let (x, y) = card(i % cards);
        let (w, h) = match i % 3 {
            0 => (cw - 24.0, 1.0),
            1 => (90.0, 22.0),
            _ => (12.0, 12.0),
        };
        dl.rect(
            x + 6.0 + (i % 5) as f32 * 3.0,
            y + 90.0 + (i % 9) as f32 * 8.0,
            w,
            h,
            0.0,
            0x6688_AAFF - (i % 7) as u32 * 0x0010_0000,
        )
        .expect("rect");
        n += 1;
    }
    // 340 opaque rounded rectangles: buttons, chips, cards.
    for i in 0..340usize {
        let (x, y) = card(i % cards);
        dl.rect(
            x + 10.0 + (i % 4) as f32 * 30.0,
            y + 40.0 + (i % 6) as f32 * 14.0,
            80.0,
            24.0,
            4.0,
            0x2F5B_D6FF - (i % 5) as u32 * 0x0008_0000,
        )
        .expect("rect");
        n += 1;
    }
    // 50 translucent rounded rectangles and 10 translucent plain ones:
    // hover, selection, overlays.
    for i in 0..60usize {
        let (x, y) = card((i * 3) % cards);
        let radius = if i < 50 { 6.0 } else { 0.0 };
        dl.rect(
            x + 4.0,
            y + 30.0 + (i % 5) as f32 * 20.0,
            cw - 20.0,
            28.0,
            radius,
            0x3366_9960,
        )
        .expect("rect");
        n += 1;
    }
    // 10 stroked paths: chart lines of four segments.
    for i in 0..10usize {
        let (x, y) = card((i * 5) % cards);
        let cmds = [
            PathCmd::MoveTo(x + 8.0, y + 100.0),
            PathCmd::LineTo(x + 40.0, y + 70.0 + i as f32),
            PathCmd::LineTo(x + 72.0, y + 90.0),
            PathCmd::LineTo(x + 104.0, y + 60.0),
            PathCmd::LineTo(x + 136.0, y + 80.0),
        ];
        dl.path(0xD64B_2FFF, 2.0, &cmds).expect("path");
        n += 1;
    }
    // The rest is text: runs of about 30 bytes, wrapped at the card width.
    let mut i = 0usize;
    while n < OPS {
        let (x, y) = card(i % cards);
        let style = styles[i % styles.len()];
        let line = LINES[i % LINES.len()];
        dl.text(
            x + 8.0,
            y + 8.0 + (i % 8) as f32 * 17.0,
            cw - 20.0,
            style,
            line.as_bytes(),
        )
        .expect("text");
        n += 1;
        i += 1;
    }
    dl.end().expect("end");
    debug_assert_eq!(dl.op_count() as usize, OPS);
    let mut images = ImageCache::from_bytes(vec![picture()]);
    images.decode(1).expect("picture decodes");
    Frame {
        list: dl.bytes().to_vec(),
        images,
    }
}

/// The text of every run in [`grid`]: 40 characters, 41 bytes.
pub const GRID_LINE: &str = "Vy ritar text i värden, aldrig i modulen";

/// The pathological lists of M3's bench (review 0004 measured them):
/// `rects` rounded (radius 3), translucent 30 × 15 rectangles in a grid
/// of 40 columns, then `runs` text runs of [`GRID_LINE`] wrapped at 300 px
/// in four columns, all in one 14 px sans style registered in `text`.
/// `grid(t, 2000, 0)`, `grid(t, 0, 200)` and `grid(t, 1800, 200)` are the
/// three rows `raster.rects-2000`, `raster.text-200` and
/// `raster.mix-1800-200`.
pub fn grid(text: &mut TextEngine, rects: u32, runs: u32) -> Vec<u8> {
    let style = text
        .style(font::SANS, 14.0, weight::NORMAL, 0x2221_1FFF)
        .expect("style");
    let mut dl = DisplayList::new();
    dl.begin().expect("begin");
    for i in 0..rects {
        let (x, y) = ((i % 40) as f32 * 32.0, (i / 40) as f32 * 17.0);
        dl.rect(x, y, 30.0, 15.0, 3.0, 0x3366_9980 + (i % 5) * 0x0010_0000)
            .expect("rect");
    }
    for i in 0..runs {
        let (x, y) = ((i % 4) as f32 * 320.0, (i / 4) as f32 * 16.0);
        dl.text(x, y, 300.0, style, GRID_LINE.as_bytes())
            .expect("text");
    }
    dl.end().expect("end");
    dl.bytes().to_vec()
}

/// 2 000 small cards, each a rounded clip pushed, an opaque rectangle
/// overflowing it drawn, and the clip popped: 6 000 ops, the second half
/// of the vy-render test `spec_5_3_3_bench_32_rounded_clips_2000_rects`.
/// Every push needs its own mask, so the row shows what a mask costs.
pub fn rounded_cards() -> Vec<u8> {
    let mut dl = DisplayList::new();
    dl.begin().expect("begin");
    for i in 0..2_000 {
        let (x, y) = ((i % 40) as f32 * 32.0, (i / 40) as f32 * 16.0);
        dl.clip_push(x, y, 30.0, 14.0, 4.0).expect("clip");
        dl.rect(x - 5.0, y - 5.0, 40.0, 24.0, 0.0, 0x3366_99FF)
            .expect("rect");
        dl.clip_pop().expect("pop");
    }
    dl.end().expect("end");
    dl.bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vy_dl::Op;

    /// The frame is 2 000 ops in the tallied proportions, and it renders.
    #[test]
    fn frame_has_the_tallied_mix() {
        let mut text = TextEngine::new();
        let frame = frame_2000(&mut text, 1280.0, 800.0);
        let ops = vy_dl::decode(&frame.list).unwrap();
        assert_eq!(ops.len(), OPS);
        let count = |f: &dyn Fn(&Op) -> bool| ops.iter().filter(|o| f(o)).count();
        let texts = count(&|o| matches!(o, Op::Text { .. }));
        let opaque_plain = count(
            &|o| matches!(o, Op::Rect { radius, color, .. } if *radius == 0.0 && color & 0xFF == 0xFF),
        );
        let opaque_round = count(
            &|o| matches!(o, Op::Rect { radius, color, .. } if *radius > 0.0 && color & 0xFF == 0xFF),
        );
        let clips = count(&|o| matches!(o, Op::ClipPush { .. }));
        let pops = count(&|o| matches!(o, Op::ClipPop));
        let images = count(&|o| matches!(o, Op::Image { .. }));
        assert!((780..=830).contains(&texts), "{texts} text runs");
        assert_eq!(opaque_plain, 400);
        assert_eq!(opaque_round, 380);
        assert_eq!(clips, 160);
        assert_eq!(pops, 160);
        assert_eq!(images, 40);
        let r = crate::render(&frame.list, 1280, 800, 1.0, &mut text, &frame.images).unwrap();
        assert!(!r.report.truncated, "{:?}", r.report);
    }
}
